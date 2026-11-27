//! The tools Larry offers over MCP.
//!
//! One tool exists: `larry_submit_task`, the typed task Claude hands Larry
//! (goal, repo, files, acceptance commands; larry#71 Track 3). It runs through
//! the executor (larry#123): a fresh clone of the repository at `HEAD` (or the
//! base the task names), the model writes each named file, each acceptance
//! command runs under `sh -c` (or the shell the task names), within the timeout
//! and deadline the task names and the tenant's ceilings admit (larry#439),
//! and the sealed report comes back with its receipt. Only an accepted run is a
//! success; a rejected run, a refusal and an unconfigured executor are typed
//! tool errors, so no success is ever reported for work that did not pass.

use crate::executor::{ExecError, Knobs, MAX_TIMEOUT_MS, Report, Settings, Task, execute};
use crate::model::{RouteError, is_route_id};
use lgwks_macros::Error;
use lgwks_std::json::{Map, Value, json};
use std::path::{Component, Path};

/// Name of the only tool.
const SUBMIT_TASK: &str = "larry_submit_task";
/// Arguments the tool accepts; anything else is refused.
const ARGUMENTS: [&str; 9] = [
    "goal",
    "repo",
    "files",
    "acceptance",
    "route",
    "shell",
    "base",
    "timeout_ms",
    "deadline_ms",
];
/// Longest shell or base in bytes: the task document's bounds.
const MAX_WORD_BYTES: usize = 256;
/// Longest route id in bytes: the route table's bound.
const MAX_ROUTE_BYTES: usize = 64;
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
    /// `LARRY_STATE_DIR` and the model settings were read. Boxed so the
    /// settings (which now carry the policy set too) stay off the enum's size.
    Connected(Box<Settings>),
    /// The settings could not be read; the typed reason names the variable (larry#343).
    Absent(ExecError),
}

