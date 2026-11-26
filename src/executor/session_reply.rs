use std::fmt;

use lgwks_std::json::{self, Value};

use super::author::unfenced;
use super::session_gate::{FinishRequest, MAX_FINISH_PATH_BYTES, MAX_FINISH_PATHS};

/// Largest model reply the session parses, in bytes.
const MAX_REPLY_BYTES: usize = 8192;
/// Largest command or summary one reply may carry, in bytes.
const MAX_COMMAND_BYTES: usize = 4096;

/// One reply, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Run this command with `sh -c`.
    Command(String),
    /// Ask the executor to verify the task before ending it.
    Finish(FinishRequest),
}

/// Why a reply was refused; the model is told and may try again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyFault {
    /// The reply is over [`MAX_REPLY_BYTES`].
    TooLong,
    /// The reply is not JSON; the parser's reason, so the model sees where it broke.
    NotJson(String),
    /// The reply is JSON but not an object.
    NotAnObject,
    /// The object does not hold exactly one key, `command` or `finish`.
    Keys,
    /// The key's value is not a string.
    NotAString,
    /// The command is empty or blank.
    EmptyCommand,
    /// The command or summary is over 4 KiB.
    FieldTooLong,
    /// The command holds a NUL byte, which no shell argument can carry.
    Nul,
    /// The finish tool has the wrong shape or an unbounded member.
    FinishShape,
}

impl fmt::Display for ReplyFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooLong => write!(out, "the reply is over {MAX_REPLY_BYTES} bytes"),
            Self::NotJson(ref cause) => write!(out, "the reply is not JSON ({cause})"),
            Self::NotAnObject => out.write_str("the reply is not a JSON object"),
            Self::Keys => out.write_str("the object must hold exactly one key, command or finish"),
            Self::NotAString => out.write_str("the value must be a string"),
            Self::EmptyCommand => out.write_str("the command is empty"),
            Self::FieldTooLong => write!(
                out,
                "the command or summary is over {MAX_COMMAND_BYTES} bytes"
            ),
            Self::Nul => out.write_str("the command holds a NUL byte"),
            Self::FinishShape => out.write_str("the finish tool has an invalid bounded shape"),
        }
    }
}

/// Parses one model reply: one surrounding code fence and outer whitespace are allowed.
pub(crate) fn parse_reply(text: &str) -> Result<Reply, ReplyFault> {
    if text.len() > MAX_REPLY_BYTES {
        return Err(ReplyFault::TooLong);
    }
    let value: Value = json::from_str(unfenced(text).trim())
        .map_err(|cause| ReplyFault::NotJson(cause.to_string()))?;
    let object = value.as_object().ok_or(ReplyFault::NotAnObject)?;
    let mut entries = object.iter();
    let (Some((key, field)), None) = (entries.next(), entries.next()) else {
        return Err(ReplyFault::Keys);
    };
    let name: &str = key;
    match name {
        "command" => {
            let text = field.as_str().ok_or(ReplyFault::NotAString)?;
            if text.len() > MAX_COMMAND_BYTES {
                return Err(ReplyFault::FieldTooLong);
            }
            if text.trim().is_empty() {
                return Err(ReplyFault::EmptyCommand);
            }
            if text.contains('\0') {
                return Err(ReplyFault::Nul);
            }
            Ok(Reply::Command(text.to_owned()))
        }
        "finish" => parse_finish(field).map(Reply::Finish),
        _ => Err(ReplyFault::Keys),
    }
}

