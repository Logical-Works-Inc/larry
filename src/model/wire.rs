//! The chat-completions wire format: the request body the client sends and the
//! reply it reads. Pure functions over bytes, so every shape is testable without
//! a socket.
//!
//! Ported from forge-harness `completion_body.rs` (tolerant reply framing) and
//! the body and reply handling of `openai_compatible.rs`.

use lgwks_std::json::{self, Value, json};

use super::{Completion, Message, ModelError};

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
    })
}

/// The one JSON document a reply carries.
///
/// Gateways that front several upstreams disagree on non-streaming framing: a
/// bare document, that document with `data: [DONE]` appended, or a full SSE
/// stream of `chat.completion.chunk` events. The first parses directly. The
/// others reassemble from their `data:` payloads, so the framing never reaches
/// the caller.
fn leading_document(text: &str) -> Result<Value, ModelError> {
    if let Ok(document) = json::from_str::<Value>(without_done_marker(text)) {
        return Ok(document);
    }
    let mut assembled = String::new();
    let mut chunks = 0_usize;
    for line in text.lines() {
        let Some(payload) = line.trim_start().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(chunk) = json::from_str::<Value>(payload)
            && let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str()
        {
            assembled.push_str(delta);
            chunks = chunks.saturating_add(1);
        }
    }
    if chunks == 0 {
        return Err(ModelError::Malformed(
            "the reply is not a completion document",
        ));
    }
    Ok(json!({ "choices": [{ "message": { "content": assembled } }] }))
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

/// Most characters of a gateway's own refusal message an error carries.
const MAX_DETAIL_CHARS: usize = 512;

/// The gateway's reason for a refusal: `error.message` when the body is an
/// OpenAI-shaped error, else the body text. Bounded, and with every control
/// character replaced, so an upstream cannot write into an operator's terminal.
fn refusal_detail(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    let message = json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|document| document["error"]["message"].as_str().map(str::to_owned));
    let reason = match message {
        Some(message) => message,
        None => trimmed.to_owned(),
    };
    reason
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_DETAIL_CHARS)
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
