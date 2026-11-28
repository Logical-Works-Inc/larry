//! A terminal session's reply protocol: one JSON object per turn, parsed and
//! refused with an exact reason the model can retry from. This is the wire
//! half of a session — `super` runs the turn and seals the report, while this
//! module decides what a reply says (larry#93).

use std::fmt;

use lgwks_std::json::{self, Value};

use super::super::author::unfenced;
use super::super::edit::Edit;
use super::super::patch::Patch;

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
    /// Apply one anchored edit to a file of the working directory.
    Edit(Edit),
    /// Apply a patch to files of the working directory, all or none.
    Patch(Patch),
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
    /// The object does not hold exactly one key, `command`, `done`, `read`,
    /// `edit` or `patch`.
    Keys,
    /// The key's value, or an edit's or a patch's member, is not a string.
    NotAString,
    /// The command is empty or blank.
    EmptyCommand,
    /// The command, summary, path or edit text is over 4 KiB.
    FieldTooLong,
    /// The command, path, edit or patch text holds a NUL byte, which no shell
    /// argument, file name or text file can carry.
    Nul,
    /// The edit is not an object of `path`, `old`, `new` and `expect_digest`.
    EditShape,
    /// The patch is not an object of `text` and the `expect_digests` object.
    PatchShape,
}

impl fmt::Display for ReplyFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooLong => write!(out, "the reply is over {} bytes", super::MAX_REPLY_BYTES),
            Self::NotJson(ref cause) => write!(out, "the reply is not JSON ({cause})"),
            Self::NotAnObject => out.write_str("the reply is not a JSON object"),
            Self::Keys => out.write_str(
                "the object must hold exactly one key, command, done, read, edit or patch",
            ),
            Self::NotAString => out.write_str("the value must be a string"),
            Self::EmptyCommand => out.write_str("the command is empty"),
            Self::FieldTooLong => write!(
                out,
                "the command, summary, path or edit text is over {} bytes",
                super::MAX_COMMAND_BYTES
            ),
            Self::Nul => out.write_str("the command, path, edit or patch text holds a NUL byte"),
            Self::EditShape => {
                out.write_str("the edit must hold exactly path, old, new and expect_digest")
            }
            Self::PatchShape => {
                out.write_str("the patch must hold exactly text and the expect_digests object")
            }
        }
    }
}

/// A string no longer than one field may be.
fn bounded(field: &Value) -> Result<&str, ReplyFault> {
    let text = field.as_str().ok_or(ReplyFault::NotAString)?;
    if text.len() > super::MAX_COMMAND_BYTES {
        return Err(ReplyFault::FieldTooLong);
    }
    Ok(text)
}

/// Parses an `edit` value: exactly `path`, `old`, `new` and `expect_digest`,
/// each a bounded NUL-free string.
fn parse_edit(field: &Value) -> Result<Reply, ReplyFault> {
    let object = field
        .as_object()
        .filter(|object| object.len() == 4)
        .ok_or(ReplyFault::EditShape)?;
    let member = |key: &str| {
        let text = bounded(object.get(key).ok_or(ReplyFault::EditShape)?)?;
        if text.contains('\0') {
            return Err(ReplyFault::Nul);
        }
        Ok(text.to_owned())
    };
    Ok(Reply::Edit(Edit {
        path: member("path")?,
        old: member("old")?,
        new: member("new")?,
        expect_digest: member("expect_digest")?,
    }))
}

