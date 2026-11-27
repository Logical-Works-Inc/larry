//! The chat-completions wire format: the request body the client sends and the
//! reply it reads. Pure functions over bytes, so every shape is testable without
//! a socket.
//!
//! Ported from forge-harness `completion_body.rs` (tolerant reply framing) and
//! the body and reply handling of `openai_compatible.rs`.

use lgwks_std::json::{self, Map, Value, json};

use super::route::Kind;
use super::{Completion, Message, ModelError, Usage};

/// The `max_tokens` an Anthropic Messages request carries: the API refuses a
/// request without one, and the per-route context bound stays `request_bytes`,
/// which the client enforces before any byte is sent (larry#205).
const ADAPTER_MAX_TOKENS: u32 = 8192;

/// Serialize a non-streaming request for `kind`. Every shape names the pinned
/// model; `stream` is never sent. A `Value` always renders, so this cannot
/// fail. The temperature rides along only when the route pins one: a model
/// that rejects the field answers with a 4xx, which is terminal (larry#205).
pub(super) fn request_body(
    kind: Kind,
    model: &str,
    temperature: Option<f64>,
    messages: &[Message],
) -> Vec<u8> {
    match kind {
        Kind::OpenAiChat => openai_body(model, temperature, messages),
        Kind::AnthropicMessages => anthropic_body(model, temperature, messages),
        Kind::OpenAiResponses => responses_body(model, temperature, messages),
    }
}

/// The members every request body carries: the pinned model, and the
/// temperature only when the route pins it.
fn body_base(model: &str, temperature: Option<f64>) -> Map {
    let mut object = Map::new();
    object.insert(String::from("model"), json!(model));
    if let Some(degrees) = temperature {
        object.insert(String::from("temperature"), json!(degrees));
    }
    object
}

/// The OpenAI chat-completions shape: model, messages and the temperature when pinned.
fn openai_body(model: &str, temperature: Option<f64>, messages: &[Message]) -> Vec<u8> {
    let turns: Vec<Value> = messages
        .iter()
        .map(|turn| json!({ "role": turn.role.as_str(), "content": turn.content }))
        .collect();
    let mut object = body_base(model, temperature);
    object.insert(String::from("messages"), Value::Array(turns));
    Value::Object(object).to_string().into_bytes()
}

/// The Anthropic Messages shape: system turns ride as `system`, the rest as
/// `messages`, and `max_tokens` is always present because the API requires it.
fn anthropic_body(model: &str, temperature: Option<f64>, messages: &[Message]) -> Vec<u8> {
    let mut system: Vec<&str> = Vec::new();
    let mut turns: Vec<Value> = Vec::new();
    for turn in messages {
        if turn.role == super::Role::System {
            system.push(turn.content.as_str());
        } else {
            turns.push(json!({ "role": turn.role.as_str(), "content": turn.content }));
        }
    }
    let mut object = body_base(model, temperature);
    object.insert(String::from("max_tokens"), json!(ADAPTER_MAX_TOKENS));
    object.insert(String::from("messages"), Value::Array(turns));
    if !system.is_empty() {
        object.insert(String::from("system"), json!(system.join("\n\n")));
    }
    Value::Object(object).to_string().into_bytes()
}

/// The OpenAI Responses shape: the turns ride as `input`, with the same roles
/// the chat shape uses.
fn responses_body(model: &str, temperature: Option<f64>, messages: &[Message]) -> Vec<u8> {
    let turns: Vec<Value> = messages
        .iter()
        .map(|turn| json!({ "role": turn.role.as_str(), "content": turn.content }))
        .collect();
    let mut object = body_base(model, temperature);
    object.insert(String::from("input"), Value::Array(turns));
    Value::Object(object).to_string().into_bytes()
}

/// `finish_reason` values that say the model ended its reply itself, compared ignoring ASCII
/// case: OpenAI and vLLM `stop`, Anthropic's `end_turn` and `stop_sequence` as a gateway passes
/// them through, and Gemini's `STOP`. Every other reason (`length`, `content_filter`,
/// `max_tokens`, `MAX_TOKENS`, `SAFETY`, `tool_calls`, one no gateway documents) is a reply the
/// model did not finish, so the list is closed and anything past it is refused (larry#396).
const FINISHED: [&str; 3] = ["stop", "end_turn", "stop_sequence"];

