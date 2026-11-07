//! The tools Larry offers over MCP.
//!
//! One tool exists: `larry_submit_task`, the typed task Claude hands Larry
//! (goal, repo, files, acceptance commands; larry#71 Track 3). It runs through
//! the executor (larry#123): a fresh clone of the repository at `HEAD`, the
//! model writes each named file, each acceptance command runs under `sh -c`,
//! and the sealed report comes back with its receipt. Only an accepted run is a
//! success; a rejected run, a refusal and an unconfigured executor are typed
//! tool errors, so no success is ever reported for work that did not pass.

use larry_demo::executor::{ExecError, Report, Settings, Task, execute};
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
/// Most files one task may name: the executor's brief bound, one completion each.
const MAX_FILES: usize = 64;
/// Longest file path in bytes.
const MAX_PATH_BYTES: usize = 1024;
/// Most acceptance commands one task may carry: the executor's command bound.
const MAX_ACCEPTANCE: usize = 16;
/// Longest acceptance command in bytes.
const MAX_COMMAND_BYTES: usize = 2048;
/// The tool error of a process with no executor behind it.
const NOT_CONNECTED: &str = "executor_not_connected";

/// What `larry_submit_task` runs on: the executor's settings, or why there are none.
#[derive(Debug)]
pub(super) enum Executor {
    /// `LARRY_STATE_DIR` and the model settings were read.
    Connected(Settings),
    /// The settings could not be read; the reason names the variable.
    Absent(String),
}

impl Executor {
    /// Reads the executor's settings through `read` once, when the server starts.
    pub(super) fn from_env(read: impl Fn(&str) -> Option<String>) -> Executor {
        match Settings::from_env(read) {
            Ok(settings) => Executor::Connected(settings),
            Err(error) => Executor::Absent(error.to_string()),
        }
    }
}

/// The `tools/list` result body: every tool ordered by name, no pagination.
///
/// MCP 2026-07-28 (Tools, Capabilities): the order is the same across requests
/// while the set is unchanged, so clients and LLM prompt caches can key on it.
pub(super) fn list() -> Value {
    let mut body = Map::new();
    body.insert(
        String::from("tools"),
        Value::Array(sorted_by_name(declared())),
    );
    Value::Object(body)
}

/// Orders tool definitions by their `name`; the sort is stable and total.
fn sorted_by_name(mut tools: Vec<Value>) -> Vec<Value> {
    tools.sort_by(|left, right| {
        let name_of = |tool: &Value| tool.get("name").and_then(Value::as_str).map(String::from);
        name_of(left).cmp(&name_of(right))
    });
    tools
}

/// Every tool definition, in no promised order. The `inputSchema` carries no
/// `$schema`: MCP 2026-07-28 (Basic, JSON Schema Usage) makes 2020-12 the default dialect.
fn declared() -> Vec<Value> {
    vec![json!({
        "name": SUBMIT_TASK,
        "title": "Submit a task to Larry",
        "description": "Hand Larry a typed task: a goal, the repository (a local path or git URL, cloned fresh at HEAD), the files the model may write and the shell acceptance commands that prove it. Larry has its model write each file, runs every command in the clone, and returns the diff, the command output and an ed25519 receipt that `larry verify` checks. Only an accepted run is a success; a rejected run is task_rejected, a refusal task_refused, a missing executor executor_not_connected.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "goal": { "type": "string", "minLength": 1, "maxLength": MAX_GOAL_BYTES },
                "repo": { "type": "string", "minLength": 1, "maxLength": MAX_REPO_BYTES },
                "files": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1, "maxLength": MAX_PATH_BYTES },
                    "minItems": 1,
                    "maxItems": MAX_FILES
                },
                "acceptance": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1, "maxLength": MAX_COMMAND_BYTES },
                    "minItems": 1,
                    "maxItems": MAX_ACCEPTANCE
                }
            },
            "required": ["goal", "repo", "files", "acceptance"],
            "additionalProperties": false
        }
    })]
}

/// Runs tool `name`; `None` means no such tool (a protocol error, not a tool error).
pub(super) fn call(name: &str, arguments: &Map, executor: &Executor) -> Option<Value> {
    match name {
        SUBMIT_TASK => Some(submit_task(arguments, executor)),
        _ => None,
    }
}

/// Validates the task, runs it through the executor and answers with its sealed report.
fn submit_task(arguments: &Map, executor: &Executor) -> Value {
    if let Err(error) = validate_task(arguments) {
        return tool_error("invalid_arguments", &error.to_string());
    }
    let settings = match *executor {
        Executor::Absent(ref reason) => return tool_error(NOT_CONNECTED, reason),
        Executor::Connected(ref settings) => settings,
    };
    let task: Task = match Task::parse(task_document(arguments).to_string().as_bytes()) {
        Ok(task) => task,
        Err(error) => return tool_error("invalid_arguments", &error.to_string()),
    };
    let report: Report = match execute(&task, settings) {
        Ok(report) => report,
        Err(error @ (ExecError::StateDirUnset | ExecError::ModelUnconfigured)) => {
            return tool_error(NOT_CONNECTED, &error.to_string());
        }
        Err(error) => return tool_error("task_refused", &error.to_string()),
    };
    finished(&report)
}