/// Parses a `patch` value: `text`, NUL-free and bounded by the reply alone, and
/// `expect_digests`, each changed file's path with its digest.
fn parse_patch(field: &Value) -> Result<Reply, ReplyFault> {
    let object = field
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or(ReplyFault::PatchShape)?;
    let text = object.get("text").ok_or(ReplyFault::PatchShape)?;
    let text = text.as_str().ok_or(ReplyFault::NotAString)?;
    if text.contains('\0') {
        return Err(ReplyFault::Nul);
    }
    let named = object
        .get("expect_digests")
        .and_then(Value::as_object)
        .ok_or(ReplyFault::PatchShape)?;
    let expect_digests = named
        .iter()
        .map(|(path, digest)| {
            let path: &str = path;
            let digest = digest.as_str().ok_or(ReplyFault::NotAString)?;
            Ok((path.to_owned(), digest.to_owned()))
        })
        .collect::<Result<_, ReplyFault>>()?;
    Ok(Reply::Patch(Patch {
        text: text.to_owned(),
        expect_digests,
    }))
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
    let name: &str = key;
    if name == "edit" {
        return parse_edit(field);
    }
    if name == "patch" {
        return parse_patch(field);
    }
    let text = bounded(field)?;
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
    use super::{Edit, Patch, Reply, ReplyFault, parse_reply};

    /// A `patch` reply carries its text and digests, past the field bound; a
    /// missing, extra, mistyped or NUL-carrying one is refused with the reason
    /// the model retries from.
    #[test]
    fn a_patch_reply_parses_and_a_malformed_one_is_refused() {
        let reply = |body: &str| parse_reply(&format!("{{\"patch\":{body}}}"));
        let long = "x".repeat(MAX_COMMAND_BYTES.saturating_add(1));
        for text in ["*** Begin Patch", long.as_str()] {
            assert_eq!(
                reply(&format!(
                    r#"{{"text":"{text}","expect_digests":{{"a.rs":"d"}}}}"#
                )),
                Ok(Reply::Patch(Patch {
                    text: text.to_owned(),
                    expect_digests: [(String::from("a.rs"), String::from("d"))].into(),
                })),
                "a well-formed patch parses"
            );
        }
        for shape in [
            r#""x""#,
            r#"{"text":"t"}"#,
            r#"{"text":"t","expect_digests":{},"more":"z"}"#,
            r#"{"text":"t","digests":{}}"#,
            r#"{"more":"t","expect_digests":{}}"#,
            r#"{"text":"t","expect_digests":[]}"#,
        ] {
            assert_eq!(reply(shape), Err(ReplyFault::PatchShape), "{shape}");
        }
        for mistyped in [
            r#"{"text":7,"expect_digests":{}}"#,
            r#"{"text":"t","expect_digests":{"a.rs":7}}"#,
        ] {
            assert_eq!(reply(mistyped), Err(ReplyFault::NotAString), "{mistyped}");
        }
        assert_eq!(
            reply(r#"{"text":"a\u0000","expect_digests":{}}"#),
            Err(ReplyFault::Nul),
            "a NUL in the text"
        );
    }

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

    /// An `edit` reply carries its four fields; a missing, extra, mistyped,
    /// over-long or NUL-carrying one is refused with the reason the model
    /// retries from.
    #[test]
    fn an_edit_reply_parses_and_a_malformed_one_is_refused() {
        let reply = |body: &str| parse_reply(&format!("{{\"edit\":{body}}}"));
        assert_eq!(
            reply(r#"{"path":"a.rs","old":"x","new":"y","expect_digest":"d"}"#),
            Ok(Reply::Edit(Edit {
                path: String::from("a.rs"),
                old: String::from("x"),
                new: String::from("y"),
                expect_digest: String::from("d"),
            })),
            "a well-formed edit parses"
        );
        for shape in [
            r#""x""#,
            r#"{"path":"a.rs","old":"x","new":"y"}"#,
            r#"{"path":"a.rs","old":"x","new":"y","expect_digest":"d","more":"z"}"#,
            r#"{"path":"a.rs","old":"x","new":"y","digest":"d"}"#,
        ] {
            assert_eq!(reply(shape), Err(ReplyFault::EditShape), "{shape}");
        }
        assert_eq!(
            reply(r#"{"path":"a.rs","old":7,"new":"y","expect_digest":"d"}"#),
            Err(ReplyFault::NotAString),
            "a mistyped member"
        );
        assert_eq!(
            reply(r#"{"path":"a.rs","old":"x","new":"a\u0000","expect_digest":"d"}"#),
            Err(ReplyFault::Nul),
            "a NUL in a member"
        );
        let long = "x".repeat(MAX_COMMAND_BYTES.saturating_add(1));
        assert_eq!(
            reply(&format!(
                r#"{{"path":"a.rs","old":"{long}","new":"y","expect_digest":"d"}}"#
            )),
            Err(ReplyFault::FieldTooLong),
            "a member past the bound"
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
