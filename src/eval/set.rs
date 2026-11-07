//! The matched task set `larry eval` reads (larry#87): a named list of
//! self-contained tasks, each with the files its origin repository starts from,
//! the goal, the files the model writes and the shell acceptance commands. The
//! whole set is checked before the first task runs, so a malformed set spends
//! nothing, and each task's goal, files and commands are checked by the one
//! executor parser (`Task::shell`), never by a second copy of its rules.

use std::fmt;

use lgwks_std::json::{self, Map, Value};

use crate::executor::{Task, TaskError, is_writable_path};

/// Largest task set, in bytes.
pub const MAX_SET_BYTES: usize = 4_194_304;
/// Most tasks one set may hold.
const MAX_TASKS: usize = 256;
/// Longest set name or task id, in bytes.
const MAX_NAME_BYTES: usize = 64;
/// Most files one task's origin may start from.
const MAX_SEED_FILES: usize = 64;
/// Largest content of one origin file, in bytes.
const MAX_SEED_BYTES: usize = 262_144;
/// Most acceptance commands one task may carry: the executor's command bound.
const MAX_ACCEPTANCE: usize = 16;
/// Every key a set may carry.
const SET_KEYS: [&str; 2] = ["name", "tasks"];
/// Every key a task may carry.
const TASK_KEYS: [&str; 5] = ["id", "files", "goal", "write", "acceptance"];

/// One task of a set, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvalTask {
    /// The task's name in the set, distinct.
    pub(crate) id: String,
    /// The origin's files: repo-relative path and content, ascending by path.
    pub(crate) files: Vec<(String, String)>,
    /// The goal the model is given.
    pub(crate) goal: String,
    /// The files the model writes.
    pub(crate) write: Vec<String>,
    /// The shell acceptance commands, each run as `sh -c`.
    pub(crate) acceptance: Vec<String>,
}

impl EvalTask {
    /// The executor task for this one, run against the origin at `repo`.
    pub(crate) fn task(&self, repo: &str) -> Result<Task, TaskError> {
        let write: Vec<&str> = self.write.iter().map(String::as_str).collect();
        let acceptance: Vec<&str> = self.acceptance.iter().map(String::as_str).collect();
        Task::shell(repo, &self.goal, &write, &acceptance)
    }
}

/// A checked task set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskSet {
    /// The set's name, printed on every scorecard so two arms name one set.
    pub(crate) name: String,
    /// The tasks, in the order they run.
    pub(crate) tasks: Vec<EvalTask>,
}

/// Why a task set was refused; nothing ran.
#[derive(Debug)]
#[non_exhaustive]
pub enum SetError {
    /// The set exceeds 4 MiB.
    TooLarge {
        /// Its length in bytes.
        len: usize,
    },
    /// The set is not UTF-8 JSON.
    NotJson(json::Error),
    /// The set, or task `index`, is not a JSON object.
    NotAnObject {
        /// The task, or `None` for the set itself.
        index: Option<usize>,
    },
    /// The set, or task `index`, carries a key outside the format.
    UnknownKey {
        /// The task, or `None` for the set itself.
        index: Option<usize>,
        /// The key.
        key: String,
    },
    /// A key is absent or holds the wrong JSON type.
    WrongType {
        /// The task, or `None` for the set itself.
        index: Option<usize>,
        /// The key.
        key: &'static str,
    },
    /// The set name is empty, too long or outside `[a-z0-9._-]`.
    BadName,
    /// The set holds no task, or more than 256.
    TaskCount {
        /// The number of tasks.
        count: usize,
    },
    /// Task `index` has an id that is empty, too long or outside `[a-z0-9._-]`.
    BadId {
        /// The task.
        index: usize,
    },
    /// Task `index` repeats an earlier id.
    DuplicateId {
        /// The task.
        index: usize,
    },
    /// Task `index` starts from no file, or more than 64.
    FileCount {
        /// The task.
        index: usize,
        /// The number of files.
        count: usize,
    },
    /// Task `index` names an origin file outside the repository or inside `.git`.
    BadFile {
        /// The task.
        index: usize,
        /// The path.
        path: String,
    },
    /// Task `index` has an origin file over 256 KiB.
    FileTooLarge {
        /// The task.
        index: usize,
        /// The path.
        path: String,
    },
    /// Task `index` carries no acceptance command, or more than 16: a task no
    /// command checks would count as accepted for nothing.
    AcceptanceCount {
        /// The task.
        index: usize,
        /// The number of commands.
        count: usize,
    },
    /// Task `index` is refused by the executor's own task parser.
    Task {
        /// The task.
        index: usize,
        /// The parser's reason.
        cause: TaskError,
    },
}

