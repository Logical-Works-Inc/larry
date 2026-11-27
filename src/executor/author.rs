//! The model step of a task that carries a brief (larry#123): for each file the
//! brief names, one completion through the model client asks for that file's
//! complete new content, and the reply is written into the workspace before any
//! acceptance command runs. The model never runs a command (model commands stay
//! denied until Nova, logicalworks-crates#846); it only writes the files named,
//! and never through a symlink the clone carries.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use lgwks_macros::Error;
use lgwks_std::json::{Map, Value, json};

use crate::model::{
    Completion, Kind, Message, ModelClient, ModelError, Role, Route, structured_content,
};

use super::{ExecError, TaskError};

/// Longest goal, in bytes.
const MAX_GOAL_BYTES: usize = 8192;
/// Most files one brief may name: one completion each.
const MAX_FILES: usize = 64;
/// Longest file path, in bytes.
const MAX_PATH_BYTES: usize = 1024;
/// Largest current content of one file sent to the model, in bytes.
const MAX_SOURCE_BYTES: u64 = 262_144;
/// Most bytes of brief context one prompt carries; past it each prompt carries only its own file (larry#230).
const MAX_CONTEXT_BYTES: usize = 524_288;
/// The standing instruction of every completion.
const SYSTEM: &str = "You are Larry's executor. The brief's files follow as they were before any change, then the goal. Reply with the complete new content of the one file the last line names and nothing else: no explanation and no surrounding code fence. Your reply replaces that file.";
/// The standing instruction of a completion over a structured route: the file
/// rides inside one JSON object, because the kind answers in its own envelope
/// and bare text would be ambiguous there (larry#205).
const SYSTEM_STRUCTURED: &str = "You are Larry's executor. The brief's files follow as they were before any change, then the goal. Reply with exactly one JSON object and nothing else: {\"content\": \"<the complete new content of the one file the last line names>\"}. Your reply replaces that file.";

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
/// normal components only (no `.`, `..`, root or prefix), none of them `.git` in any
/// case (larry#335: a case-insensitive file system would otherwise reach `.git`).
pub(crate) fn is_writable_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_BYTES
        && !path.contains('\0')
        && Path::new(path).components().all(|part| match part {
            Component::Normal(name) => !name.eq_ignore_ascii_case(".git"),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => false,
        })
}

/// Why the model step could not write one file.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AuthorFault {
    /// A component of the path is a symlink in the clone.
    #[error("a component of the path is a symlink")]
    Symlink,
    /// The path names something other than a regular file.
    #[error("the path is not a regular file")]
    NotAFile,
    /// The current content exceeds the bound sent to the model.
    #[error("the file is {len} bytes, over the {}-byte bound", MAX_SOURCE_BYTES)]
    TooLarge {
        /// The file's length in bytes.
        len: u64,
    },
    /// The current content is not UTF-8 text.
    #[error("the file is not UTF-8 text: {0}")]
    NotText(#[source] std::str::Utf8Error),
    /// The file or a parent directory could not be read or written.
    #[error("{0}")]
    Io(#[source] io::Error),
    /// The model gateway refused or failed.
    #[error("{0}")]
    Model(#[source] ModelError),
    /// The model's reply holds nothing but whitespace once its fence is removed, so it is
    /// never written as the file (larry#396).
    #[error("the model's reply is empty, so the file is not written")]
    EmptyReply,
}

/// The typed refusal for `path`.
fn refused(path: &str, fault: AuthorFault) -> ExecError {
    ExecError::Author {
        path: path.to_owned(),
        fault,
    }
}

/// One file of the brief as it was before the model wrote anything.
struct Source<'brief> {
    /// The repo-relative path the brief names.
    path: &'brief str,
    /// The file inside the workspace, checked against symlinks.
    target: PathBuf,
    /// The text it held, `None` when it did not exist yet.
    before: Option<String>,
}

/// Writes each file the brief names with the model's reply, in order, inside
/// `tree`, and returns one record per file for the sealed report.
///
/// Every file is read before any is written, so all the brief's prompts show the
/// files as they were, and each prompt of the brief begins with the same bytes
/// (larry#230): the route's template when it names one, the standing
/// instruction, the tenant's partition, then the shared context. A provider's
/// prefix cache reads the context once and every later file's prompt hits it.
/// `partition` names the tenant, so no two tenants share a prompt prefix. The
/// snapshot holds at most 64 files of 256 KiB; past
/// [`MAX_CONTEXT_BYTES`] of context, or when any request carrying it would pass
/// the route's request bound, each prompt carries only its own file.
pub(crate) fn author(
    brief: &Brief,
    tree: &Path,
    client: &ModelClient,
    route: &Route,
    partition: &str,
) -> Result<Vec<Value>, ExecError> {
    let mut sources: Vec<Source<'_>> = Vec::with_capacity(brief.files.len());
    for path in &brief.files {
        let target = checked_target(tree, path)?;
        let before = current_content(&target, path)?;
        sources.push(Source {
            path: path.as_str(),
            target,
            before,
        });
    }
    let views: Vec<(&str, Option<&str>)> = sources
        .iter()
        .map(|source| (source.path, source.before.as_deref()))
        .collect();
    let system = system_line(route, partition);
    let paths: Vec<&str> = sources.iter().map(|source| source.path).collect();
    let shared = fitting_context(
        shared_context(&brief.goal, &views),
        &system,
        &paths,
        |messages| client.fits(messages),
    );
    let prefix = prefix_label(shared.as_deref());
    let mut authored = Vec::with_capacity(sources.len());
    for source in &sources {
        let path = source.path;
        let turn = user_turn(
            shared.as_deref(),
            &brief.goal,
            path,
            source.before.as_deref(),
        );
        let messages = [
            Message::new(Role::System, system.as_str()),
            Message::new(Role::User, turn),
        ];
        let completion: Completion = client
            .complete(&messages)
            .map_err(|cause| refused(path, AuthorFault::Model(cause)))?;
        let content =
            reply_file(route.kind(), completion.content()).map_err(|fault| refused(path, fault))?;
        if let Some(parent) = source.target.parent() {
            fs::create_dir_all(parent).map_err(|cause| refused(path, AuthorFault::Io(cause)))?;
        }
        fs::write(&source.target, content.as_bytes())
            .map_err(|cause| refused(path, AuthorFault::Io(cause)))?;
        let usage = completion.usage();
        authored.push(json!({
            "path": path,
            "bytes": content.len(),
            "existed": source.before.is_some(),
            "prompt_tokens": usage.map(|spent| spent.prompt_tokens),
            "completion_tokens": usage.map(|spent| spent.completion_tokens),
            "cached_tokens": usage.and_then(|spent| spent.cached_tokens),
            "cache_write_tokens": usage.and_then(|spent| spent.cache_write_tokens),
            "prefix": prefix,
        }));
    }
    Ok(authored)
}

/// The system turn: the route's template when it names one, the standing
/// instruction for the kind, then the tenant's partition, so two tenants'
/// prompts differ from the first byte after the instruction and a provider's
/// prefix cache never serves one tenant's context to another.
fn system_line(route: &Route, partition: &str) -> String {
    let instruction = if route.kind().structured() {
        SYSTEM_STRUCTURED
    } else {
        SYSTEM
    };
    match route.prompt() {
        Some(template) => format!("{template}\n{instruction}\nCache partition: {partition}"),
        None => format!("{instruction}\nCache partition: {partition}"),
    }
}

/// The context every prompt of one brief begins with: the brief's files as they
/// were, sorted by path bytes so the brief's own order never changes a byte, then
/// the goal. `None` when it would be longer than [`MAX_CONTEXT_BYTES`]; it is cut
/// off as soon as it is.
fn shared_context(goal: &str, files: &[(&str, Option<&str>)]) -> Option<String> {
    let mut ordered: Vec<(&str, Option<&str>)> = files.to_vec();
    ordered.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut text = String::from("Files of the brief, before any change:\n");
    for (path, content) in ordered {
        text.push_str("\n=== ");
        text.push_str(path);
        match content {
            Some(body) => {
                text.push_str(" ===\n");
                text.push_str(body);
                if !body.ends_with('\n') {
                    text.push('\n');
                }
            }
            None => text.push_str(" (does not exist yet) ===\n"),
        }
        if text.len() > MAX_CONTEXT_BYTES {
            return None;
        }
    }
    text.push_str("\nGoal:\n");
    text.push_str(goal);
    text.push('\n');
    (text.len() <= MAX_CONTEXT_BYTES).then_some(text)
}

/// What a record says about its prompt: `shared` when the brief's prompts begin with
/// one context, `own` when each carries only its own file.
fn prefix_label(shared: Option<&str>) -> &'static str {
    if shared.is_some() { "shared" } else { "own" }
}

