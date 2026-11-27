//! The matched task set `larry eval` reads (larry#87): a named list of
//! self-contained tasks, each with the files its origin repository starts from,
//! the goal, the files the model writes and the shell acceptance commands. The
//! whole set is checked before the first task runs, so a malformed set spends
//! nothing, and each task's goal, files and commands are checked by the one
//! executor parser (`Task::shell`), never by a second copy of its rules.

use lgwks_macros::Error;
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
const TASK_KEYS: [&str; 6] = ["id", "files", "goal", "path", "write", "acceptance"];

/// The executor path a matched task drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalPath {
    /// The brief author writes files, then the acceptance commands run.
    Command,
    /// The terminal session model chooses commands and Larry runs them.
    Session,
}

impl EvalPath {
    /// The stable path name recorded on a scorecard result.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Session => "session",
        }
    }
}

/// One task of a set, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvalTask {
    /// The task's name in the set, distinct.
    pub(crate) id: String,
    /// The origin's files: repo-relative path and content, ascending by path.
    pub(crate) files: Vec<(String, String)>,
    /// The goal the model is given.
    pub(crate) goal: String,
    /// The executor path, defaulting to the brief command path.
    pub(crate) path: EvalPath,
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
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SetError {
    /// The set exceeds 4 MiB.
    #[error("the task set is {len} bytes, over 4 MiB")]
    TooLarge {
        /// Its length in bytes.
        len: usize,
    },
    /// The set is not UTF-8 JSON.
    #[error("the task set is not JSON: {0}")]
    NotJson(#[source] json::Error),
    /// The set, or task `index`, is not a JSON object.
    #[error("{} is not a JSON object", place(*index))]
    NotAnObject {
        /// The task, or `None` for the set itself.
        index: Option<usize>,
    },
    /// The set, or task `index`, carries a key outside the format.
    #[error("{} has unknown key {key}", place(*index))]
    UnknownKey {
        /// The task, or `None` for the set itself.
        index: Option<usize>,
        /// The key.
        key: String,
    },
    /// A key is absent or holds the wrong JSON type.
    #[error("{} key {key} is missing or has the wrong type", place(*index))]
    WrongType {
        /// The task, or `None` for the set itself.
        index: Option<usize>,
        /// The key.
        key: &'static str,
    },
    /// The set name is empty, too long or outside `[a-z0-9._-]`.
    #[error("the set name must be 1 to 64 of [a-z0-9._-]")]
    BadName,
    /// The set holds no task, or more than 256.
    #[error("the set holds {count} tasks; 1 to 256")]
    TaskCount {
        /// The number of tasks.
        count: usize,
    },
    /// Task `index` has an id that is empty, too long or outside `[a-z0-9._-]`.
    #[error("task {index} id must be 1 to 64 of [a-z0-9._-]")]
    BadId {
        /// The task.
        index: usize,
    },
    /// Task `index` repeats an earlier id.
    #[error("task {index} repeats an earlier id")]
    DuplicateId {
        /// The task.
        index: usize,
    },
    /// Task `index` starts from no file, or more than 64.
    #[error("task {index} starts from {count} files; 1 to 64")]
    FileCount {
        /// The task.
        index: usize,
        /// The number of files.
        count: usize,
    },
    /// Task `index` names an origin file outside the repository or inside `.git`.
    #[error("task {index} file {path:?} is not a repo-relative path outside .git")]
    BadFile {
        /// The task.
        index: usize,
        /// The path.
        path: String,
    },
    /// Task `index` has an origin file over 256 KiB.
    #[error("task {index} file {path:?} is over 256 KiB")]
    FileTooLarge {
        /// The task.
        index: usize,
        /// The path.
        path: String,
    },
    /// Task `index` carries no acceptance command, or more than 16: a task no
    /// command checks would count as accepted for nothing.
    #[error("task {index} carries {count} acceptance commands; 1 to 16")]
    AcceptanceCount {
        /// The task.
        index: usize,
        /// The number of commands.
        count: usize,
    },
    /// Task `index` is refused by the executor's own task parser.
    #[error("task {index}: {cause}")]
    Task {
        /// The task.
        index: usize,
        /// The parser's reason.
        #[source]
        cause: TaskError,
    },
    /// Task `index` names no executor path Larry knows.
    #[error("task {index} path {path:?} is unknown; use \"command\" or \"session\"")]
    UnknownPath {
        /// The task.
        index: usize,
        /// The path supplied by the set.
        path: String,
    },
}

/// Where a refusal is: one task of the set, or the set itself.
fn place(index: Option<usize>) -> String {
    match index {
        Some(task) => format!("task {task}"),
        None => String::from("the set"),
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
    let unknown = object.keys().find(|key| !allowed.contains(key));
    if let Some(key) = unknown {
        return Err(SetError::UnknownKey {
            index,
            key: key.to_owned(),
        });
    }
    Ok(())
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
    let path = match object.get("path") {
        None => EvalPath::Command,
        Some(value) => match value.as_str() {
            Some("command") => EvalPath::Command,
            Some("session") => EvalPath::Session,
            Some(path) => {
                return Err(SetError::UnknownPath {
                    index,
                    path: path.to_owned(),
                });
            }
            None => {
                return Err(SetError::WrongType {
                    index: Some(index),
                    key: "path",
                });
            }
        },
    };
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
        path,
        write,
        acceptance,
    };
    // The set parser validates task shape before an eval arm creates its real
    // absolute origin. A host temp path keeps this validation on the same
    // absolute-repository contract without authorizing or touching a directory.
    let validation_repo = std::env::temp_dir().to_string_lossy().into_owned();
    task.task(&validation_repo)
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