impl fmt::Display for SetError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let place = |index: Option<usize>| match index {
            Some(task) => format!("task {task}"),
            None => String::from("the set"),
        };
        match *self {
            Self::TooLarge { len } => write!(out, "the task set is {len} bytes, over 4 MiB"),
            Self::NotJson(ref cause) => write!(out, "the task set is not JSON: {cause}"),
            Self::NotAnObject { index } => write!(out, "{} is not a JSON object", place(index)),
            Self::UnknownKey { index, ref key } => {
                write!(out, "{} has unknown key {key}", place(index))
            }
            Self::WrongType { index, key } => {
                write!(
                    out,
                    "{} key {key} is missing or has the wrong type",
                    place(index)
                )
            }
            Self::BadName => out.write_str("the set name must be 1 to 64 of [a-z0-9._-]"),
            Self::TaskCount { count } => write!(out, "the set holds {count} tasks; 1 to 256"),
            Self::BadId { index } => write!(out, "task {index} id must be 1 to 64 of [a-z0-9._-]"),
            Self::DuplicateId { index } => write!(out, "task {index} repeats an earlier id"),
            Self::FileCount { index, count } => {
                write!(out, "task {index} starts from {count} files; 1 to 64")
            }
            Self::BadFile { index, ref path } => write!(
                out,
                "task {index} file {path:?} is not a repo-relative path outside .git"
            ),
            Self::FileTooLarge { index, ref path } => {
                write!(out, "task {index} file {path:?} is over 256 KiB")
            }
            Self::AcceptanceCount { index, count } => write!(
                out,
                "task {index} carries {count} acceptance commands; 1 to 16"
            ),
            Self::Task { index, ref cause } => write!(out, "task {index}: {cause}"),
        }
    }
}

impl std::error::Error for SetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::NotJson(ref cause) => Some(cause),
            Self::Task { ref cause, .. } => Some(cause),
            Self::TooLarge { .. }
            | Self::NotAnObject { .. }
            | Self::UnknownKey { .. }
            | Self::WrongType { .. }
            | Self::BadName
            | Self::TaskCount { .. }
            | Self::BadId { .. }
            | Self::DuplicateId { .. }
            | Self::FileCount { .. }
            | Self::BadFile { .. }
            | Self::FileTooLarge { .. }
            | Self::AcceptanceCount { .. } => None,
        }
    }
}

/// A set name or task id: 1 to 64 bytes of `[a-z0-9._-]`.
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME_BYTES
        && text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

/// Refuses any key of `object` outside `allowed`.
fn known_keys(object: &Map, allowed: &[&str], index: Option<usize>) -> Result<(), SetError> {
    match object.keys().find(|key| !allowed.contains(key)) {
        Some(key) => Err(SetError::UnknownKey {
            index,
            key: key.to_owned(),
        }),
        None => Ok(()),
    }
}

