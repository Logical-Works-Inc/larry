//! A terminal session's reply protocol: one JSON object per turn, parsed and
//! refused with an exact reason the model can retry from. This is the wire
//! half of a session — `super` runs the turn and seals the report, while this
//! module decides what a reply says (larry#93).

use std::fmt;

use lgwks_std::json::{self, Value};

use super::super::author::unfenced;

/// One reply, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Run this command with `sh -c`.
    Command(String),
    /// The task is done; the summary for the person.
    Done(String),
    /// Show one file of the working directory with its digest.
    Read {
        /// The repo-relative path to read.
        path: String,
    },
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
    /// The object does not hold exactly one key, `command`, `done` or `read`.
    Keys,
    /// The key's value is not a string.
    NotAString,
    /// The command is empty or blank.
    EmptyCommand,
    /// The command, summary or path is over 4 KiB.
    FieldTooLong,
    /// The command or path holds a NUL byte, which no shell argument and no
    /// file name can carry.
    Nul,
}

impl fmt::Display for ReplyFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooLong => write!(out, "the reply is over {} bytes", super::MAX_REPLY_BYTES),
            Self::NotJson(ref cause) => write!(out, "the reply is not JSON ({cause})"),
            Self::NotAnObject => out.write_str("the reply is not a JSON object"),
            Self::Keys => {
                out.write_str("the object must hold exactly one key, command, done or read")
            }
            Self::NotAString => out.write_str("the value must be a string"),
            Self::EmptyCommand => out.write_str("the command is empty"),
            Self::FieldTooLong => write!(
                out,
                "the command, summary or path is over {} bytes",
                super::MAX_COMMAND_BYTES
            ),
            Self::Nul => out.write_str("the command or path holds a NUL byte"),
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
        "command" | "read" if text.contains('\0') => Err(ReplyFault::Nul),
        "command" => Ok(Reply::Command(text.to_owned())),
        "done" => Ok(Reply::Done(text.to_owned())),
        "read" => Ok(Reply::Read {
            path: text.to_owned(),
        }),
        _ => Err(ReplyFault::Keys),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{MAX_COMMAND_BYTES, MAX_REPLY_BYTES};
    use super::{Reply, ReplyFault, parse_reply};

    /// A `command` field of exactly the bound parses; one byte past it is refused.
    #[test]
    fn a_command_field_at_the_bound_parses_and_one_past_is_refused() {
        let field = "x".repeat(MAX_COMMAND_BYTES);
        let reply = format!("{{\"command\":{field:?}}}");
        assert_eq!(
            parse_reply(&reply),
            Ok(Reply::Command(field)),
            "a field of exactly the bound parses"
        );
        let past = "x".repeat(MAX_COMMAND_BYTES.saturating_add(1));
        assert_eq!(
            parse_reply(&format!("{{\"command\":{past:?}}}")),
            Err(ReplyFault::FieldTooLong),
            "one byte past the bound is refused"
        );
    }

    /// A `read` reply carries its path; a mistyped one or a NUL is refused.
    #[test]
    fn a_read_reply_parses_and_a_mistyped_one_is_refused() {
        assert_eq!(
            parse_reply("{\"read\":\"src/lib.rs\"}"),
            Ok(Reply::Read {
                path: String::from("src/lib.rs"),
            }),
            "a read reply parses"
        );
        assert_eq!(
            parse_reply("{\"read\":7}"),
            Err(ReplyFault::NotAString),
            "a mistyped path is refused"
        );
        assert_eq!(
            parse_reply("{\"read\":\"a\\u0000b\"}"),
            Err(ReplyFault::Nul),
            "a path holding a NUL is refused"
        );
    }

    /// A reply of exactly the bound still reaches the parser; one byte past it never does.
    #[test]
    fn a_reply_at_the_bound_still_parses_and_one_past_is_refused() {
        let field = "x".repeat(MAX_REPLY_BYTES.saturating_sub(14));
        let reply = format!("{{\"command\":{field:?}}}");
        assert_eq!(
            reply.len(),
            MAX_REPLY_BYTES,
            "the reply is exactly the bound"
        );
        assert_eq!(
            parse_reply(&reply),
            Err(ReplyFault::FieldTooLong),
            "a reply of exactly the bound still parses"
        );
        assert_eq!(
            parse_reply(&format!("x{reply}")),
            Err(ReplyFault::TooLong),
            "one byte past the bound never parses"
        );
    }
}
