//! Seeded simulations of the `usage` block a completion reply carries (larry#230).
//! Source: the OpenAI chat completion object (`usage.prompt_tokens_details.cached_tokens`),
//! the DeepSeek KV cache guide (`usage.prompt_cache_hit_tokens`) and Anthropic prompt
//! caching (`usage.cache_read_input_tokens`, `usage.cache_creation_input_tokens`).
//! Requirement: every provider's cache counts reach the cost as cached and written
//! tokens; a report that caches more than the prompt held is never priced; a stream
//! keeps the usage of its last chunk. One seed drives every count, shape and framing.

use std::error::Error;
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::json::{Map, Value, json};
use lgwks_std::seeded::{DrawError, Seeded};

use super::{
    MAX_CODE_CHARS, MAX_DETAIL_CHARS, MAX_ERROR_BODY, parse_completion, refusal_detail,
    request_body,
};
use crate::model::{
    Kind, MAX_MESSAGES, Message, ModelClient, ModelError, Role, Usage, WindowRefusal, sim_config,
};

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds the simulation runs.
const SEEDS: u64 = 256;

/// The usage a reply reports, or the failure that read it.
fn reported(text: &str) -> Result<Option<Usage>, ModelError> {
    Ok(parse_completion(Kind::OpenAiChat, text.as_bytes())?.usage())
}

/// The usage a reader must report for these counts.
const fn counts(prompt: u64, completion: u64, cached: Option<u64>, written: Option<u64>) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        cached_tokens: cached,
        cache_write_tokens: written,
    }
}

