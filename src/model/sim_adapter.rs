//! Seeded simulations of the provider adapters (larry#205): every kind sends its own
//! body members and reads its own reply into the one `Usage` shape. One seed drives every
//! kind, temperature and count, so the same seeds replay the same verdicts.

use std::error::Error;

use lgwks_std::json::{Value, json};
use lgwks_std::seeded::Seeded;

use super::{ADAPTER_MAX_TOKENS, parse_completion, request_body, structured_content};
use crate::model::{Kind, Message, ModelError, Role, Usage};

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds the simulations run.
const SEEDS: u64 = 256;

/// larry#205: every kind's body carries the model and its own members: chat
/// sends `messages`, responses sends `input`, messages sends `messages` plus
/// `system` and `max_tokens`; `temperature` rides only when the route pins it,
/// and a system turn never lands in a messages list. Catches an adapter that
/// sends one shape for every kind, or a temperature a model rejects.
#[test]
fn sim_each_kind_sends_its_own_members() -> TestResult {
    let turns = [
        Message::new(Role::System, "be terse"),
        Message::new(Role::User, "write it"),
        Message::new(Role::Assistant, "draft"),
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let temperature = match rng.below(2)? {
            0 => None,
            _ => Some(f64::from(u32::try_from(rng.below(9)?)?) / 4.0),
        };
        for kind in [
            Kind::OpenAiChat,
            Kind::AnthropicMessages,
            Kind::OpenAiResponses,
        ] {
            let body = request_body(kind, "model-x", temperature, &turns);
            let sent: Value = lgwks_std::json::from_slice(&body)?;
            assert_eq!(
                sent.get("model").and_then(Value::as_str),
                Some("model-x"),
                "seed {seed}: {kind:?} names the model"
            );
            match temperature {
                Some(degrees) => assert_eq!(
                    sent.get("temperature"),
                    Some(&json!(degrees)),
                    "seed {seed}: {kind:?} sends a pinned temperature"
                ),
                None => assert!(
                    sent.get("temperature").is_none(),
                    "seed {seed}: {kind:?} sends no temperature member"
                ),
            }
            assert!(
                sent.get("stream").is_none(),
                "seed {seed}: {kind:?} never streams"
            );
            match kind {
                Kind::OpenAiChat => {
                    let messages = sent
                        .get("messages")
                        .and_then(Value::as_array)
                        .ok_or("a messages list")?;
                    assert_eq!(messages.len(), 3, "seed {seed}: chat keeps every turn");
                    assert!(
                        sent.get("input").is_none() && sent.get("system").is_none(),
                        "seed {seed}: chat sends no input or system member"
                    );
                }
                Kind::AnthropicMessages => {
                    let messages = sent
                        .get("messages")
                        .and_then(Value::as_array)
                        .ok_or("a messages list")?;
                    assert_eq!(
                        messages.len(),
                        2,
                        "seed {seed}: the system turn rides as system, not in messages"
                    );
                    assert_eq!(
                        sent.get("system").and_then(Value::as_str),
                        Some("be terse"),
                        "seed {seed}: the system turn rides as system"
                    );
                    assert_eq!(
                        sent.get("max_tokens").and_then(Value::as_u64),
                        Some(u64::from(ADAPTER_MAX_TOKENS)),
                        "seed {seed}: max_tokens rides along"
                    );
                    assert!(
                        sent.get("input").is_none(),
                        "seed {seed}: messages sends no input member"
                    );
                }
                Kind::OpenAiResponses => {
                    assert!(
                        sent.get("messages").is_none() && sent.get("system").is_none(),
                        "seed {seed}: responses sends only input"
                    );
                    let input = sent
                        .get("input")
                        .and_then(Value::as_array)
                        .ok_or("an input list")?;
                    assert_eq!(input.len(), 3, "seed {seed}: responses keeps every turn");
                }
            }
        }
    }
    Ok(())
}

