//! The tools Larry offers over MCP.
//!
//! One tool exists: `larry_submit_task`, the typed task Claude hands Larry
//! (goal, repo, files, acceptance commands; larry#71 Track 3). The executor
//! that would run it (larry#85, larry#83) is not built, so a well-formed task
//! is refused with a typed tool error and no success is ever reported:
//! orchestration is never simulated (INVARIANTS, Product).

use lgwks_std::json::{Map, Value, json};
use std::path::{Component, Path};

/// Name of the only tool.
const SUBMIT_TASK: &str = "larry_submit_task";
/// Arguments the tool accepts; anything else is refused.
const ARGUMENTS: [&str; 4] = ["goal", "repo", "files", "acceptance"];
/// Longest goal in bytes.
const MAX_GOAL_BYTES: usize = 8192;
/// Longest repository reference in bytes.
const MAX_REPO_BYTES: usize = 512;
/// Most files one task may name.
const MAX_FILES: usize = 256;
/// Longest file path in bytes.
const MAX_PATH_BYTES: usize = 1024;
/// Most acceptance commands one task may carry.
const MAX_ACCEPTANCE: usize = 32;
/// Longest acceptance command in bytes.
const MAX_COMMAND_BYTES: usize = 2048;

/// The `tools/list` result: every tool in declaration order, no pagination.
pub(super) fn list() -> Value {
    json!({
        "tools": [{
            "name": SUBMIT_TASK,
            "title": "Submit a task to Larry",
            "description": "Hand Larry a typed task: a goal, the repository, the files it may touch and the acceptance commands that prove it. Larry returns a diff, command output and a receipt once an executor is connected; until then the call is refused with executor_not_connected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal": { "type": "string", "minLength": 1, "maxLength": MAX_GOAL_BYTES },
                    "repo": { "type": "string", "minLength": 1, "maxLength": MAX_REPO_BYTES },
                    "files": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1, "maxLength": MAX_PATH_BYTES },
                        "maxItems": MAX_FILES
                    },
                    "acceptance": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1, "maxLength": MAX_COMMAND_BYTES },
                        "minItems": 1,
                        "maxItems": MAX_ACCEPTANCE
                    }
                },
                "required": ["goal", "repo", "acceptance"],
                "additionalProperties": false
            }
        }]
    })
}

/// Runs tool `name`; `None` means no such tool (a protocol error, not a tool error).
pub(super) fn call(name: &str, arguments: &Map<String, Value>) -> Option<Value> {
    match name {
        SUBMIT_TASK => Some(submit_task(arguments)),
        _ => None,
    }
}

/// Validates the task, then refuses it because no executor is connected.
fn submit_task(arguments: &Map<String, Value>) -> Value {
    match validate_task(arguments) {
        Err(error) => tool_error("invalid_arguments", &error.to_string()),
        Ok(()) => tool_error(
            "executor_not_connected",
            "the task is well formed but no executor is connected (larry#85, larry#83); nothing ran and no receipt exists",
        ),
    }
}

/// A tool execution error: `isError` true with the typed code in structured content.
fn tool_error(code: &str, message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": format!("{code}: {message}") }],
        "structuredContent": { "error": { "code": code, "message": message } },
        "isError": true
    })
}

