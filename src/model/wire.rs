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
    if let Ok(document) = json::from_str::<Value>(text) {
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

/// Map a non-success status to its error. 429 carries the gateway's
/// `Retry-After` hint when present. A 5xx is `Server`, every other status `Client`.
pub(super) fn status_error(status: u16, retry_after: Option<u64>) -> ModelError {
    match status {
        429 => ModelError::RateLimited { retry_after },
        500..=599 => ModelError::Server(status),
        _ => ModelError::Client(status),
    }
}