/// Read one successful reply body of `kind` into its completion. The
/// OpenAI chat shape may be a bare JSON document, a document followed by
/// `data: [DONE]`, or an SSE chunk stream; all three reduce to the same
/// document (see [`leading_document`]). The Anthropic and Responses shapes are
/// bare documents only. A reply the model did not finish is refused before its
/// content is read, so a filtered reply with no content is named by its reason
/// (larry#396).
pub(super) fn parse_completion(kind: Kind, body: &[u8]) -> Result<Completion, ModelError> {
    let text = std::str::from_utf8(body).map_err(ModelError::NotUtf8)?;
    match kind {
        Kind::OpenAiChat => parse_chat(text),
        Kind::AnthropicMessages => parse_anthropic(text),
        Kind::OpenAiResponses => parse_responses(text),
    }
}

/// One OpenAI chat-completions reply.
fn parse_chat(text: &str) -> Result<Completion, ModelError> {
    let document = leading_document(text)?;
    finished(&document)?;
    let content = document
        .get("choices")
        .and_then(|choices| choices.get(0_usize))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or(ModelError::Malformed("the reply has no message content"))?;
    Ok(Completion {
        content: content.to_owned(),
        usage: usage_of(&document),
    })
}

/// One Anthropic Messages reply: the `text` blocks joined, refused unless
/// `stop_reason` says the model ended its turn, with the input/output counts.
fn parse_anthropic(text: &str) -> Result<Completion, ModelError> {
    let document = json::from_str::<Value>(text.trim()).map_err(|cause| ModelError::NotJson {
        column: cause.column(),
    })?;
    let reason = document
        .get("stop_reason")
        .and_then(Value::as_str)
        .ok_or(ModelError::Malformed("the reply has no stop reason"))?;
    if !matches!(reason, "end_turn" | "stop_sequence") {
        return Err(ModelError::Unfinished {
            reason: bounded(reason, MAX_CODE_CHARS),
        });
    }
    let blocks = document
        .get("content")
        .and_then(Value::as_array)
        .ok_or(ModelError::Malformed("the reply has no message content"))?;
    let mut content = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            content.push_str(
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(ModelError::Malformed("the reply has no message content"))?,
            );
        }
    }
    Ok(Completion {
        content,
        usage: anthropic_usage(&document),
    })
}

/// One OpenAI Responses reply: the `output_text` of its message outputs
/// joined, refused unless `status` is `completed`, with the input/output counts.
fn parse_responses(text: &str) -> Result<Completion, ModelError> {
    let document = json::from_str::<Value>(text.trim()).map_err(|cause| ModelError::NotJson {
        column: cause.column(),
    })?;
    let status = document
        .get("status")
        .and_then(Value::as_str)
        .ok_or(ModelError::Malformed("the reply has no status"))?;
    if status != "completed" {
        return Err(ModelError::Unfinished {
            reason: bounded(status, MAX_CODE_CHARS),
        });
    }
    let outputs = document
        .get("output")
        .and_then(Value::as_array)
        .ok_or(ModelError::Malformed("the reply has no message content"))?;
    let mut content = String::new();
    for output in outputs {
        if output.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let parts = output
            .get("content")
            .and_then(Value::as_array)
            .ok_or(ModelError::Malformed("the reply has no message content"))?;
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("output_text") {
                content.push_str(
                    part.get("text")
                        .and_then(Value::as_str)
                        .ok_or(ModelError::Malformed("the reply has no message content"))?,
                );
            }
        }
    }
    Ok(Completion {
        content,
        usage: responses_usage(&document),
    })
}

/// The file text a structured reply carries: the `content` string of the one
/// JSON object the kind's brief asks for. A reply that is not that object is
/// refused, never written (larry#205).
pub(crate) fn structured_content(text: &str) -> Result<String, ModelError> {
    let document = json::from_str::<Value>(text.trim()).map_err(|cause| ModelError::NotJson {
        column: cause.column(),
    })?;
    document
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ModelError::Malformed(
            "the reply is not a JSON object with a content string",
        ))
}

