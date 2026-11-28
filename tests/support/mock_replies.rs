//! The kind-shaped reply composer of the seeded mock endpoint (larry#205): one well-formed
//! reply per provider path and the fault injection into the shape that path speaks. Test
//! support, never linked into a shipped binary (INVARIANTS: test doubles never ship).

use lgwks_std::json::{self, Value, json};
use lgwks_std::seeded::Seeded;

use super::{Failure, VOCAB};
use crate::http_wire::Request;

/// A well-formed reply for `path` and `model`: the scripted content when there is
/// one, otherwise words drawn from the seed.
pub(crate) fn completion(
    path: &str,
    model: &str,
    request: &Request,
    rng: &mut Seeded,
    scripted: Option<&String>,
) -> Result<Vec<u8>, Failure> {
    let count = usize::try_from(rng.below(10)?.saturating_add(3))?;
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        words.push(*VOCAB.get(rng.index(VOCAB.len())?).ok_or(Failure::Table)?);
    }
    let (content, produced) = match scripted {
        Some(text) => (text.clone(), text.len().div_ceil(4)),
        None => (words.join(" "), count),
    };
    let prompt = u64::try_from(request.body.len().div_ceil(4))?;
    let produced = u64::try_from(produced)?;
    // The structured kinds answer the file inside one JSON object, so the
    // content the author extracts is that object's `content` string.
    let text = match path {
        "/v1/messages" | "/v1/responses" => json!({"content": content}).to_string(),
        _ => content,
    };
    let reply = match path {
        "/v1/messages" => json!({
            "id": format!("msg-{:016x}", rng.next_u64()),
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": prompt,
                "output_tokens": produced,
            }
        }),
        "/v1/responses" => json!({
            "id": format!("resp-{:016x}", rng.next_u64()),
            "object": "response",
            "status": "completed",
            "model": model,
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}]
            }],
            "usage": {
                "input_tokens": prompt,
                "output_tokens": produced,
                "total_tokens": prompt.saturating_add(produced)
            }
        }),
        _ => json!({
            "id": format!("chatcmpl-{:016x}", rng.next_u64()),
            "object": "chat.completion",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": text },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": produced,
                "total_tokens": prompt.saturating_add(produced)
            }
        }),
    };
    Ok(json::to_vec(&reply)?)
}

/// `good` with its top-level `usage` block removed, the way proxies that never
/// meter answer (larry#204): every provider shape carries its counts under one
/// `usage` object, so the strip is the same on all three paths. A `null`
/// usage would parse the same way; the block is removed outright so the reply
/// matches the unmetered shape byte for byte.
pub(crate) fn without_usage(good: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut reply = json::from_slice::<Value>(good)?;
    if let Value::Object(ref mut map) = reply {
        map.remove("usage");
    }
    Ok(json::to_vec(&reply)?)
}

/// `good` (a chat reply) whose `usage` also says `cached` prompt tokens were read from the
/// cache and `written` written to it (larry#230), the OpenAI `prompt_tokens_details` shape.
pub(crate) fn with_cache_usage(good: &[u8], cached: u64, written: u64) -> Result<Vec<u8>, Failure> {
    let mut reply = json::from_slice::<Value>(good)?;
    if let Some(&mut Value::Object(ref mut usage)) = reply.get_mut("usage") {
        usage.insert(
            String::from("prompt_tokens_details"),
            json!({"cached_tokens": cached, "cache_write_tokens": written}),
        );
    }
    Ok(json::to_vec(&reply)?)
}

/// `good` (a reply for `path`) with the finish marker or the content set to
/// `value`: the chat shape's first choice, the Messages shape's `stop_reason`
/// or first text block, the Responses shape's `status` or first output text.
pub(crate) fn with_choice(
    path: &str,
    good: &[u8],
    member: &str,
    value: &str,
) -> Result<Vec<u8>, Failure> {
    let mut reply = json::from_slice::<Value>(good)?;
    match path {
        "/v1/messages" if member == "finish_reason" => {
            *reply.get_mut("stop_reason").ok_or(Failure::Table)? = json!(value);
        }
        "/v1/messages" => {
            *reply
                .get_mut("content")
                .and_then(|content| content.get_mut(0_usize))
                .and_then(|block| block.get_mut("text"))
                .ok_or(Failure::Table)? = json!(value);
        }
        "/v1/responses" if member == "finish_reason" => {
            *reply.get_mut("status").ok_or(Failure::Table)? = json!(value);
        }
        "/v1/responses" => {
            let outputs = reply
                .get_mut("output")
                .and_then(Value::as_array_mut)
                .ok_or(Failure::Table)?;
            let part = outputs
                .iter_mut()
                .filter(|output| output.get("type").and_then(Value::as_str) == Some("message"))
                .filter_map(|output| output.get_mut("content"))
                .filter_map(Value::as_array_mut)
                .flat_map(|parts| parts.iter_mut())
                .find(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
                .and_then(|part| part.get_mut("text"))
                .ok_or(Failure::Table)?;
            *part = json!(value);
        }
        _ => {
            let choice = reply
                .get_mut("choices")
                .and_then(|choices| choices.get_mut(0_usize))
                .ok_or(Failure::Table)?;
            let target = if member.is_empty() {
                choice
                    .get_mut("message")
                    .and_then(|message| message.get_mut("content"))
            } else {
                choice.get_mut(member)
            };
            *target.ok_or(Failure::Table)? = json!(value);
        }
    }
    Ok(json::to_vec(&reply)?)
}