/// `shared` when every request of the brief that carries it fits the route, as
/// `fits` (the client's own pre-send check) says; otherwise `None`, so the brief
/// falls back to one-file prompts instead of failing. The context is bounded in
/// raw bytes, but JSON escaping can grow a control byte sixfold, and a route may
/// name a bound below the default, so only the encoded request decides.
fn fitting_context(
    shared: Option<String>,
    system: &str,
    paths: &[&str],
    fits: impl Fn(&[Message]) -> bool,
) -> Option<String> {
    shared.filter(|context| {
        paths.iter().all(|path| {
            fits(&[
                Message::new(Role::System, system),
                Message::new(Role::User, user_turn(Some(context), "", path, None)),
            ])
        })
    })
}

/// The user turn: the shared context and the file to write now, or, when the
/// context was too large to share, the prompt that carries only this file.
fn user_turn(shared: Option<&str>, goal: &str, path: &str, current: Option<&str>) -> String {
    match shared {
        Some(context) => format!("{context}\nWrite this file now: {path}"),
        None => prompt(goal, path, current),
    }
}

/// `tree` joined with `path`, refused when any component that already exists is a symlink:
/// a clone may carry a symlink that points outside the workspace. The workspace copy checks
/// every ancestor of each write with it too, so an untracked symlink cannot shadow a tracked
/// directory name (larry#335, CWE-59).
pub(crate) fn checked_target<P: AsRef<Path>>(tree: &Path, path: P) -> Result<PathBuf, ExecError> {
    let relative = path.as_ref();
    let mut target = tree.to_path_buf();
    for part in relative.components() {
        target.push(part);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(refused(
                    &relative.display().to_string(),
                    AuthorFault::Symlink,
                ));
            }
            Ok(_) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => {
                return Err(refused(
                    &relative.display().to_string(),
                    AuthorFault::Io(cause),
                ));
            }
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

/// The file text a reply writes over `kind`, refused when nothing but
/// whitespace is left: an empty reply is a cut or a refusal, never a file
/// (larry#396). A structured kind's reply is the one JSON object the brief asks
/// for, so the `content` string is the file and anything else is refused,
/// never written (larry#205).
fn reply_file(kind: Kind, reply: &str) -> Result<String, AuthorFault> {
    let text = if kind.structured() {
        let content = structured_content(reply).map_err(AuthorFault::Model)?;
        file_text(&content)
    } else {
        file_text(reply)
    };
    if text.trim().is_empty() {
        return Err(AuthorFault::EmptyReply);
    }
    Ok(text)
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