/// The strings of array `key` of task `index`.
fn strings(object: &Map, key: &'static str, index: usize) -> Result<Vec<String>, SetError> {
    let wrong = SetError::WrongType {
        index: Some(index),
        key,
    };
    let Some(items) = object.get(key).and_then(Value::as_array) else {
        return Err(wrong);
    };
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str() {
            Some(text) => texts.push(text.to_owned()),
            None => {
                return Err(SetError::WrongType {
                    index: Some(index),
                    key,
                });
            }
        }
    }
    Ok(texts)
}

/// The origin files of task `index`: an object of path to content.
fn seed_files(object: &Map, index: usize) -> Result<Vec<(String, String)>, SetError> {
    let Some(files) = object.get("files").and_then(Value::as_object) else {
        return Err(SetError::WrongType {
            index: Some(index),
            key: "files",
        });
    };
    if files.is_empty() || files.len() > MAX_SEED_FILES {
        return Err(SetError::FileCount {
            index,
            count: files.len(),
        });
    }
    let mut seeded = Vec::with_capacity(files.len());
    for (path, content) in files {
        if !is_writable_path(path) {
            return Err(SetError::BadFile {
                index,
                path: path.to_owned(),
            });
        }
        let Some(text) = content.as_str() else {
            return Err(SetError::WrongType {
                index: Some(index),
                key: "files",
            });
        };
        if text.len() > MAX_SEED_BYTES {
            return Err(SetError::FileTooLarge {
                index,
                path: path.to_owned(),
            });
        }
        seeded.push((path.to_owned(), text.to_owned()));
    }
    Ok(seeded)
}

/// Task `index` of the set, checked whole.
fn eval_task(value: &Value, index: usize) -> Result<EvalTask, SetError> {
    let object = value
        .as_object()
        .ok_or(SetError::NotAnObject { index: Some(index) })?;
    known_keys(object, &TASK_KEYS, Some(index))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or(SetError::WrongType {
            index: Some(index),
            key: "id",
        })?;
    if !is_name(id) {
        return Err(SetError::BadId { index });
    }
    let goal = object
        .get("goal")
        .and_then(Value::as_str)
        .ok_or(SetError::WrongType {
            index: Some(index),
            key: "goal",
        })?;
    let files = seed_files(object, index)?;
    let write = strings(object, "write", index)?;
    let acceptance = strings(object, "acceptance", index)?;
    if acceptance.is_empty() || acceptance.len() > MAX_ACCEPTANCE {
        return Err(SetError::AcceptanceCount {
            index,
            count: acceptance.len(),
        });
    }
    let task = EvalTask {
        id: id.to_owned(),
        files,
        goal: goal.to_owned(),
        write,
        acceptance,
    };
    task.task(".")
        .map_err(|cause| SetError::Task { index, cause })?;
    Ok(task)
}

impl TaskSet {
    /// Parses and checks a whole task set from its bytes.
    pub(crate) fn parse(bytes: &[u8]) -> Result<TaskSet, SetError> {
        if bytes.len() > MAX_SET_BYTES {
            return Err(SetError::TooLarge { len: bytes.len() });
        }
        let value: Value = json::from_slice(bytes).map_err(SetError::NotJson)?;
        let object = value
            .as_object()
            .ok_or(SetError::NotAnObject { index: None })?;
        known_keys(object, &SET_KEYS, None)?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or(SetError::WrongType {
                index: None,
                key: "name",
            })?;
        if !is_name(name) {
            return Err(SetError::BadName);
        }
        let entries = object
            .get("tasks")
            .and_then(Value::as_array)
            .ok_or(SetError::WrongType {
                index: None,
                key: "tasks",
            })?;
        if entries.is_empty() || entries.len() > MAX_TASKS {
            return Err(SetError::TaskCount {
                count: entries.len(),
            });
        }
        let mut tasks: Vec<EvalTask> = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let task = eval_task(entry, index)?;
            if tasks.iter().any(|seen| seen.id == task.id) {
                return Err(SetError::DuplicateId { index });
            }
            tasks.push(task);
        }
        Ok(TaskSet {
            name: name.to_owned(),
            tasks,
        })
    }
}