impl Executor {
    /// Reads the executor's settings through `read` once, when the server starts.
    pub(super) fn from_env(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Executor {
        match Settings::remote_from_env(read) {
            Ok(settings) => Executor::Connected(Box::new(settings)),
            Err(error) => Executor::Absent(error),
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
        "description": "Hand Larry a typed task: a goal, the repository (a local path or git URL, cloned fresh at HEAD), the files the model may write and the shell acceptance commands that prove it. Larry has its model write each file, runs every command in the clone, and returns the diff, the command output and an ed25519 receipt that `larry verify` checks. An optional shell (default sh), base (default HEAD), per-command timeout_ms and whole-task deadline_ms are each bounded by the tenant's ceilings; past one is task_refused with nothing cloned. Only an accepted run is a success; a rejected run is task_rejected, a refusal task_refused, a missing executor executor_not_connected.",
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
                },
                "route": { "type": "string", "minLength": 1, "maxLength": MAX_ROUTE_BYTES },
                "shell": { "type": "string", "minLength": 1, "maxLength": MAX_WORD_BYTES },
                "base": { "type": "string", "minLength": 1, "maxLength": MAX_WORD_BYTES },
                "timeout_ms": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS },
                "deadline_ms": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS }
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
    let checked = match validate_task(arguments) {
        Ok(checked) => checked,
        Err(error) => return tool_error("invalid_arguments", &error.to_string()),
    };
    let settings = match *executor {
        Executor::Absent(ref reason) => return tool_error(NOT_CONNECTED, &reason.to_string()),
        Executor::Connected(ref settings) => settings,
    };
    let task: Task = match Task::shell_via(
        checked.repo,
        checked.goal,
        &checked.files,
        &checked.acceptance,
        checked.route,
        checked.knobs,
    ) {
        Ok(task) => task.confining_acceptance(),
        Err(error) => return tool_error("invalid_arguments", &error.to_string()),
    };
    let report: Report = match execute(&task, settings) {
        Ok(report) => report,
        Err(error @ (ExecError::StateDirUnset | ExecError::Route(RouteError::NoRoute))) => {
            return tool_error(NOT_CONNECTED, &error.to_string());
        }
        Err(error) => return tool_error("task_refused", &error.to_string()),
    };
    finished(&report)
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
#[derive(Debug, PartialEq, Eq, Error)]
enum TaskError {
    /// An argument outside the schema.
    #[error(
        "unknown argument; allowed: goal, repo, files, acceptance, route, shell, base, timeout_ms, deadline_ms"
    )]
    UnknownArgument,
    /// A required argument is absent.
    #[error("`{0}` is required")]
    Missing(&'static str),
    /// An argument has the wrong JSON type; the refusal names the form this
    /// door reads, so a client that sent the other door's shape learns it.
    #[error("`{key}` must be {form}")]
    WrongType {
        /// The argument.
        key: &'static str,
        /// The form the argument must take.
        form: &'static str,
    },
    /// A string is empty or blank.
    #[error("`{0}` must not be empty")]
    Empty(&'static str),
    /// A string exceeds its byte bound.
    #[error("`{0}` exceeds {1} bytes")]
    TooLong(&'static str, usize),
    /// A string holds a NUL byte.
    #[error("`{0}` contains a NUL byte")]
    Nul(&'static str),
    /// A list holds too few or too many entries.
    #[error("`{0}` must hold {1} to {2} entries")]
    Count(&'static str, usize, usize),
    /// A file path escapes the repository.
    #[error("files must be repo-relative paths without `..`")]
    NotRepoRelative,
    /// The route is not a route id.
    #[error(
        "`route` must be a route id: 1 to {} bytes of a-z, 0-9, _ and -",
        MAX_ROUTE_BYTES
    )]
    BadRoute,
}

/// A task that passed the schema, every string borrowed from the arguments.
struct Checked<'args> {
    /// The goal.
    goal: &'args str,
    /// The repository reference.
    repo: &'args str,
    /// The files the model writes.
    files: Vec<&'args str>,
    /// The shell acceptance commands.
    acceptance: Vec<&'args str>,
    /// The route the model step uses, when the task names one.
    route: Option<&'args str>,
    /// The shell, base, timeout and deadline the task names (larry#439).
    knobs: Knobs<'args>,
}

/// Checks the typed task against the schema `list` advertises.
fn validate_task(arguments: &Map) -> Result<Checked<'_>, TaskError> {
    if arguments.keys().any(|key| !ARGUMENTS.contains(&key)) {
        return Err(TaskError::UnknownArgument);
    }
    let goal = text(arguments, "goal", MAX_GOAL_BYTES)?;
    let repo = text(arguments, "repo", MAX_REPO_BYTES)?;
    let files = text_list(
        arguments,
        "files",
        1,
        MAX_FILES,
        MAX_PATH_BYTES,
        "an array of repo-relative path strings",
    )?;
    if !files.iter().all(|path| is_repo_relative(path)) {
        return Err(TaskError::NotRepoRelative);
    }
    let acceptance = text_list(
        arguments,
        "acceptance",
        1,
        MAX_ACCEPTANCE,
        MAX_COMMAND_BYTES,
        "an array of shell command strings, one per command, such as [\"cargo nextest run --locked\"]",
    )?;
    let route = match arguments.get("route") {
        None => None,
        Some(value) => {
            let named = value.as_str().ok_or(TaskError::WrongType {
                key: "route",
                form: "a string naming a route id",
            })?;
            if !is_route_id(named) {
                return Err(TaskError::BadRoute);
            }
            Some(named)
        }
    };
    let knobs = Knobs {
        shell: optional_text(arguments, "shell")?,
        base: optional_text(arguments, "base")?,
        timeout_ms: optional_millis(arguments, "timeout_ms")?,
        deadline_ms: optional_millis(arguments, "deadline_ms")?,
    };
    Ok(Checked {
        goal,
        repo,
        files,
        acceptance,
        route,
        knobs,
    })
}