/// Why a task failed validation.
#[derive(Debug, PartialEq, Eq)]
enum TaskError {
    /// An argument outside the schema.
    UnknownArgument,
    /// A required argument is absent.
    Missing(&'static str),
    /// An argument has the wrong JSON type.
    WrongType(&'static str),
    /// A string is empty or blank.
    Empty(&'static str),
    /// A string exceeds its byte bound.
    TooLong(&'static str, usize),
    /// A string holds a NUL byte.
    Nul(&'static str),
    /// A list holds too few or too many entries.
    Count(&'static str, usize, usize),
    /// A file path escapes the repository.
    NotRepoRelative,
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::UnknownArgument => {
                write!(
                    out,
                    "unknown argument; allowed: goal, repo, files, acceptance"
                )
            }
            Self::Missing(key) => write!(out, "`{key}` is required"),
            Self::WrongType(key) => write!(out, "`{key}` has the wrong type"),
            Self::Empty(key) => write!(out, "`{key}` must not be empty"),
            Self::TooLong(key, max) => write!(out, "`{key}` exceeds {max} bytes"),
            Self::Nul(key) => write!(out, "`{key}` contains a NUL byte"),
            Self::Count(key, min, max) => {
                write!(out, "`{key}` must hold {min} to {max} entries")
            }
            Self::NotRepoRelative => {
                write!(out, "files must be repo-relative paths without `..`")
            }
        }
    }
}

/// Checks the typed task against the schema `list` advertises.
fn validate_task(arguments: &Map<String, Value>) -> Result<(), TaskError> {
    if arguments
        .keys()
        .any(|key| !ARGUMENTS.contains(&key.as_str()))
    {
        return Err(TaskError::UnknownArgument);
    }
    text(arguments, "goal", MAX_GOAL_BYTES)?;
    text(arguments, "repo", MAX_REPO_BYTES)?;
    if arguments.contains_key("files") {
        for path in text_list(arguments, "files", 0, MAX_FILES, MAX_PATH_BYTES)? {
            if !is_repo_relative(path) {
                return Err(TaskError::NotRepoRelative);
            }
        }
    }
    text_list(
        arguments,
        "acceptance",
        1,
        MAX_ACCEPTANCE,
        MAX_COMMAND_BYTES,
    )?;
    Ok(())
}

/// A required, non-empty string without NUL, at most `max` bytes.
fn text<'a>(
    arguments: &'a Map<String, Value>,
    key: &'static str,
    max: usize,
) -> Result<&'a str, TaskError> {
    let Some(value) = arguments.get(key) else {
        return Err(TaskError::Missing(key));
    };
    let Some(string) = value.as_str() else {
        return Err(TaskError::WrongType(key));
    };
    check_text(key, string, max)?;
    Ok(string)
}

/// A string is non-blank, NUL-free and at most `max` bytes.
fn check_text(key: &'static str, string: &str, max: usize) -> Result<(), TaskError> {
    if string.trim().is_empty() {
        return Err(TaskError::Empty(key));
    }
    if string.len() > max {
        return Err(TaskError::TooLong(key, max));
    }
    if string.contains('\0') {
        return Err(TaskError::Nul(key));
    }
    Ok(())
}

/// A required array of `text`-valid strings holding `min..=max_items` entries.
fn text_list<'a>(
    arguments: &'a Map<String, Value>,
    key: &'static str,
    min: usize,
    max_items: usize,
    max_len: usize,
) -> Result<Vec<&'a str>, TaskError> {
    let Some(value) = arguments.get(key) else {
        return Err(TaskError::Missing(key));
    };
    let Some(items) = value.as_array() else {
        return Err(TaskError::WrongType(key));
    };
    if items.len() < min || items.len() > max_items {
        return Err(TaskError::Count(key, min, max_items));
    }
    let mut checked = Vec::with_capacity(items.len());
    for item in items {
        let Some(entry) = item.as_str() else {
            return Err(TaskError::WrongType(key));
        };
        check_text(key, entry, max_len)?;
        checked.push(entry);
    }
    Ok(checked)
}

/// Whether `path` stays inside the repository: relative, no `..`, no root or prefix.
fn is_repo_relative(path: &str) -> bool {
    Path::new(path)
        .components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::{call, list};
    use lgwks_std::json::{Value, json};

    /// Calls the tool with `arguments` and returns the structured error code.
    fn code_of(arguments: &Value) -> Option<String> {
        let map = arguments.as_object()?;
        let reply = call("larry_submit_task", map)?;
        Some(
            reply
                .pointer("/structuredContent/error/code")?
                .as_str()?
                .to_owned(),
        )
    }

    /// A well-formed task is refused as not connected, never accepted.
    #[test]
    fn well_formed_task_is_refused_not_connected() {
        let task = json!({"goal": "fix retry", "repo": "Logical-Works-Inc/larry",
            "files": ["src/retry.rs"], "acceptance": ["cargo nextest run --locked"]});
        assert_eq!(
            code_of(&task).as_deref(),
            Some("executor_not_connected"),
            "typed refusal"
        );
    }

    /// Every malformed shape is a typed invalid_arguments tool error.
    #[test]
    fn malformed_tasks_are_invalid_arguments() {
        let cases = [
            json!({}),
            json!({"goal": "g", "repo": "r"}),
            json!({"goal": "", "repo": "r", "acceptance": ["c"]}),
            json!({"goal": "g", "repo": "r", "acceptance": []}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"], "extra": 1}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"], "files": ["../etc/passwd"]}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"], "files": ["/etc/passwd"]}),
            json!({"goal": 7, "repo": "r", "acceptance": ["c"]}),
            json!({"goal": "g\u{0}", "repo": "r", "acceptance": ["c"]}),
        ];
        for case in &cases {
            assert_eq!(
                code_of(case).as_deref(),
                Some("invalid_arguments"),
                "case {case}"
            );
        }
    }

    /// An unknown tool is not a tool error; the dispatcher turns it into -32602.
    #[test]
    fn unknown_tool_is_none_and_list_declares_one_tool() {
        assert!(call("nope", &serde_empty()).is_none(), "unknown tool");
        assert_eq!(
            list()["tools"].as_array().map(Vec::len),
            Some(1),
            "one tool declared"
        );
    }

    /// An empty argument map.
    fn serde_empty() -> lgwks_std::json::Map<String, Value> {
        lgwks_std::json::Map::new()
    }
}