/// Parses the typed finish tool and refuses unknown or unbounded fields.
fn parse_finish(value: &Value) -> Result<FinishRequest, ReplyFault> {
    let object = value.as_object().ok_or(ReplyFault::FinishShape)?;
    let allowed = ["summary", "check", "artifacts", "state", "servers"];
    if object.keys().any(|key| !allowed.contains(&key)) {
        return Err(ReplyFault::FinishShape);
    }
    let text = |name: &str| {
        let value = object.get(name).ok_or(ReplyFault::FinishShape)?;
        let text = value.as_str().ok_or(ReplyFault::FinishShape)?;
        if text.trim().is_empty() || text.len() > MAX_COMMAND_BYTES || text.contains('\0') {
            return Err(ReplyFault::FinishShape);
        }
        Ok(text.to_owned())
    };
    let paths = |name: &str| {
        let values = object
            .get(name)
            .and_then(Value::as_array)
            .ok_or(ReplyFault::FinishShape)?;
        if values.is_empty() || values.len() > MAX_FINISH_PATHS {
            return Err(ReplyFault::FinishShape);
        }
        values
            .iter()
            .map(|value| {
                let text = value.as_str().ok_or(ReplyFault::FinishShape)?;
                if text.is_empty() || text.len() > MAX_FINISH_PATH_BYTES || text.contains('\0') {
                    return Err(ReplyFault::FinishShape);
                }
                Ok(text.to_owned())
            })
            .collect::<Result<Vec<_>, ReplyFault>>()
    };
    Ok(FinishRequest {
        summary: text("summary")?,
        check: text("check")?,
        artifacts: paths("artifacts")?,
        state: paths("state")?,
        servers: paths("servers")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_COMMAND_BYTES, MAX_FINISH_PATH_BYTES, MAX_FINISH_PATHS, MAX_REPLY_BYTES, Reply,
        ReplyFault, parse_reply,
    };

    fn finish_json(summary: &str, artifacts: &str) -> String {
        format!(
            r#"{{"finish":{{"summary":"{summary}","check":"true","artifacts":{artifacts},"state":["clean"],"servers":["none"]}}}}"#
        )
    }

    #[test]
    fn parses_commands_and_typed_finishes() {
        assert_eq!(
            parse_reply(
                r#"```json
{"command":"printf ok"}
```"#,
            ),
            Ok(Reply::Command(String::from("printf ok")))
        );
        let finish = parse_reply(
            r#"{"finish":{"summary":"ready","check":"test -f out","artifacts":["out"],"state":["clean"],"servers":["none"]}}"#,
        );
        assert!(matches!(finish, Ok(Reply::Finish(_))));
    }

    #[test]
    fn refuses_unbounded_or_legacy_replies() {
        assert_eq!(parse_reply(r#"{"done":"ready"}"#), Err(ReplyFault::Keys));
        assert_eq!(
            parse_reply(r#"{"command":" "}"#),
            Err(ReplyFault::EmptyCommand)
        );
        assert_eq!(
            parse_reply(
                r#"{"finish":{"summary":"x","check":"true","artifacts":[],"state":["clean"],"servers":["none"]}}"#
            ),
            Err(ReplyFault::FinishShape)
        );
        assert_eq!(
            parse_reply(
                r#"{"finish":{"summary":"ready","check":"true","artifacts":["out"],"state":["clean"],"servers":["none"],"extra":"unknown"}}"#
            ),
            Err(ReplyFault::FinishShape)
        );
    }

    #[test]
    fn accepts_each_reply_bound_at_its_limit() {
        let command = format!(r#"{{"command":"{}"}}"#, "x".repeat(MAX_COMMAND_BYTES));
        assert!(matches!(parse_reply(&command), Ok(Reply::Command(_))));

        let summary = "x".repeat(MAX_COMMAND_BYTES);
        assert!(matches!(
            parse_reply(&finish_json(&summary, r#"["out"]"#)),
            Ok(Reply::Finish(_))
        ));

        let path = "x".repeat(MAX_FINISH_PATH_BYTES);
        let artifacts = format!(r#"["{path}"]"#);
        assert!(matches!(
            parse_reply(&finish_json("ready", &artifacts)),
            Ok(Reply::Finish(_))
        ));

        let artifacts = format!(
            "[{}]",
            (0..MAX_FINISH_PATHS)
                .map(|_| r#""out""#)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(matches!(
            parse_reply(&finish_json("ready", &artifacts)),
            Ok(Reply::Finish(_))
        ));

        let mut reply = String::from(r#"{"command":"ok"}"#);
        reply.extend(std::iter::repeat_n(' ', MAX_REPLY_BYTES - reply.len()));
        assert_eq!(reply.len(), MAX_REPLY_BYTES);
        assert!(matches!(parse_reply(&reply), Ok(Reply::Command(_))));
    }
}