/// An optional `text`-valid string of at most [`MAX_WORD_BYTES`]; the task parser then holds
/// it to the shell or base rule.
fn optional_text<'a>(arguments: &'a Map, key: &'static str) -> Result<Option<&'a str>, TaskError> {
    match arguments.get(key) {
        None => Ok(None),
        Some(_) => text(arguments, key, MAX_WORD_BYTES).map(Some),
    }
}

/// An optional whole number of milliseconds in `1..=86_400_000`, the executor's own
/// bound on any wait: [`MAX_TIMEOUT_MS`].
fn optional_millis(arguments: &Map, key: &'static str) -> Result<Option<u64>, TaskError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    match value.as_u64() {
        Some(millis) if (1..=MAX_TIMEOUT_MS).contains(&millis) => Ok(Some(millis)),
        _ => Err(TaskError::WrongType {
            key,
            form: "a whole number of milliseconds in 1..=86400000",
        }),
    }
}

/// A required, non-empty string without NUL, at most `max` bytes.
fn text<'a>(arguments: &'a Map, key: &'static str, max: usize) -> Result<&'a str, TaskError> {
    let Some(value) = arguments.get(key) else {
        return Err(TaskError::Missing(key));
    };
    let Some(string) = value.as_str() else {
        return Err(TaskError::WrongType {
            key,
            form: "a string",
        });
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

/// A required array of `text`-valid strings holding `min..=max_items` entries,
/// each at most `max_len` bytes; `form` names the shape its refusal asks for.
fn text_list<'a>(
    arguments: &'a Map,
    key: &'static str,
    min: usize,
    max_items: usize,
    max_len: usize,
    form: &'static str,
) -> Result<Vec<&'a str>, TaskError> {
    let Some(value) = arguments.get(key) else {
        return Err(TaskError::Missing(key));
    };
    let Some(items) = value.as_array() else {
        return Err(TaskError::WrongType { key, form });
    };
    if items.len() < min || items.len() > max_items {
        return Err(TaskError::Count(key, min, max_items));
    }
    let mut checked = Vec::with_capacity(items.len());
    for item in items {
        let Some(entry) = item.as_str() else {
            return Err(TaskError::WrongType { key, form });
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
    use crate::executor::ExecError;
    use crate::json_at::At;
    use lgwks_std::json::{Value, json};

    /// A process with no executor behind it.
    fn absent() -> Executor {
        Executor::Absent(ExecError::StateDirUnset)
    }

    /// A non-UTF-8 variable Larry reads is a typed refusal that names the variable in the
    /// tool result, so an operator knows what to fix (larry#343).
    #[test]
    fn a_non_utf8_variable_names_itself_in_the_tool_result()
    -> Result<(), Box<dyn std::error::Error>> {
        let executor = Executor::Absent(ExecError::NotUnicode {
            name: String::from("LARRY_MODEL_KEY"),
        });
        let task = json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": ["c"]});
        let arguments = task.as_object().ok_or("the task is an object")?;
        let reply = call("larry_submit_task", arguments, &executor).ok_or("the tool is listed")?;
        assert_eq!(
            reply.get("isError").and_then(Value::as_bool),
            Some(true),
            "a refusal is never success"
        );
        let message = reply
            .get("structuredContent")
            .and_then(|content| content.get("error"))
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .ok_or("the refusal carries its message")?;
        assert!(
            message.contains("LARRY_MODEL_KEY") && message.contains("not UTF-8"),
            "the refusal names the variable: {message}"
        );
        Ok(())
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
            // larry#439: a timeout or deadline outside one day, or a shell or base not text.
            json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": ["c"], "timeout_ms": 0}),
            json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": ["c"], "deadline_ms": 86_400_001}),
            json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": ["c"], "shell": 7}),
            json!({"goal": "g", "repo": "r", "files": ["f"], "acceptance": ["c"], "base": ""}),
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
            list()
                .member("/tools")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1),
            "one tool declared"
        );
    }

    /// An empty argument map.
    fn serde_empty() -> lgwks_std::json::Map {
        lgwks_std::json::Map::new()
    }
}
