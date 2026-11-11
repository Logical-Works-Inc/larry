//! Seeded simulations of a terminal session's reply parser and output clip
//! (larry#93). Each case is built from a seed together with the answer the rule
//! gives it, so the parser is judged against the rule, never against itself.
//! Source: the Agent Client Protocol prompt turn (one model reply, one tool call)
//! and CWE-20 (improper input validation): every malformed reply gets its exact
//! refusal and nothing runs.

use std::mem::discriminant;

use lgwks_std::seeded::Seeded;

use super::super::run::clip;
use super::{
    MAX_INSTRUCTION_BYTES, OBSERVED_BYTES, Reply, ReplyFault, SessionError, check, parse_reply,
};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each simulation runs.
const SEEDS: u64 = 2048;

/// Characters a generated command draws from: ASCII, a quote, a backslash, a
/// newline, two-, three- and four-byte UTF-8.
const ALPHABET: [&str; 10] = ["a", "Z", "0", " ", "'", "\\", "\n", "é", "→", "🦝"];

/// A generated text of `len` characters.
fn text(rng: &mut Seeded, len: usize) -> Result<String, SimError> {
    let mut out = String::new();
    for _ in 0..len {
        let pick = ALPHABET
            .get(rng.index(ALPHABET.len())?)
            .ok_or(SimError::Missing("alphabet"))?;
        out.push_str(pick);
    }
    Ok(out)
}

/// A JSON string literal for `value`.
fn quoted(value: &str) -> Result<String, SimError> {
    Ok(lgwks_std::json::to_string(value)?)
}

/// One generated reply and what the rule says of it.
fn case(seed: u64) -> Result<(String, Result<Reply, ReplyFault>), SimError> {
    let mut rng = Seeded::from_seed(seed);
    let kind = rng.below(11)?;
    let len = usize::try_from(rng.below(40)?.saturating_add(1))?;
    let body = format!("x{}", text(&mut rng, len)?);
    let pad = if rng.below(2)? == 0 { "" } else { "  \n" };
    Ok(match kind {
        0 => (
            format!("{pad}{{\"command\":{}}}{pad}", quoted(&body)?),
            Ok(Reply::Command(body)),
        ),
        1 => (
            format!("{{\"done\":{}}}", quoted(&body)?),
            Ok(Reply::Done(body)),
        ),
        2 => (
            format!("```json\n{{\"command\":{}}}\n```", quoted(&body)?),
            Ok(Reply::Command(body)),
        ),
        3 => (
            format!("{{\"command\":{0},\"done\":{0}}}", quoted(&body)?),
            Err(ReplyFault::Keys),
        ),
        4 => (
            format!("{{\"run\":{}}}", quoted(&body)?),
            Err(ReplyFault::Keys),
        ),
        5 => (
            format!("{{\"command\":{len}}}"),
            Err(ReplyFault::NotAString),
        ),
        6 => (
            format!("run {body}"),
            Err(ReplyFault::NotJson(String::new())),
        ),
        7 => (
            format!("[{}]", quoted(&body)?),
            Err(ReplyFault::NotAnObject),
        ),
        8 => (
            format!("{{\"command\":{}}}", quoted(&"y".repeat(4097))?),
            Err(ReplyFault::FieldTooLong),
        ),
        9 => (
            format!("{{\"command\":{}}}", quoted(&format!("{body}\0"))?),
            Err(ReplyFault::Nul),
        ),
        _ => (
            format!("{{\"command\":{}}}", quoted(&" ".repeat(len))?),
            Err(ReplyFault::EmptyCommand),
        ),
    })
}

/// Whether `got` is the rule's answer: the same reply, or the same refusal kind
/// (a parse refusal carries the parser's own reason, which the rule does not fix).
fn agrees(got: &Result<Reply, ReplyFault>, want: &Result<Reply, ReplyFault>) -> bool {
    match *want {
        Ok(ref reply) => got.as_ref().ok() == Some(reply),
        Err(ReplyFault::NotJson(_)) => {
            matches!(*got, Err(ReplyFault::NotJson(ref reason)) if !reason.is_empty())
        }
        Err(ref fault) => got
            .as_ref()
            .err()
            .is_some_and(|refused| discriminant(refused) == discriminant(fault)),
    }
}

