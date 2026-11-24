//! Seeded simulations of a terminal session learning its model's context window
//! from a gateway's own refusal (larry#236).
//!
//! Source: the refusals OpenAI, Anthropic, Gemini and vLLM send when a prompt is
//! past the model's context window, RFC 9110 15.5.14 (413 Content Too Large) and
//! the seeded-simulation rule (one seed, one trace). Requirement proved: a
//! refusal that names the model's limit shrinks the window to that limit, one
//! such refusal brings the next request under it, a number no `u64` holds is no
//! number, and the bound on shrinks still ends the session typed.

use std::error::Error;
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::seeded::Seeded;

use super::{Answer, Conversation, MAX_SHRINKS, Overflow, quarters};
use crate::model::{ModelClient, ModelError, WindowRefusal, sim_config};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds each simulation runs.
const SEEDS: u64 = 24;

/// The system turn every simulated conversation opens with.
const SYSTEM: &str = "You are Larry, a terminal agent.";

/// A drawn number below `bound`, as a length.
fn upto(rng: &mut Seeded, bound: u64) -> Result<usize, SimError> {
    Ok(usize::try_from(rng.below(bound)?)?)
}

/// A client that never sends, whose route bound is `window` bytes.
fn client(window: usize) -> Result<ModelClient, ModelError> {
    Ok(ModelClient::new(
        sim_config(Some(window))?,
        Clock::virtual_at(Duration::ZERO),
        |_wait| false,
    ))
}

/// A gateway's `status` refusal carrying `detail`.
fn client_error(status: u16, detail: &str) -> ModelError {
    ModelError::Client {
        status,
        detail: detail.to_owned(),
    }
}

/// One step a session ran: a short command and `output` bytes of stdout.
fn worked(index: usize, output: usize) -> (String, Answer) {
    let command = format!("cat part-{index}");
    let reply = format!("{{\"command\":\"{command}\"}}");
    let answer = Answer::Observed {
        step: index,
        command,
        status: String::from("exit status 0"),
        output: Some(format!("stdout:\n{}\nstderr:\n", "a".repeat(output))),
    };
    (reply, answer)
}

/// The refusal a gateway sends past its window in `shape` (larry#236): OpenAI
/// and Gemini name the limit and the count, Anthropic names the count first,
/// and vLLM (shape 3) names only the limit.
fn refusal_text(shape: usize, counted: u64, limit: u64) -> String {
    match shape {
        0 => format!(
            "This model's maximum context length is {limit} tokens. However, your messages resulted in {counted} tokens."
        ),
        1 => format!("prompt is too long: {counted} tokens > {limit} maximum"),
        2 => format!(
            "The input token count ({counted}) exceeds the maximum number of tokens allowed ({limit})."
        ),
        _ => format!("This model's maximum context length is {limit} tokens."),
    }
}

/// A 413, or a 400 whose code or words name the context window, is an overflow
/// and carries the numbers its gateway named (larry#236): OpenAI and Gemini
/// name the limit and the count, Anthropic names the count first and the limit
/// after `maximum`, vLLM names the limit alone, a code alone names neither, and
/// any other refusal, the same words on a 5xx, or a number no `u64` holds is
/// not one and invents nothing.
#[test]
fn context_overflow_is_a_413_or_a_400_naming_the_window() {
    for (detail, named) in [
        (
            "This model's maximum context length is 128000 tokens. However, your messages resulted in 130112 tokens.",
            WindowRefusal {
                counted: Some(130_112),
                limit: Some(128_000),
            },
        ),
        (
            "prompt is too long: 210000 tokens > 200000 maximum",
            WindowRefusal {
                counted: Some(210_000),
                limit: Some(200_000),
            },
        ),
        (
            "The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).",
            WindowRefusal {
                counted: Some(1_200_000),
                limit: Some(1_048_576),
            },
        ),
        (
            "This model's maximum context length is 4096 tokens.",
            WindowRefusal {
                counted: None,
                limit: Some(4_096),
            },
        ),
        (
            "context_length_exceeded",
            WindowRefusal {
                counted: None,
                limit: None,
            },
        ),
        (
            "Input exceeds the Context Window of this model",
            WindowRefusal {
                counted: None,
                limit: None,
            },
        ),
    ] {
        let refusal = client_error(400, detail);
        assert!(refusal.is_context_overflow(), "{detail}");
        assert_eq!(refusal.window(), named, "{detail}");
    }
    // larry#295: a gateway whose `error.code` is the bare `context_length`, with or
    // without a message, still names the window, and names no number.
    for code_only in ["context_length", "context_length: request too large"] {
        let refusal = client_error(400, code_only);
        assert!(refusal.is_context_overflow(), "{code_only}");
        assert_eq!(refusal.window(), WindowRefusal::default(), "{code_only}");
    }
    assert!(client_error(413, "").is_context_overflow(), "413");
    assert_eq!(
        client_error(413, "too large").window(),
        WindowRefusal::default(),
        "a 413 that names no number carries none"
    );
}