/// The `larry exec` document for a validated task: the repository at `HEAD`, the brief,
/// and each acceptance command as `sh -c <command>`.
fn task_document(arguments: &Map) -> Value {
    let commands: Vec<Value> = arguments
        .get("acceptance")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|command| json!(["sh", "-c", command]))
        .collect();
    json!({
        "repo": arguments.get("repo"),
        "base": "HEAD",
        "goal": arguments.get("goal"),
        "files": arguments.get("files"),
        "acceptance": commands,
    })
}

/// A sealed run as a tool result: the report and its verdict in structured content and,
/// serialized, in the text block. A run that was not accepted is a tool error.
fn finished(report: &Report) -> Value {
    let accepted = report.accepted();
    let mut structured = Map::new();
    structured.insert(String::from("accepted"), Value::Bool(accepted));
    structured.insert(String::from("report"), report.json().clone());
    if !accepted {
        structured.insert(
            String::from("error"),
            json!({ "code": "task_rejected", "message": "the run was sealed as rejected: a command failed or timed out, or the run was denied" }),
        );
    }
    let structured = Value::Object(structured);
    json!({
        "content": [{ "type": "text", "text": structured.to_string() }],
        "structuredContent": structured,
        "isError": !accepted
    })
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
fn validate_task(arguments: &Map) -> Result<(), TaskError> {
    if arguments.keys().any(|key| !ARGUMENTS.contains(&key)) {
        return Err(TaskError::UnknownArgument);
    }
    text(arguments, "goal", MAX_GOAL_BYTES)?;
    text(arguments, "repo", MAX_REPO_BYTES)?;
    for path in text_list(arguments, "files", 1, MAX_FILES, MAX_PATH_BYTES)? {
        if !is_repo_relative(path) {
            return Err(TaskError::NotRepoRelative);
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
fn text<'a>(arguments: &'a Map, key: &'static str, max: usize) -> Result<&'a str, TaskError> {
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
    arguments: &'a Map,
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
    use super::{Executor, call, list, sorted_by_name};
    use lgwks_std::json::{Value, json};

    /// A process with no executor behind it.
    fn absent() -> Executor {
        Executor::Absent(String::from("exec LARRY_STATE_DIR is unset or empty"))
    }

    /// Source: MCP 2026-07-28 Tools, "Servers SHOULD return tools in a
    /// deterministic order": any input order yields name order, and a repeat
    /// of the sort changes nothing.
    #[test]
    fn tools_are_ordered_by_name_whatever_the_input_order() {
        let tool = |name: &str| json!({ "name": name });
        let names = |tools: &[Value]| -> Vec<String> {
            tools
                .iter()
                .filter_map(|entry| entry["name"].as_str().map(String::from))
                .collect()
        };
        let sorted = sorted_by_name(vec![tool("b"), tool("c"), tool("a")]);
        assert_eq!(names(&sorted), ["a", "b", "c"], "name order");
        let again = sorted_by_name(sorted.clone());
        assert_eq!(names(&again), names(&sorted), "idempotent");
    }

    /// Calls the tool with `arguments` and returns the structured error code.
    fn code_of(arguments: &Value) -> Option<String> {
        let map = arguments.as_object()?;
        let reply = call("larry_submit_task", map, &absent())?;
        Some(
            reply
                .get("structuredContent")?
                .get("error")?
                .get("code")?
                .as_str()?
                .to_owned(),
        )
    }

    /// A well-formed task with no executor behind the process is refused as not
    /// connected, never accepted.
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
            json!({"goal": "g", "repo": "r", "files": ["f"]}),
            json!({"goal": "", "repo": "r", "files": ["f"], "acceptance": ["c"]}),
            json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": []}),
            json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": ["c"], "extra": 1}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"]}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"], "files": []}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"], "files": ["../etc/passwd"]}),
            json!({"goal": "g", "repo": "r", "acceptance": ["c"], "files": ["/etc/passwd"]}),
            json!({"goal": 7, "repo": "r", "files": ["f"], "acceptance": ["c"]}),
            json!({"goal": "g\u{0}", "repo": "r", "files": ["f"], "acceptance": ["c"]}),
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
        assert!(
            call("nope", &serde_empty(), &absent()).is_none(),
            "unknown tool"
        );
        assert_eq!(
            list()["tools"].as_array().map(Vec::len),
            Some(1),
            "one tool declared"
        );
    }

    /// An empty argument map.
    fn serde_empty() -> lgwks_std::json::Map {
        lgwks_std::json::Map::new()
    }
}