/// Refuses a reply whose first choice says the model did not finish it. A missing or `null`
/// reason reads as finished, because many gateways send none; a reason that is not text is
/// malformed (larry#396).
fn finished(document: &Value) -> Result<(), ModelError> {
    let reason = match document
        .get("choices")
        .and_then(|choices| choices.get(0_usize))
        .and_then(|choice| choice.get("finish_reason"))
    {
        None | Some(&Value::Null) => return Ok(()),
        Some(reason) => reason.as_str().ok_or(ModelError::Malformed(
            "the reply's finish_reason is not text",
        ))?,
    };
    if FINISHED
        .iter()
        .any(|done| reason.eq_ignore_ascii_case(done))
    {
        return Ok(());
    }
    Err(ModelError::Unfinished {
        reason: bounded(reason, MAX_CODE_CHARS),
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
    let details = &usage["prompt_tokens_details"];
    counts(
        usage["prompt_tokens"].as_u64(),
        usage["completion_tokens"].as_u64(),
        first_count(&[
            &details["cached_tokens"],
            &usage["prompt_cache_hit_tokens"],
            &usage["cache_read_input_tokens"],
        ]),
        first_count(&[
            &details["cache_write_tokens"],
            &usage["cache_creation_input_tokens"],
        ]),
    )
}

/// The Anthropic `usage` block: `input_tokens` and `output_tokens`, with the
/// prompt-caching counts when reported.
fn anthropic_usage(document: &Value) -> Option<Usage> {
    let usage = &document["usage"];
    counts(
        usage["input_tokens"].as_u64(),
        usage["output_tokens"].as_u64(),
        first_count(&[&usage["cache_read_input_tokens"]]),
        first_count(&[&usage["cache_creation_input_tokens"]]),
    )
}

/// The Responses `usage` block: `input_tokens` and `output_tokens`, with the
/// cached input tokens when reported.
fn responses_usage(document: &Value) -> Option<Usage> {
    let usage = &document["usage"];
    let details = &usage["input_tokens_details"];
    counts(
        usage["input_tokens"].as_u64(),
        usage["output_tokens"].as_u64(),
        first_count(&[&details["cached_tokens"]]),
        None,
    )
}

/// The `Usage` for these counts, or `None` when a count is missing. A gateway
/// that reports one count, a negative or a fraction reported nothing a cost can
/// be built on, so nothing is invented (larry#87). Counts that together exceed
/// the prompt are an impossible report and are dropped, so a cost is never
/// built on them.
fn counts(
    prompt: Option<u64>,
    completion: Option<u64>,
    cached: Option<u64>,
    written: Option<u64>,
) -> Option<Usage> {
    let (Some(prompt_tokens), Some(completion_tokens)) = (prompt, completion) else {
        return None;
    };
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
/// Its finish reason is the last one a chunk names, and a `data:` payload that is
/// not JSON refuses the reply, because the content it carried is lost (larry#396).
fn leading_document(text: &str) -> Result<Value, ModelError> {
    if let Ok(document) = json::from_str::<Value>(without_done_marker(text)) {
        return Ok(document);
    }
    let mut assembled = String::new();
    let mut chunks = 0_usize;
    let mut usage = Value::Null;
    let mut finish = Value::Null;
    for (number, line) in text.lines().enumerate() {
        let Some(payload) = line.trim_start().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let chunk = json::from_str::<Value>(payload).map_err(|cause| ModelError::StreamChunk {
            line: number.saturating_add(1),
            column: cause.column(),
        })?;
        if let Some(reported) = chunk.get("usage")
            && reported.as_object().is_some()
        {
            usage = reported.clone();
        }
        let choice = chunk
            .get("choices")
            .and_then(|choices| choices.get(0_usize));
        if let Some(reason) = choice.and_then(|choice| choice.get("finish_reason"))
            && !reason.is_null()
        {
            finish = reason.clone();
        }
        if let Some(delta) = choice
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
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
    Ok(json!({
        "choices": [{ "message": { "content": assembled }, "finish_reason": finish }],
        "usage": usage,
    }))
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

/// Seeded simulations of the per-kind adapters (larry#205).
#[cfg(test)]
#[path = "sim_adapter.rs"]
mod adapter;

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
        .and_then(|document| document.get("error"))
        .and_then(|refusal| refusal.get("code"))
        .and_then(Value::as_str)
        .map(|code| bounded(code, MAX_CODE_CHARS));
    let message = document
        .as_ref()
        .ok()
        .and_then(|document| document.get("error"))
        .and_then(|refusal| refusal.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let reason = match (code, message) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code,
        (None, Some(message)) => message,
        (None, None) => trimmed.to_owned(),
    };
    bounded(&reason, MAX_DETAIL_CHARS)
}

/// Whether `character` reorders text around it: the Unicode bidi controls. A refusal that
/// carries one can read differently from the text it names (CVE-2021-42574).
const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// `text` with every control or bidi character replaced by a space, at most `most`
/// characters long.
fn bounded(text: &str, most: usize) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || is_bidi_control(character) {
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