/// larry#205: each new kind reads its own reply into the one `Usage` shape: a
/// finished reply keeps its text and counts, and an impossible usage is
/// dropped. A structured file reply is its `content` string; anything else is
/// refused, never written. Catches a parser that knows only the chat envelope
/// or prices an impossible report.
#[test]
fn sim_each_new_kind_reads_its_reply_to_one_shape() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let prompt = rng.below(5000)?.saturating_add(10);
        let completion = rng.below(500)?;
        let cached = rng.below(prompt.saturating_add(1))?;
        let written = prompt.saturating_sub(cached);
        let want = Some(Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_tokens: Some(cached),
            cache_write_tokens: Some(written),
        });
        let anthropic = json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "tool_use", "id": "x", "name": "y", "input": {}},
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": prompt,
                "output_tokens": completion,
                "cache_read_input_tokens": cached,
                "cache_creation_input_tokens": written,
            },
        })
        .to_string();
        let reply = parse_completion(Kind::AnthropicMessages, anthropic.as_bytes())?;
        assert_eq!(
            reply.content(),
            "hello",
            "seed {seed}: only text blocks are content"
        );
        assert_eq!(
            reply.usage(),
            want,
            "seed {seed}: the counts land in one shape"
        );
        let responses = json!({
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": []},
                {"type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": "hel"},
                    {"type": "refusal", "refusal": "no"},
                    {"type": "output_text", "text": "lo"},
                ]},
            ],
            "usage": {
                "input_tokens": prompt,
                "output_tokens": completion,
                "input_tokens_details": {"cached_tokens": cached},
            },
        })
        .to_string();
        let reply = parse_completion(Kind::OpenAiResponses, responses.as_bytes())?;
        assert_eq!(
            reply.content(),
            "hello",
            "seed {seed}: only output text is content"
        );
        assert_eq!(
            reply.usage(),
            Some(Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                cached_tokens: Some(cached),
                cache_write_tokens: None,
            }),
            "seed {seed}: the cached input tokens land in one shape"
        );
        assert_eq!(
            structured_content(r#"{"content": "fn main() {}\n"}"#)?,
            "fn main() {}\n",
            "seed {seed}: a structured reply is its content string"
        );
        for bad in [r#"{"content": 7}"#, r#"{"other": "x"}"#] {
            assert!(
                matches!(structured_content(bad), Err(ModelError::Malformed(_))),
                "seed {seed}: {bad:?} is no structured reply"
            );
        }
        for bad in ["hi", "", r#"{"content": "ok""#] {
            assert!(
                matches!(structured_content(bad), Err(ModelError::NotJson { .. })),
                "seed {seed}: {bad:?} is no JSON document"
            );
        }
    }
    Ok(())
}

/// larry#205: a reply the model did not finish is refused naming why, and a
/// reply with no content is malformed, on each new kind. Catches a parser that
/// reads a cut reply as a file.
#[test]
fn sim_each_new_kind_refuses_an_unfinished_or_torn_reply() {
    for (kind, cut) in [
        (
            Kind::AnthropicMessages,
            r#"{"content": [], "stop_reason": "max_tokens", "usage": {"input_tokens": 3, "output_tokens": 0}}"#,
        ),
        (
            Kind::OpenAiResponses,
            r#"{"status": "incomplete", "output": [], "usage": {"input_tokens": 3, "output_tokens": 0}}"#,
        ),
    ] {
        assert!(
            matches!(
                parse_completion(kind, cut.as_bytes()),
                Err(ModelError::Unfinished { .. })
            ),
            "{kind:?}: an unfinished reply is refused"
        );
    }
    for (kind, torn, why) in [
        (
            Kind::AnthropicMessages,
            r#"{"stop_reason": "end_turn", "usage": {"input_tokens": 3, "output_tokens": 0}}"#,
            "the reply names no content",
        ),
        (
            Kind::OpenAiResponses,
            r#"{"status": "completed", "usage": {"input_tokens": 3, "output_tokens": 0}}"#,
            "the reply names no output",
        ),
    ] {
        assert!(
            matches!(
                parse_completion(kind, torn.as_bytes()),
                Err(ModelError::Malformed(_))
            ),
            "{kind:?}: {why}"
        );
    }
    for (kind, raw) in [
        (Kind::AnthropicMessages, "not json"),
        (Kind::OpenAiResponses, "not json"),
    ] {
        assert!(
            matches!(
                parse_completion(kind, raw.as_bytes()),
                Err(ModelError::NotJson { .. })
            ),
            "{kind:?}: a reply that is not JSON is refused as itself"
        );
    }
}
