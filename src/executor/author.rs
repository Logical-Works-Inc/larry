//! The model step of a task that carries a brief (larry#123): for each file the
//! brief names, one completion through the model client asks for that file's
//! complete new content, and the reply is written into the workspace before any
//! acceptance command runs. The model never runs a command (model commands stay
//! denied until Nova, logicalworks-crates#846); it only writes the files named,
//! and never through a symlink the clone carries.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use lgwks_std::json::{Map, Value, json};

use crate::model::{Completion, Message, ModelClient, ModelError, Role};

use super::{ExecError, TaskError};

/// Longest goal, in bytes.
const MAX_GOAL_BYTES: usize = 8192;
/// Most files one brief may name: one completion each.
const MAX_FILES: usize = 64;
/// Longest file path, in bytes.
const MAX_PATH_BYTES: usize = 1024;
/// Largest current content of one file sent to the model, in bytes.
const MAX_SOURCE_BYTES: u64 = 262_144;
/// The standing instruction of every completion.
const SYSTEM: &str = "You are Larry's executor. Reply with the complete new content of the one file named below and nothing else: no explanation and no surrounding code fence. Your reply replaces the file.";

/// What the model is asked to do: the goal and the files it may write, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Brief {
    /// The goal, as the planner wrote it.
    goal: String,
    /// Repo-relative paths, distinct, each made of normal components only.
    files: Vec<String>,
}

impl Brief {
    /// Reads `goal` and `files` from a task document: both or neither.
    pub(crate) fn parse(object: &Map) -> Result<Option<Brief>, TaskError> {
        let (goal, files) = match (object.get("goal"), object.get("files")) {
            (None, None) => return Ok(None),
            (Some(goal), Some(files)) => (goal, files),
            _ => return Err(TaskError::IncompleteBrief),
        };
        let goal = goal.as_str().ok_or(TaskError::WrongType { key: "goal" })?;
        if goal.trim().is_empty() || goal.len() > MAX_GOAL_BYTES || goal.contains('\0') {
            return Err(TaskError::BadGoal);
        }
        let items = files
            .as_array()
            .ok_or(TaskError::WrongType { key: "files" })?;
        if items.is_empty() || items.len() > MAX_FILES {
            return Err(TaskError::FileCount { count: items.len() });
        }
        let mut checked: Vec<String> = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let path = item.as_str().ok_or(TaskError::WrongType { key: "files" })?;
            if !is_writable_path(path) || checked.iter().any(|seen| seen == path) {
                return Err(TaskError::BadFile { index });
            }
            checked.push(path.to_owned());
        }
        Ok(Some(Brief {
            goal: goal.to_owned(),
            files: checked,
        }))
    }
}

/// A path the model may write: non-empty, bounded, NUL-free, relative, made of
/// normal components only (no `.`, `..`, root or prefix), none of them `.git`.
pub(crate) fn is_writable_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_BYTES
        && !path.contains('\0')
        && Path::new(path).components().all(|part| match part {
            Component::Normal(name) => name != ".git",
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => false,
        })
}

/// Why the model step could not write one file.
#[derive(Debug)]
#[non_exhaustive]
pub enum AuthorFault {
    /// A component of the path is a symlink in the clone.
    Symlink,
    /// The path names something other than a regular file.
    NotAFile,
    /// The current content exceeds the bound sent to the model.
    TooLarge {
        /// The file's length in bytes.
        len: u64,
    },
    /// The current content is not UTF-8 text.
    NotText(std::str::Utf8Error),
    /// The file or a parent directory could not be read or written.
    Io(io::Error),
    /// The model gateway refused or failed.
    Model(ModelError),
}

impl fmt::Display for AuthorFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Symlink => out.write_str("a component of the path is a symlink"),
            Self::NotAFile => out.write_str("the path is not a regular file"),
            Self::TooLarge { len } => write!(
                out,
                "the file is {len} bytes, over the {MAX_SOURCE_BYTES}-byte bound"
            ),
            Self::NotText(ref cause) => write!(out, "the file is not UTF-8 text: {cause}"),
            Self::Io(ref cause) => write!(out, "{cause}"),
            Self::Model(ref cause) => write!(out, "{cause}"),
        }
    }
}

