//! The chat-completions wire format: the request body the client sends and the
//! reply it reads. Pure functions over bytes, so every shape is testable without
//! a socket.
//!
//! Ported from forge-harness `completion_body.rs` (tolerant reply framing) and
//! the body and reply handling of `openai_compatible.rs`.

use lgwks_std::json::{self, Value, json};

use super::{Completion, Message, ModelError, Usage};

/// Serialize a non-streaming request. Every request names the pinned model and
/// the temperature; `stream` is never sent. A `Value` always renders, so this
/// cannot fail; the temperature is finite by construction of the config.
pub(super) fn request_body(model: &str, temperature: f64, messages: &[Message]) -> Vec<u8> {
    let turns: Vec<Value> = messages
        .iter()
        .map(|turn| json!({ "role": turn.role.as_str(), "content": turn.content }))
        .collect();
    let body = json!({
        "model": model,
        "temperature": temperature,
        "messages": turns,
    });
    body.to_string().into_bytes()
}

/// Read one successful reply body into its completion. The body may be a bare
/// JSON document, a document followed by `data: [DONE]`, or an SSE chunk stream;
/// all three reduce to the same document (see [`leading_document`]).
pub(super) fn parse_completion(body: &[u8]) -> Result<Completion, ModelError> {
    let text = std::str::from_utf8(body).map_err(ModelError::NotUtf8)?;
    let document = leading_document(text)?;
    let message = &document["choices"][0]["message"];
    let content = message["content"]
        .as_str()
        .ok_or(ModelError::Malformed("the reply has no message content"))?;
    Ok(Completion {
        content: content.to_owned(),
        usage: usage_of(&document),
    })
}

/// The OpenAI `usage` block: both token counts as whole numbers, or `None`. A
/// gateway that reports one count, a negative or a fraction reported nothing
/// a cost can be built on, so nothing is invented (larry#87). The cache counts
/// ride along when a gateway reports them (larry#230): cached tokens from
/// `prompt_tokens_details.cached_tokens` (OpenAI), `prompt_cache_hit_tokens`
/// (DeepSeek) or `cache_read_input_tokens` (Anthropic), written tokens from
/// `prompt_tokens_details.cache_write_tokens` or `cache_creation_input_tokens`.
/// Counts that together exceed the prompt are an impossible report and are
/// dropped, so a cost is never built on them.
fn usage_of(document: &Value) -> Option<Usage> {
    let usage = &document["usage"];
    let prompt_tokens = usage["prompt_tokens"].as_u64()?;
    let completion_tokens = usage["completion_tokens"].as_u64()?;
    let details = &usage["prompt_tokens_details"];
    let cached = first_count(&[
        &details["cached_tokens"],
        &usage["prompt_cache_hit_tokens"],
        &usage["cache_read_input_tokens"],
    ]);
    let written = first_count(&[
        &details["cache_write_tokens"],
        &usage["cache_creation_input_tokens"],
    ]);
    let together = cached
        .into_iter()
        .chain(written)
        .try_fold(0_u64, u64::checked_add);
    let (cached_tokens, cache_write_tokens) = match together {
        Some(sum) if sum <= prompt_tokens => (cached, written),
        Some(_) | None => (None, None),
    };
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        cache_write_tokens,
    })
}

/// The first of `candidates` that is a whole non-negative number.
fn first_count(candidates: &[&Value]) -> Option<u64> {
    candidates.iter().find_map(|candidate| candidate.as_u64())
}

/// The one JSON document a reply carries.
///
/// Gateways that front several upstreams disagree on non-streaming framing: a
/// bare document, that document with `data: [DONE]` appended, or a full SSE
/// stream of `chat.completion.chunk` events. The first parses directly. The
/// others reassemble from their `data:` payloads, so the framing never reaches
/// the caller. A stream reports its token counts as the `usage` object of its
/// last chunk that carries one; that object is kept (larry#230), a `null` is not.
fn leading_document(text: &str) -> Result<Value, ModelError> {
    if let Ok(document) = json::from_str::<Value>(without_done_marker(text)) {
        return Ok(document);
    }
    let mut assembled = String::new();
    let mut chunks = 0_usize;
    let mut usage = Value::Null;
    for line in text.lines() {
        let Some(payload) = line.trim_start().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(chunk) = json::from_str::<Value>(payload) else {
            continue;
        };
        if chunk["usage"].as_object().is_some() {
            usage = chunk["usage"].clone();
        }
        if let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str() {
            assembled.push_str(delta);
            chunks = chunks.saturating_add(1);
        }
    }
    if chunks == 0 {
        return Err(ModelError::Malformed(
            "the reply is not a completion document",
        ));
    }
    Ok(json!({ "choices": [{ "message": { "content": assembled } }], "usage": usage }))
}

/// The text without surrounding whitespace and without one trailing
/// `data: [DONE]` marker. OpenRouter through 9router pads a non-streaming
/// reply with keepalive whitespace and appends the marker to the document on
/// the same line (larry#128).
fn without_done_marker(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(head) = trimmed.strip_suffix("[DONE]") else {
        return trimmed;
    };
    match head.trim_end().strip_suffix("data:") {
        Some(document) => document.trim_end(),
        None => trimmed,
    }
}

#[cfg(test)]
#[path = "sim_wire.rs"]
mod sim;

/// Most characters of a gateway's own refusal message an error carries.
const MAX_DETAIL_CHARS: usize = 512;

/// Most bytes of a refusal body the client reads (larry#236): a gateway's error
/// document is a few hundred bytes, and nothing past this is its reason.
const MAX_ERROR_BODY: usize = 65_536;

/// Most characters of a gateway's own `error.code` a refusal carries.
const MAX_CODE_CHARS: usize = 64;

/// The gateway's reason for a refusal: its `error.code` and `error.message`
/// when the body is an OpenAI-shaped error, else the body text. Bounded, and
/// with every control character replaced, so an upstream cannot write into an
/// operator's terminal. The code leads, so a translated message still says what
/// the refusal is (larry#236).
fn refusal_detail(body: &[u8]) -> String {
    let head = match body.get(..MAX_ERROR_BODY) {
        Some(head) => head,
        None => body,
    };
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim();
    let document = json::from_str::<Value>(trimmed);
    let code = document
        .as_ref()
        .ok()
        .and_then(|document| document["error"]["code"].as_str())
        .map(|code| bounded(code, MAX_CODE_CHARS));
    let message = document
        .as_ref()
        .ok()
        .and_then(|document| document["error"]["message"].as_str())
        .map(str::to_owned);
    let reason = match (code, message) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code,
        (None, Some(message)) => message,
        (None, None) => trimmed.to_owned(),
    };
    bounded(&reason, MAX_DETAIL_CHARS)
}

/// `text` with every control character replaced by a space, at most `most`
/// characters long.
fn bounded(text: &str, most: usize) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(most)
        .collect()
}

/// Map a non-success status and its body to an error. 429 carries the
/// gateway's `Retry-After` hint when present. A 5xx is `Server`, every other
/// status `Client`; both carry the gateway's own reason.
pub(super) fn status_error(status: u16, retry_after: Option<u64>, body: &[u8]) -> ModelError {
    match status {
        429 => ModelError::RateLimited { retry_after },
        500..=599 => ModelError::Server {
            status,
            detail: refusal_detail(body),
        },
        _ => ModelError::Client {
            status,
            detail: refusal_detail(body),
        },
    }
}