/// A bare completion document whose `usage` member is the JSON text `usage`.
fn bare(usage: &str) -> String {
    format!(r#"{{"choices":[{{"message":{{"content":"ok"}}}}],"usage":{usage}}}"#)
}

/// Each usage text a gateway sends and the usage a reader must report for it.
fn fixed_cases() -> [(&'static str, Option<Usage>); 13] {
    [
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":8}}"#,
            Some(counts(10, 2, Some(8), None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":8}"#,
            Some(counts(10, 2, Some(8), None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"cache_read_input_tokens":8}"#,
            Some(counts(10, 2, Some(8), None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cache_write_tokens":4}}"#,
            Some(counts(10, 2, None, Some(4))),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"cache_creation_input_tokens":4}"#,
            Some(counts(10, 2, None, Some(4))),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"cache_read_input_tokens":6,"cache_creation_input_tokens":4}"#,
            Some(counts(10, 2, Some(6), Some(4))),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2}"#,
            Some(counts(10, 2, None, None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"cache_read_input_tokens":8,"cache_creation_input_tokens":4}"#,
            Some(counts(10, 2, None, None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":11}"#,
            Some(counts(10, 2, None, None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":8},"prompt_cache_hit_tokens":3}"#,
            Some(counts(10, 2, Some(8), None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":-1,"cache_read_input_tokens":5}"#,
            Some(counts(10, 2, Some(5), None)),
        ),
        (
            r#"{"prompt_tokens":10,"completion_tokens":2,"cache_read_input_tokens":"5"}"#,
            Some(counts(10, 2, None, None)),
        ),
        (r#"{"prompt_tokens":10,"cached_tokens":5}"#, None),
    ]
}

/// weird-bytes and liar-upstream: each provider's cached and written shapes read as
/// the same counts, the first present shape wins, a negative or string count is no
/// report, cached plus written above the prompt drops both, and a stream keeps the
/// usage of its last chunk through every framing. Catches a reader that knows only
/// the OpenAI shape, prices an impossible report, or loses a stream's usage.
#[test]
fn cached_tokens_are_read_from_every_usage_shape_and_sse() -> TestResult {
    for (usage, want) in fixed_cases() {
        assert_eq!(reported(&bare(usage))?, want, "bare {usage}");
        let marked = format!("{}\n\ndata: [DONE]\n\n", bare(usage));
        assert_eq!(reported(&marked)?, want, "with the done marker {usage}");
    }
    let last =
        r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":8}}"#;
    let stream = format!(
        concat!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"he\"}}}}],\"usage\":null}}\n\n",
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"llo\"}}}}],\"usage\":null}}\n\n",
            "data: {{\"choices\":[],\"usage\":{}}}\n\n",
            "data: [DONE]\n\n",
        ),
        last
    );
    let completion = parse_completion(Kind::OpenAiChat, stream.as_bytes())?;
    assert_eq!(
        completion.content(),
        "hello",
        "the content is assembled from the deltas"
    );
    assert_eq!(
        completion.usage(),
        Some(counts(10, 2, Some(8), None)),
        "the last chunk's usage survives the stream and its done marker"
    );
    let silent = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
    assert_eq!(
        reported(silent)?,
        None,
        "a stream that reports nothing is priced at nothing"
    );
    let text = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}],\"usage\":\"lots\"}\n\n";
    assert_eq!(
        reported(text)?,
        None,
        "a usage that is no object is ignored"
    );
    Ok(())
}

/// A structured refusal carries its `error.code` before its message, so a
/// translated message still says what the refusal is, a code alone is the whole
/// reason, control characters never reach an operator, and a body past the read
/// bound is not parsed as a document at all (larry#236).
#[test]
fn a_refusal_keeps_the_gateways_code_and_reads_a_bounded_body() {
    assert_eq!(
        refusal_detail(
            br#"{"error":{"code":"context_length_exceeded","message":"the prompt is too long"}}"#
        ),
        "context_length_exceeded: the prompt is too long"
    );
    assert_eq!(
        refusal_detail(br#"{"error":{"code":"context_length_exceeded"}}"#),
        "context_length_exceeded",
        "a code alone is the reason"
    );
    assert_eq!(
        refusal_detail(br#"{"error":{"message":"out of memory"}}"#),
        "out of memory"
    );
    assert_eq!(refusal_detail(b"plain refusal"), "plain refusal");
    assert_eq!(
        refusal_detail(br#"{"error":{"message":"a\nb\tc"}}"#),
        "a b c",
        "a control character is blanked"
    );
    assert_eq!(
        refusal_detail("{\"error\":{\"message\":\"a\u{202e}bc\u{2066}d\u{2069}\"}}".as_bytes()),
        "a bc d ",
        "a bidi control is blanked, so a refusal cannot reorder a terminal (CVE-2021-42574)"
    );
    let long_code = format!(r#"{{"error":{{"code":"{}"}}}}"#, "c".repeat(100));
    assert_eq!(
        refusal_detail(long_code.as_bytes()).chars().count(),
        MAX_CODE_CHARS,
        "a code past its bound is cut"
    );
    let long = format!(
        r#"{{"error":{{"message":"{}"}}}}"#,
        "x".repeat(MAX_ERROR_BODY)
    );
    let detail = refusal_detail(long.as_bytes());
    assert_eq!(
        detail.chars().count(),
        MAX_DETAIL_CHARS,
        "the reason is bounded"
    );
    assert!(
        detail.starts_with("{\"error\""),
        "a body past the read bound is not parsed as a document: {detail:.20}"
    );
    let hostile = refusal_detail(
        br#"{"error":{"message":"maximum context length is 99999999999999999999999999 tokens"}}"#,
    );
    let refusal = ModelError::Client {
        status: 400,
        detail: hostile,
    };
    assert!(
        refusal.is_context_overflow(),
        "the words still name the window"
    );
    assert_eq!(
        refusal.window(),
        WindowRefusal::default(),
        "a 26 digit count is no number and the body is no panic"
    );
}

/// One seeded reply: the counts a gateway reports and how it dresses them.
struct Draw {
    /// `usage.prompt_tokens`.
    prompt: u64,
    /// `usage.completion_tokens`, sent only when `complete`.
    completion: u64,
    /// Whether the gateway sends `completion_tokens` at all.
    complete: bool,
    /// Cached tokens the gateway reports, when it does.
    cached: Option<u64>,
    /// Written tokens the gateway reports, when it does.
    written: Option<u64>,
    /// When `cached` is absent, whether the cached member holds a string instead.
    junk: bool,
    /// Which of the three cached shapes carries `cached`.
    cached_shape: u64,
    /// Which of the two written shapes carries `written`.
    write_shape: u64,
    /// Bare document, document with a done marker, or an SSE stream.
    framing: u64,
}

/// A count in `0..=prompt`, past it, huge or absent.
fn count_draw(rng: &mut Seeded, prompt: u64) -> Result<Option<u64>, DrawError> {
    let within = rng.below(prompt.saturating_add(1))?;
    Ok(match rng.below(4)? {
        0 => None,
        1 => Some(within),
        2 => Some(within.saturating_add(rng.below(3)?)),
        _ => Some(u64::MAX),
    })
}

/// The draw for `seed`.
fn draw(seed: u64) -> Result<Draw, DrawError> {
    let mut rng = Seeded::from_seed(seed);
    let prompt = if rng.below(8)? == 0 {
        u64::MAX.saturating_sub(rng.below(4)?)
    } else {
        rng.below(5000)?
    };
    let completion = rng.below(5000)?;
    let complete = rng.below(8)? != 0;
    let cached = count_draw(&mut rng, prompt)?;
    let written = count_draw(&mut rng, prompt)?;
    let junk = rng.below(4)? == 0;
    Ok(Draw {
        prompt,
        completion,
        complete,
        cached,
        written,
        junk,
        cached_shape: rng.below(3)?,
        write_shape: rng.below(2)?,
        framing: rng.below(3)?,
    })
}

/// Sets `key` of `map` to the whole number `count`.
fn put(map: &mut Map, key: &str, count: u64) {
    map.insert(String::from(key), json!(count));
}

/// The `usage` object the draw's gateway sends.
fn usage_value(draw: &Draw) -> Value {
    let mut usage = Map::new();
    put(&mut usage, "prompt_tokens", draw.prompt);
    if draw.complete {
        put(&mut usage, "completion_tokens", draw.completion);
    }
    let mut details = Map::new();
    let mut detailed = false;
    match (draw.cached, draw.cached_shape) {
        (Some(count), 0) => {
            put(&mut details, "cached_tokens", count);
            detailed = true;
        }
        (Some(count), 1) => put(&mut usage, "prompt_cache_hit_tokens", count),
        (Some(count), _) => put(&mut usage, "cache_read_input_tokens", count),
        (None, shape) if draw.junk => {
            let key = if shape == 2 {
                "cache_read_input_tokens"
            } else {
                "prompt_cache_hit_tokens"
            };
            usage.insert(String::from(key), json!("7"));
        }
        (None, _) => {}
    }
    match (draw.written, draw.write_shape) {
        (Some(count), 0) => {
            put(&mut details, "cache_write_tokens", count);
            detailed = true;
        }
        (Some(count), _) => put(&mut usage, "cache_creation_input_tokens", count),
        (None, _) => {}
    }
    if detailed {
        usage.insert(
            String::from("prompt_tokens_details"),
            Value::Object(details),
        );
    }
    Value::Object(usage)
}

/// The reply body the draw's gateway sends, in its framing.
fn body(draw: &Draw) -> String {
    let usage = usage_value(draw);
    match draw.framing {
        0 => json!({"choices": [{"message": {"content": "hello"}}], "usage": usage}).to_string(),
        1 => format!(
            "{}\n\ndata: [DONE]\n\n",
            json!({"choices": [{"message": {"content": "hello"}}], "usage": usage})
        ),
        _ => format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices": [{"delta": {"content": "he"}}], "usage": null}),
            json!({"choices": [{"delta": {"content": "llo"}}], "usage": null}),
            json!({"choices": [], "usage": usage}),
        ),
    }
}

/// The reference model: the usage a reader must report for the draw. Counts are
/// compared as wide integers, so no sum can wrap.
fn reference(draw: &Draw) -> Option<Usage> {
    if !draw.complete {
        return None;
    }
    let wide = |count: Option<u64>| count.into_iter().map(u128::from).sum::<u128>();
    let possible = wide(draw.cached).saturating_add(wide(draw.written)) <= u128::from(draw.prompt);
    Some(Usage {
        prompt_tokens: draw.prompt,
        completion_tokens: draw.completion,
        cached_tokens: draw.cached.filter(|_| possible),
        cache_write_tokens: draw.written.filter(|_| possible),
    })
}

/// weird-bytes, giant-and-empty and liar-upstream: for 256 seeds a gateway reports
/// prompt, completion, cached and written counts (absent, within the prompt, past
/// it, `u64::MAX`, or a string) in a seeded cached shape, write shape and framing,
/// and the parsed usage equals the reference model's. Catches a reader that wraps a
/// sum past `u64::MAX`, prices cached plus written above the prompt, misses a
/// provider's shape, or reads a string as a count.
#[test]
fn sim_usage_shapes_round_trip_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let drawn = draw(seed)?;
        let text = body(&drawn);
        let completion = parse_completion(Kind::OpenAiChat, text.as_bytes())?;
        assert_eq!(
            completion.content(),
            "hello",
            "seed {seed}: the content survives its framing {}",
            drawn.framing
        );
        assert_eq!(
            completion.usage(),
            reference(&drawn),
            "seed {seed}: usage of {text}"
        );
    }
    Ok(())
}

/// A client that only answers `fits`: its pause refuses every wait and it never sends.
fn client(bound: Option<usize>) -> Result<ModelClient, ModelError> {
    Ok(ModelClient::new(
        sim_config(bound)?,
        Clock::virtual_at(Duration::ZERO),
        |_wait| false,
    ))
}

/// giant-and-empty (larry#230: the executor shares a prompt prefix only when the
/// client's own pre-send check passes): for 64 seeds a request whose encoded body is
/// exactly the route's bound fits, one byte more or a bound one byte less does not,
/// and exactly MAX_MESSAGES messages fit where one more does not. Catches a client
/// whose `fits` is off by one at either bound, which would share a prefix the send
/// then refuses as BodyTooLarge, or drop one the route could carry.
#[test]
fn sim_client_fits_exactly_at_its_byte_and_message_bounds() -> TestResult {
    let defaults = sim_config(None)?;
    for seed in 0..64_u64 {
        let mut rng = Seeded::from_seed(seed);
        let filler = "q".repeat(usize::try_from(rng.below(3000)?.saturating_add(1100))?);
        let sent = [
            Message::new(Role::System, "sys"),
            Message::new(Role::User, filler.as_str()),
        ];
        let size = request_body(
            Kind::OpenAiChat,
            defaults.model(),
            defaults.temperature(),
            &sent,
        )
        .len();
        assert!(
            client(Some(size))?.fits(&sent),
            "seed {seed}: {size} bytes fit a {size} byte bound"
        );
        assert!(
            !client(Some(size.saturating_sub(1)))?.fits(&sent),
            "seed {seed}: {size} bytes pass a bound one byte smaller"
        );
        let longer = [
            Message::new(Role::System, "sys"),
            Message::new(Role::User, format!("{filler}q")),
        ];
        assert!(
            !client(Some(size))?.fits(&longer),
            "seed {seed}: one byte past the bound does not fit"
        );
    }
    let wide = client(None)?;
    let full: Vec<Message> = (0..MAX_MESSAGES)
        .map(|_| Message::new(Role::User, "m"))
        .collect();
    assert!(wide.fits(&full), "exactly {MAX_MESSAGES} messages fit");
    let over: Vec<Message> = (0..=MAX_MESSAGES)
        .map(|_| Message::new(Role::User, "m"))
        .collect();
    assert!(
        !wide.fits(&over),
        "one message past {MAX_MESSAGES} does not fit"
    );
    Ok(())
}

/// Each `finish_reason` a gateway sends, as JSON text (empty: no member), and whether a reader
/// reads the reply as finished. Source: the OpenAI chat completion object (`stop`, `length`,
/// `content_filter`, `tool_calls`), Anthropic's stop reasons (`end_turn`, `stop_sequence`,
/// `max_tokens`, `refusal`) and Gemini's finish reasons (`STOP`, `MAX_TOKENS`, `SAFETY`).
const REASONS: [(&str, bool); 14] = [
    ("", true),
    ("null", true),
    ("\"stop\"", true),
    ("\"STOP\"", true),
    ("\"end_turn\"", true),
    ("\"stop_sequence\"", true),
    ("\"length\"", false),
    ("\"content_filter\"", false),
    ("\"max_tokens\"", false),
    ("\"MAX_TOKENS\"", false),
    ("\"SAFETY\"", false),
    ("\"tool_calls\"", false),
    ("\"refusal\"", false),
    ("\"\\u001b[31mlength\"", false),
];

/// The reply body for `reason` in `framing`: a bare document, the document with a done
/// marker, or an SSE stream whose last chunk carries only the reason.
fn finish_body(reason: &str, framing: u64, content: &str) -> String {
    let member = if reason.is_empty() {
        String::new()
    } else {
        format!(",\"finish_reason\":{reason}")
    };
    let content = json!(content).to_string();
    let document = format!(r#"{{"choices":[{{"message":{{"content":{content}}}{member}}}]}}"#);
    match framing {
        0 => document,
        1 => format!("{document}\n\ndata: [DONE]\n\n"),
        _ => format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{content}}},\"finish_reason\":null}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{}}{member}}}]}}\n\ndata: [DONE]\n\n"
        ),
    }
}

/// weird-bytes and liar-upstream (larry#396): for 256 seeds a gateway sends a seeded finish
/// reason in a seeded framing, and the reader passes exactly the finished ones with their content
/// and refuses every other as `Unfinished`, naming the reason with no control character. Catches
/// a reader that ignores the reason, knows only OpenAI's words, compares case, or loses a
/// stream's last reason.
#[test]
fn sim_every_finish_reason_in_every_framing_gets_the_reference_verdict() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let index = usize::try_from(rng.below(u64::try_from(REASONS.len())?)?)?;
        let (reason, done) = *REASONS.get(index).ok_or("a drawn reason")?;
        let framing = rng.below(3)?;
        let text = finish_body(reason, framing, "fn main() {}\n");
        match (parse_completion(Kind::OpenAiChat, text.as_bytes()), done) {
            (Ok(completion), true) => assert_eq!(
                completion.content(),
                "fn main() {}\n",
                "seed {seed}: {reason} in framing {framing} keeps its content"
            ),
            (Err(ModelError::Unfinished { reason: named }), false) => assert!(
                named.contains("length")
                    || named.contains("filter")
                    || named.contains("tokens")
                    || named.contains("TOKENS")
                    || named.contains("SAFETY")
                    || named.contains("calls")
                    || named.contains("refusal"),
                "seed {seed}: the refusal names {reason}: {named:?}"
            ),
            (outcome, _) => {
                return Err(
                    format!("seed {seed}: {reason} in framing {framing} gave {outcome:?}").into(),
                );
            }
        }
    }
    for (reason, done) in REASONS {
        for framing in 0..3 {
            let outcome = parse_completion(
                Kind::OpenAiChat,
                finish_body(reason, framing, "x").as_bytes(),
            );
            assert_eq!(outcome.is_ok(), done, "{reason} in framing {framing}");
            if let Err(ModelError::Unfinished { reason: named }) = outcome {
                assert!(
                    !named.chars().any(char::is_control),
                    "{reason}: no control character reaches an operator: {named:?}"
                );
            }
        }
    }
    let filtered = r#"{"choices":[{"message":{"content":null},"finish_reason":"content_filter"}]}"#;
    assert_eq!(
        parse_completion(Kind::OpenAiChat, filtered.as_bytes()),
        Err(ModelError::Unfinished {
            reason: String::from("content_filter")
        }),
        "a filtered reply with no content is named by its reason"
    );
    let numbered = r#"{"choices":[{"message":{"content":"x"},"finish_reason":7}]}"#;
    assert_eq!(
        parse_completion(Kind::OpenAiChat, numbered.as_bytes()),
        Err(ModelError::Malformed(
            "the reply's finish_reason is not text"
        ))
    );
    Ok(())
}

/// larry#396: a stream chunk that is not JSON refuses the reply by its line, where the reader
/// once skipped it and wrote what was left. Catches a reader that drops a torn chunk.
#[test]
fn a_torn_stream_chunk_refuses_the_reply_by_its_line() {
    let torn = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"fn main\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"() {\n\n",
        "data: [DONE]\n\n",
    );
    assert!(
        matches!(
            parse_completion(Kind::OpenAiChat, torn.as_bytes()),
            Err(ModelError::StreamChunk { line: 3, .. })
        ),
        "the torn chunk on line 3 refuses the reply"
    );
}

/// A context code is a member of the refusal's code, never a prefix of the whole reason:
/// a namespaced code is an overflow and a longer member that merely contains the stem is
/// not (larry#343).
#[test]
fn a_namespaced_context_code_is_an_overflow() {
    let namespaced = ModelError::Client {
        status: 400,
        detail: String::from("invalid_request_error.context_length_exceeded: too long"),
    };
    assert!(
        namespaced.is_context_overflow(),
        "a namespaced code names the context window"
    );
    let unrelated = ModelError::Client {
        status: 400,
        detail: String::from("max_context_length: not a context overflow"),
    };
    assert!(
        !unrelated.is_context_overflow(),
        "a longer member is not the code itself"
    );
    let bare = ModelError::Client {
        status: 400,
        detail: String::from("context_length_exceeded: too long"),
    };
    assert!(bare.is_context_overflow(), "a plain code still reads");
}

/// larry#205: one adapter per provider kind. Each kind's name parses back to
/// it, and a route that pins no temperature sends no `temperature` member: a
/// model that rejects the field answers every request with a 4xx, which is
/// terminal. Catches a client that knows only the OpenAI chat shape and a body
/// that always carries the field.
#[test]
fn sim_each_kind_sends_its_body_shape_and_temperature_rule() -> TestResult {
    for name in ["openai-chat", "anthropic-messages", "openai-responses"] {
        let kind = crate::model::Kind::parse(name).ok_or("a kind parses")?;
        assert_eq!(kind.name(), name, "{name} round trips");
    }
    let unpinned = client(None)?;
    let body = unpinned.encode(&[Message::new(Role::User, "probe")]);
    let text = String::from_utf8(body.body().to_vec())?;
    assert!(
        !text.contains("temperature"),
        "a route with no temperature sends none: {text}"
    );
    Ok(())
}