impl std::error::Error for AuthorFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Io(ref cause) => Some(cause),
            Self::Model(ref cause) => Some(cause),
            Self::NotText(ref cause) => Some(cause),
            Self::Symlink | Self::NotAFile | Self::TooLarge { .. } => None,
        }
    }
}

/// The typed refusal for `path`.
fn refused(path: &str, fault: AuthorFault) -> ExecError {
    ExecError::Author {
        path: path.to_owned(),
        fault,
    }
}

/// Writes each file the brief names with the model's reply, in order, inside
/// `tree`, and returns one record per file for the sealed report.
pub(crate) fn author(
    brief: &Brief,
    tree: &Path,
    client: &ModelClient,
) -> Result<Vec<Value>, ExecError> {
    let mut authored = Vec::with_capacity(brief.files.len());
    for path in &brief.files {
        let target = checked_target(tree, path)?;
        let current = current_content(&target, path)?;
        let messages = [
            Message::new(Role::System, SYSTEM),
            Message::new(Role::User, prompt(&brief.goal, path, current.as_deref())),
        ];
        let completion: Completion = client
            .complete(&messages)
            .map_err(|cause| refused(path, AuthorFault::Model(cause)))?;
        let content = file_text(completion.content());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|cause| refused(path, AuthorFault::Io(cause)))?;
        }
        fs::write(&target, content.as_bytes())
            .map_err(|cause| refused(path, AuthorFault::Io(cause)))?;
        let usage = completion.usage();
        authored.push(json!({
            "path": path,
            "bytes": content.len(),
            "existed": current.is_some(),
            "prompt_tokens": usage.map(|spent| spent.prompt_tokens),
            "completion_tokens": usage.map(|spent| spent.completion_tokens),
        }));
    }
    Ok(authored)
}

/// `tree` joined with `path`, refused when any component that already exists is a symlink:
/// a clone may carry a symlink that points outside the workspace.
fn checked_target(tree: &Path, path: &str) -> Result<PathBuf, ExecError> {
    let mut target = tree.to_path_buf();
    for part in Path::new(path).components() {
        target.push(part);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(refused(path, AuthorFault::Symlink));
            }
            Ok(_) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => return Err(refused(path, AuthorFault::Io(cause))),
        }
    }
    Ok(target)
}

/// The file's current text, `None` when it does not exist yet.
fn current_content(target: &Path, path: &str) -> Result<Option<String>, ExecError> {
    let meta = match fs::metadata(target) {
        Ok(meta) => meta,
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(refused(path, AuthorFault::Io(cause))),
    };
    if !meta.is_file() {
        return Err(refused(path, AuthorFault::NotAFile));
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(refused(path, AuthorFault::TooLarge { len: meta.len() }));
    }
    let bytes = fs::read(target).map_err(|cause| refused(path, AuthorFault::Io(cause)))?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|cause| refused(path, AuthorFault::NotText(cause.utf8_error())))
}

/// The user turn: the goal, the path and the current content.
fn prompt(goal: &str, path: &str, current: Option<&str>) -> String {
    match current {
        Some(text) => format!("Goal:\n{goal}\n\nFile: {path}\n\nCurrent content:\n{text}"),
        None => format!("Goal:\n{goal}\n\nFile: {path}\n\nThe file does not exist yet; write it."),
    }
}

/// The reply as file text: one surrounding code fence removed, a final newline ensured.
fn file_text(reply: &str) -> String {
    let mut text = unfenced(reply).to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The reply without one surrounding ```` ``` ```` fence; the reply itself when it has none.
/// A terminal session reads its replies through it too (larry#93).
pub(super) fn unfenced(reply: &str) -> &str {
    let Some(opened) = reply.trim().strip_prefix("```") else {
        return reply;
    };
    let Some((_, inner)) = opened.split_once('\n') else {
        return reply;
    };
    match inner.strip_suffix("```") {
        Some(body) => body,
        None => reply,
    }
}

#[cfg(test)]
#[path = "author_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sim_author.rs"]
mod sim;