/// A number no `u64` holds, or none at all, is no number: the refusal is still an
/// overflow, and it names only what it can read.
#[test]
fn a_hostile_number_in_a_refusal_names_no_window() {
    for (hostile, named) in [
        (
            "maximum context length is 99999999999999999999999999 tokens",
            WindowRefusal {
                counted: None,
                limit: None,
            },
        ),
        (
            "prompt is too long: 99999999999999999999999999 tokens > 200000 maximum",
            WindowRefusal {
                counted: None,
                limit: Some(200_000),
            },
        ),
        (
            "maximum context length is  tokens",
            WindowRefusal {
                counted: None,
                limit: None,
            },
        ),
    ] {
        let refusal = client_error(400, hostile);
        assert!(refusal.is_context_overflow(), "{hostile}");
        assert_eq!(refusal.window(), named, "{hostile}");
    }
}

/// Any other refusal, the same words on a 5xx, a 401, a timeout or an oversize body is
/// no overflow and names no window.
#[test]
fn a_refusal_that_is_no_overflow_names_no_window() {
    for refusal in [
        client_error(400, "temperature must be at most 1"),
        // larry#295: a code is read only where the wire puts it, first, so a
        // message that names a `max_context_length` parameter is no overflow.
        client_error(
            400,
            "invalid_request_error: max_context_length must be positive",
        ),
        client_error(401, "context length"),
        ModelError::Server {
            status: 500,
            detail: String::from("maximum context length"),
        },
        ModelError::BodyTooLarge,
        ModelError::Timeout,
    ] {
        assert!(!refusal.is_context_overflow(), "{refusal:?}");
        assert_eq!(refusal.window(), WindowRefusal::default(), "{refusal:?}");
    }
}

/// A refusal that names no number shrinks the window to three quarters of the
/// refused request, a named one never grows it, and the ninth is refused typed
/// with the window left where the eighth put it.
#[test]
fn a_refused_window_shrinks_to_three_quarters_and_stops_after_eight() -> TestResult {
    let mut conversation = Conversation::new(SYSTEM, String::from("task"), 1_048_576);
    let mut len = 1_048_576_usize;
    for _ in 0..MAX_SHRINKS {
        conversation.shrink(len, WindowRefusal::default())?;
        assert_eq!(
            conversation.window,
            quarters(len, 3),
            "three quarters of {len}"
        );
        len = conversation.window;
    }
    assert_eq!(
        conversation.shrink(len, WindowRefusal::default()),
        Err(Overflow::Shrinks),
        "the ninth"
    );
    assert_eq!(conversation.window, len, "unchanged by the refusal");
    assert_eq!(conversation.tally.shrinks, MAX_SHRINKS, "counted");
    let mut fresh = Conversation::new(SYSTEM, String::from("task"), 4096);
    fresh.shrink(1_048_576, WindowRefusal::default())?;
    assert_eq!(fresh.window, 4096, "a refusal never grows the window");
    let huge = WindowRefusal {
        counted: Some(1),
        limit: Some(u64::MAX),
    };
    fresh.shrink(1_048_576, huge)?;
    assert_eq!(fresh.window, 4096, "no named limit grows the window either");
    Ok(())
}

