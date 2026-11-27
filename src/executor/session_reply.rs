//! A terminal session's model-reply protocol: one JSON object per turn, parsed,
//! refused with an exact reason the model can retry from, and rendered back as
//! the status and streams the model sees. This is the wire half of a session —
//! `super` runs the turn and seals the report, while this module decides what a
//! reply says and what the model is shown (larry#93).

use std::fmt;

use lgwks_std::json::{self, Value};

use super::super::author::unfenced;
use super::super::run::CommandResult;

/// One reply, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Run this command with `sh -c`.
    Command(String),
    /// The task is done; the summary for the person.
    Done(String),
}

/// Why a reply was refused; the model is told and may try again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyFault {
    /// The reply is over [`super::MAX_REPLY_BYTES`].
    TooLong,
    /// The reply is not JSON; the parser's reason, so the model sees where it broke.
    NotJson(String),
    /// The reply is JSON but not an object.
    NotAnObject,
    /// The object does not hold exactly one key, `command` or `done`.
    Keys,
    /// The key's value is not a string.
    NotAString,
    /// The command is empty or blank.
    EmptyCommand,
    /// The command or summary is over 4 KiB.
    FieldTooLong,
    /// The command holds a NUL byte, which no shell argument can carry.
    Nul,
}

impl fmt::Display for ReplyFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooLong => write!(out, "the reply is over {} bytes", super::MAX_REPLY_BYTES),
            Self::NotJson(ref cause) => write!(out, "the reply is not JSON ({cause})"),
            Self::NotAnObject => out.write_str("the reply is not a JSON object"),
            Self::Keys => out.write_str("the object must hold exactly one key, command or done"),
            Self::NotAString => out.write_str("the value must be a string"),
            Self::EmptyCommand => out.write_str("the command is empty"),
            Self::FieldTooLong => write!(
                out,
                "the command or summary is over {} bytes",
                super::MAX_COMMAND_BYTES
            ),
            Self::Nul => out.write_str("the command holds a NUL byte"),
        }
    }
}

/// Parses one model reply: one surrounding code fence and outer whitespace are allowed.
pub(crate) fn parse_reply(text: &str) -> Result<Reply, ReplyFault> {
    if text.len() > super::MAX_REPLY_BYTES {
        return Err(ReplyFault::TooLong);
    }
    let value: Value = json::from_str(unfenced(text).trim())
        .map_err(|cause| ReplyFault::NotJson(cause.to_string()))?;
    let object = value.as_object().ok_or(ReplyFault::NotAnObject)?;
    let mut entries = object.iter();
    let (Some((key, field)), None) = (entries.next(), entries.next()) else {
        return Err(ReplyFault::Keys);
    };
    let text = field.as_str().ok_or(ReplyFault::NotAString)?;
    if text.len() > super::MAX_COMMAND_BYTES {
        return Err(ReplyFault::FieldTooLong);
    }
    let name: &str = key;
    match name {
        "command" if text.trim().is_empty() => Err(ReplyFault::EmptyCommand),
        "command" if text.contains('\0') => Err(ReplyFault::Nul),
        "command" => Ok(Reply::Command(text.to_owned())),
        "done" => Ok(Reply::Done(text.to_owned())),
        _ => Err(ReplyFault::Keys),
    }
}

/// The status line the model is shown for one clipped command result.
pub(super) fn status(result: &CommandResult) -> String {
    let ended = match (result.timed_out(), result.interrupted(), result.exit_code()) {
        (false, false, Some(code)) => format!("exit status {code}"),
        (false, false, None) => String::from("ended by a signal"),
        (true, false, _) => String::from("timed out; its process group was killed"),
        (_, true, _) => String::from("interrupted by the person; its process group was killed"),
    };
    if result.truncated() {
        format!(
            "{ended} (output past {} bytes of a stream was cut)",
            super::OBSERVED_BYTES
        )
    } else {
        ended
    }
}

/// The streams the model is shown for one clipped command result, under its status.
pub(super) fn streams(result: &CommandResult) -> String {
    format!("stdout:\n{}\nstderr:\n{}", result.stdout(), result.stderr())
}

/// What of a model reply enters the conversation: the reply itself, or only its
/// length when it is past the bound, so a huge reply cannot grow the next request.
pub(super) fn kept_reply(reply: &str) -> String {
    if reply.len() > super::MAX_REPLY_BYTES {
        format!(
            "(a reply of {} bytes, over the bound, was dropped)",
            reply.len()
        )
    } else {
        reply.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::mem::discriminant;

    use lgwks_std::seeded::Seeded;

    use super::super::sim::text;
    use super::{Reply, ReplyFault, parse_reply};
    use crate::sim_error::SimError;

    /// What every reply-parser simulation returns.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Seeds the reply-parser sweep.
    const SEEDS: u64 = 2048;

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
    fn sim_session_reply_parser_matches_the_reference_for_every_seed() -> TestResult {
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
    fn session_reply_oversized_or_keyless_is_refused_before_anything_runs() {
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
}