/// Every generated reply gets exactly the rule's answer, for every seed.
#[test]
fn sim_reply_parser_matches_the_reference_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let (reply, expected) = case(seed)?;
        let got = parse_reply(&reply);
        assert!(
            agrees(&got, &expected),
            "seed {seed}: {reply:?} gave {got:?}, the rule says {expected:?}"
        );
    }
    Ok(())
}

/// The whole reply is bounded before it is parsed, and an empty object or an
/// unknown sole key is refused.
#[test]
fn an_oversized_or_keyless_reply_is_refused_before_anything_runs() {
    let long = format!("{{\"done\":\"{}\"}}", "z".repeat(16_400));
    assert_eq!(
        parse_reply(&long),
        Err(ReplyFault::TooLong),
        "the reply bound"
    );
    assert_eq!(parse_reply("{}"), Err(ReplyFault::Keys), "no key");
    assert_eq!(
        parse_reply("{\"done\":\"\"}"),
        Ok(Reply::Done(String::new())),
        "empty summary"
    );
}

/// The fat-finger hands a session a blank, giant, NUL-carrying or ordinary
/// instruction and a directory, a file or nothing; the refusal is the first rule
/// broken in the documented order (blank, size, NUL, directory), checked before
/// anything runs.
#[test]
fn sim_session_fat_finger_instruction_and_directory_get_the_rule_refusal() -> TestResult {
    let dir = std::env::temp_dir();
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(2 << 32));
        let len = match rng.below(4)? {
            0 => 0,
            1 => usize::try_from(rng.below(64)?)?,
            2 => MAX_INSTRUCTION_BYTES,
            _ => MAX_INSTRUCTION_BYTES.saturating_add(usize::try_from(rng.below(3)?)?),
        };
        let mut instruction: String = if rng.below(3)? == 0 {
            [' ', '\n', '\t'].iter().cycle().take(len).collect()
        } else {
            "x".repeat(len)
        };
        if rng.below(4)? == 0 && !instruction.is_empty() {
            instruction.replace_range(..1, "\0");
        }
        let missing = dir.join(format!("larry-no-such-dir-{seed}"));
        let cwd = match rng.below(3)? {
            0 => dir.as_path(),
            1 => file.as_path(),
            _ => missing.as_path(),
        };
        let expected = if instruction.trim().is_empty() {
            Err(SessionError::EmptyInstruction)
        } else if instruction.len() > MAX_INSTRUCTION_BYTES {
            Err(SessionError::InstructionTooLarge {
                len: instruction.len(),
            })
        } else if instruction.contains('\0') {
            Err(SessionError::NulInInstruction)
        } else if cwd == dir.as_path() {
            Ok(())
        } else {
            Err(SessionError::NotADirectory)
        };
        assert_eq!(check(&instruction, cwd), expected, "seed {seed}: len {len}");
    }
    Ok(())
}

/// A stream past the observed bound is cut on a character boundary at or under
/// the bound and marked cut; a stream at the bound is kept whole.
#[test]
fn sim_observations_keep_their_bound_on_char_boundaries_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 32));
        let target = OBSERVED_BYTES
            .saturating_sub(8)
            .saturating_add(usize::try_from(rng.below(24)?)?);
        let mut stream = String::new();
        while stream.len() < target {
            stream.push_str(text(&mut rng, 1)?.as_str());
        }
        let original = stream.clone();
        let cut = clip(&mut stream, OBSERVED_BYTES);
        assert_eq!(
            cut,
            original.len() > OBSERVED_BYTES,
            "seed {seed}: cut flag"
        );
        assert!(stream.len() <= OBSERVED_BYTES, "seed {seed}: bound");
        assert!(original.starts_with(&stream), "seed {seed}: a prefix");
        if cut {
            assert!(
                OBSERVED_BYTES.saturating_sub(stream.len()) < 4,
                "seed {seed}: at most one character short of the bound"
            );
        }
    }
    Ok(())
}