/// A refusal that names the model's limit shrinks the window to nine tenths of
/// the bytes that limit leaves: at the rate the refusal shows, at one byte a
/// token when it names only the limit, and at three quarters when it names no
/// number or an impossible one (larry#236).
#[test]
fn a_named_limit_shrinks_to_the_model_own_window() -> TestResult {
    let shrink = |len: usize, detail: &str| -> Result<usize, Box<dyn Error>> {
        let refusal = ModelError::Client {
            status: 400,
            detail: detail.to_owned(),
        };
        assert!(refusal.is_context_overflow(), "{detail}");
        let mut conversation = Conversation::new(SYSTEM, String::from("task"), len);
        conversation.shrink(len, refusal.window())?;
        Ok(conversation.window)
    };
    assert_eq!(
        shrink(
            1_200_000,
            "This model's maximum context length is 100000 tokens. However, your messages resulted in 120000 tokens.",
        )?,
        900_000,
        "OpenAI names both numbers"
    );
    assert_eq!(
        shrink(
            2_100_000,
            "prompt is too long: 210000 tokens > 200000 maximum"
        )?,
        1_800_000,
        "Anthropic names the limit after the count"
    );
    assert_eq!(
        shrink(
            1_200_000,
            "The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).",
        )?,
        943_719,
        "Gemini names both numbers in parentheses"
    );
    assert_eq!(
        shrink(
            1_000_000,
            "This model's maximum context length is 100000 tokens."
        )?,
        90_000,
        "vLLM names only the limit, at one byte a token"
    );
    // A gateway that counts zero tokens names no rate: the window is the named limit at one
    // byte a token, never three quarters of the refused request (larry#236).
    let mut zero = Conversation::new(SYSTEM, String::from("task"), 1_000_000);
    zero.shrink(
        1_000_000,
        WindowRefusal {
            counted: Some(0),
            limit: Some(100_000),
        },
    )?;
    assert_eq!(
        zero.window, 90_000,
        "a zero count sizes at one byte a token"
    );
    assert_eq!(
        shrink(1_000_000, "context_length_exceeded")?,
        750_000,
        "a code alone still takes three quarters"
    );
    assert_eq!(
        shrink(
            1_000_000,
            "maximum context length is 99999999999999999999999999 tokens",
        )?,
        750_000,
        "a count no u64 holds is no number"
    );
    let mut conversation = Conversation::new(SYSTEM, String::from("task"), 1_048_576);
    let named = WindowRefusal {
        counted: Some(120_000),
        limit: Some(100_000),
    };
    conversation.shrink(1_200_000, named)?;
    assert_eq!(
        conversation.window, 900_000,
        "a named refusal leaves nine tenths of the model's bytes"
    );
    conversation.shrink(1_200_000, named)?;
    assert_eq!(
        conversation.window, 900_000,
        "the same limit shrinks no further"
    );
    Ok(())
}

/// One refusal that names the model's limit, in any of the four gateway shapes,
/// brings the next request the session sends under that limit, for every seed
/// (larry#236). The model's own tokenizer is `rate` bytes a token, and the next
/// encoding's text tokenizes up to twice as dense; compaction toward half the
/// window covers that. Catches a shrink that reads no number, one that reads
/// the count for the limit, and one whose margin leaves a dense request over.
#[test]
fn sim_one_refusal_naming_the_limit_brings_the_next_request_under_it() -> TestResult {
    // The product's parse of vLLM's shape, not the sim's own formatting (larry#295).
    let vllm = ModelError::Client {
        status: 400,
        detail: refusal_text(3, 4_096, 4_096),
    };
    assert!(vllm.is_context_overflow(), "vLLM's shape is an overflow");
    assert_eq!(
        vllm.window(),
        WindowRefusal {
            counted: None,
            limit: Some(4_096),
        },
        "vLLM's shape names only the limit"
    );
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 48));
        let rate = upto(&mut rng, 5)?.saturating_add(1);
        let half = u64::try_from(rate.div_euclid(2).saturating_add(1))?;
        let denser = rate.saturating_sub(upto(&mut rng, half)?).max(1);
        let shape = upto(&mut rng, 4)?;
        let model = client(1_048_576)?;
        let mut conversation =
            Conversation::new(SYSTEM, String::from("task"), model.request_bound());
        let steps = upto(&mut rng, 80)?.saturating_add(80);
        for index in 0..steps {
            let (reply, answer) = worked(index, upto(&mut rng, 400)?.saturating_add(200));
            conversation.push(reply, answer);
        }
        let len = conversation.prepare(&model)?.bytes();
        let counted = u64::try_from(len.div_euclid(rate))?;
        let limit = counted.saturating_sub(u64::try_from(upto(&mut rng, 3)?.saturating_add(1))?);
        let refusal = ModelError::Client {
            status: 400,
            detail: refusal_text(shape, counted, limit),
        };
        assert!(refusal.is_context_overflow(), "seed {seed}: shape {shape}");
        let named = refusal.window();
        assert_eq!(named.limit, Some(limit), "seed {seed}: the limit");
        let want_counted = if shape == 3 { None } else { Some(counted) };
        assert_eq!(named.counted, want_counted, "seed {seed}: the count");
        conversation.shrink(len, named)?;
        let next = conversation.prepare(&model)?.bytes();
        let tokens = u64::try_from(next.div_euclid(denser))?;
        assert!(
            tokens <= limit,
            "seed {seed}: shape {shape} sent {next} bytes at {denser} bytes a token, {tokens} tokens over the model's {limit}"
        );
    }
    Ok(())
}
