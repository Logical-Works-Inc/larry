//! The workspace of a terminal session (larry#243, ADR 0046): a bounded copy of the
//! files the person sees in their folder, committed as a base inside the workspace
//! and never in the folder, and the way a verified change goes back.
//!
//! A git folder is listed by `git ls-files --cached --others --exclude-standard`, so
//! the copy is the working state (uncommitted edits, untracked files, files deleted
//! on disk) without the ignored build output; any other folder is walked. A symlink
//! is copied as a symlink and never followed, a `.git` entry is never copied, and
//! every listed path is checked to stay beneath the folder. The copy is bounded by
//! files, bytes, directory depth and listing size, and a folder past a bound is
//! refused with [`ExecError::SnapshotTooLarge`] while the half-built workspace is
//! removed. Apply is `git apply` of the sealed diff, run only when the diff is whole
//! and its BLAKE3 digest still matches, under `GIT_CEILING_DIRECTORIES` so a folder
//! inside some other repository is patched as a folder. Source: git-ls-files(1),
//! git-apply(1) and CWE-59 (a link followed out of a tree).

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

use super::{Diff, ExecError, Git, Workspace, flag, spawn_error};
use crate::executor::author::checked_target;
use crate::executor::run::clip;

/// Most entries (files and links) one snapshot copies.
pub(crate) const MAX_FILES: usize = 20_000;

/// Most bytes of file content one snapshot copies (256 MiB).
pub(crate) const MAX_BYTES: u64 = 268_435_456;

/// Most bytes of `git ls-files` output read (8 MiB): past it the folder is refused.
const MAX_LISTING_BYTES: u64 = 8_388_608;

/// Deepest directory nesting one snapshot walks.
const MAX_DEPTH: usize = 64;

/// Most files the review summary names; the rest are counted.
const SUMMARY_FILES: usize = 12;

/// Bytes of git's refusal kept in an apply fault.
const FAULT_BYTES: usize = 512;

/// What a diff header line says happened to a file, first match wins.
const KINDS: [(&str, &str); 4] = [
    ("new file mode 120000", "link"),
    ("new file mode", "new"),
    ("deleted file mode", "deleted"),
    ("rename from", "renamed"),
];

/// The settings of the base commit: an identity, and no signing, hooks of maintenance.
const BASE_CONFIG: [&str; 5] = [
    "user.name=larry",
    "user.email=larry@localhost.invalid",
    "commit.gpgsign=false",
    "maintenance.auto=false",
    "gc.auto=0",
];

/// The bounds of one snapshot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Most entries copied.
    files: usize,
    /// Most content bytes copied.
    bytes: u64,
    /// Deepest directory level walked.
    depth: usize,
}

impl Limits {
    /// The bounds every session runs under.
    pub(crate) const STANDARD: Self = Self {
        files: MAX_FILES,
        bytes: MAX_BYTES,
        depth: MAX_DEPTH,
    };
}

/// What one snapshot copied, for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct Copied {
    /// Entries copied, files and symlinks.
    pub(crate) files: usize,
    /// Content bytes copied.
    pub(crate) bytes: u64,
}

/// Why a verified change did not go back into the folder.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) enum ApplyFault {
    /// The diff was cut at its bound, so it is not the whole change.
    Truncated,
    /// The diff's bytes no longer hash to the digest it was sealed with.
    DigestMismatch,
    /// The patch file or git could not be written or started.
    Io(io::Error),
    /// git refused the patch; the folder is unchanged.
    Git {
        /// Git's refusal, clipped.
        detail: String,
    },
}

impl fmt::Display for ApplyFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated => {
                out.write_str("the change is over 1 MiB, so it cannot be applied whole")
            }
            Self::DigestMismatch => out.write_str("the change no longer matches its digest"),
            Self::Io(ref cause) => write!(out, "the change could not be handed to git: {cause}"),
            Self::Git { ref detail } => write!(out, "git refused the change: {detail}"),
        }
    }
}

impl std::error::Error for ApplyFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Io(ref cause) => Some(cause),
            Self::Truncated | Self::DigestMismatch | Self::Git { .. } => None,
        }
    }
}

/// `arguments` behind the overrides every git call inside the person's folder takes: that
/// folder's own config never names a program git runs (`core.fsmonitor`), and the system
/// config is not read (`Git::command`). The folder is untrusted input to Larry.
fn guarded<'a>(arguments: &[&'a OsStr]) -> Vec<&'a OsStr> {
    let mut full = vec![OsStr::new("-c"), OsStr::new("core.fsmonitor=false")];
    full.extend_from_slice(arguments);
    full
}

/// `text` with every control character and bidirectional formatting mark made `?`, so a
/// file name cannot rewrite the line a person reads it in.
fn plain(text: &str) -> String {
    text.chars()
        .map(|character| {
            let bidi = matches!(
                character,
                '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
            );
            if character.is_control() || bidi {
                '?'
            } else {
                character
            }
        })
        .collect()
}

/// The refusal for a folder past a count `limit` of `what`.
fn too_many(what: &'static str, limit: usize) -> ExecError {
    match u64::try_from(limit) {
        Ok(limit) => ExecError::SnapshotTooLarge { what, limit },
        Err(cause) => ExecError::Workspace(io::Error::other(cause)),
    }
}

/// Kills and reaps a child whose output is no longer wanted, then returns `error`, or the
/// failure that names both when the child could not be ended.
fn abandon(child: &mut std::process::Child, error: ExecError) -> ExecError {
    if let Err(fault) = child.kill() {
        return ExecError::cleanup(
            error,
            "the listing child could not be killed",
            ExecError::Reap(fault),
        );
    }
    if let Err(fault) = child.wait() {
        return ExecError::cleanup(
            error,
            "the listing child could not be reaped",
            ExecError::Reap(fault),
        );
    }
    error
}

/// Every entry of the folder `origin`, relative and sorted: git's listing when `origin`
/// is a git work tree, else a walk.
fn listing(git: &Git<'_>, origin: &Path, limits: Limits) -> Result<Vec<PathBuf>, ExecError> {
    let probe = git.output(
        Some(origin),
        &guarded(&[OsStr::new("rev-parse"), OsStr::new("--is-inside-work-tree")]),
    )?;
    if !probe.status.success() {
        // Only a folder that is no repository at all is walked; a repository git cannot
        // read is refused, since a walk would copy the build output its ignore rules hide.
        // A `.git` that git cannot read (a damaged HEAD says "not a git repository" too)
        // still marks a repository: only a folder with none above it is walked.
        let detail = String::from_utf8_lossy(&probe.stderr).trim().to_owned();
        let resolved = std::fs::canonicalize(origin).map_err(ExecError::Workspace)?;
        if detail.contains("not a git repository")
            && !resolved.ancestors().any(|dir| dir.join(".git").exists())
        {
            return walk(origin, limits);
        }
        return Err(ExecError::Git {
            step: "rev-parse",
            detail,
        });
    }
    if String::from_utf8_lossy(&probe.stdout).trim() != "true" {
        return walk(origin, limits);
    }
    let mut command = git.command(
        Some(origin),
        &guarded(&[
            OsStr::new("ls-files"),
            OsStr::new("-z"),
            OsStr::new("--cached"),
            OsStr::new("--others"),
            OsStr::new("--exclude-standard"),
        ]),
    );
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(spawn_error)?;
    let Some(stdout) = child.stdout.take() else {
        return Err(abandon(
            &mut child,
            ExecError::Workspace(io::Error::other("the listing pipe was not opened")),
        ));
    };
    let mut raw = Vec::new();
    let read = stdout
        .take(MAX_LISTING_BYTES.saturating_add(1))
        .read_to_end(&mut raw);
    if let Err(cause) = read {
        return Err(abandon(&mut child, ExecError::Workspace(cause)));
    }
    let length =
        u64::try_from(raw.len()).map_err(|cause| ExecError::Workspace(io::Error::other(cause)))?;
    if length > MAX_LISTING_BYTES {
        return Err(abandon(
            &mut child,
            ExecError::SnapshotTooLarge {
                what: "bytes of file names",
                limit: MAX_LISTING_BYTES,
            },
        ));
    }
    let status = child.wait().map_err(spawn_error)?;
    if !status.success() {
        return Err(ExecError::Git {
            step: "ls-files",
            detail: String::from("git could not list the folder"),
        });
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for name in raw.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
        let text = std::str::from_utf8(name).map_err(|cause| {
            ExecError::Workspace(io::Error::new(io::ErrorKind::InvalidData, cause))
        })?;
        found.push(PathBuf::from(text));
    }
    found.sort();
    found.dedup();
    if found.len() > limits.files {
        return Err(too_many("files", limits.files));
    }
    Ok(found)
}

/// Every file and symlink beneath `origin`, relative and sorted, no `.git` entry and no
/// link followed. Directories are visited from an explicit stack, so depth is a number
/// and not recursion.
fn walk(origin: &Path, limits: Limits) -> Result<Vec<PathBuf>, ExecError> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut pending: Vec<(PathBuf, usize)> = vec![(PathBuf::new(), 0)];
    while let Some((relative, depth)) = pending.pop() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(origin.join(&relative)).map_err(ExecError::Workspace)? {
            let entry = entry.map_err(ExecError::Workspace)?;
            let kind = entry.file_type().map_err(ExecError::Workspace)?;
            names.push((entry.file_name(), kind));
            if found.len().saturating_add(names.len()) > limits.files {
                return Err(too_many("files", limits.files));
            }
        }
        names.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, kind) in names {
            if name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let path = relative.join(&name);
            if kind.is_dir() {
                let next = depth.saturating_add(1);
                if next > limits.depth {
                    return Err(too_many("directory levels", limits.depth));
                }
                pending.push((path, next));
            } else if copies_as_entry(kind) {
                found.push(path);
            }
            if found.len().saturating_add(pending.len()) > limits.files {
                return Err(too_many("files", limits.files));
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Whether an entry of this kind is copied: a regular file or a symlink. A directory is
/// walked, not copied, and a fifo, a socket or a device has no bytes to copy. The unix file
/// types are read only where the platform has them (larry#343).
#[cfg(unix)]
fn copies_as_entry(kind: std::fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    kind.is_symlink()
        || !(kind.is_dir()
            || kind.is_fifo()
            || kind.is_socket()
            || kind.is_block_device()
            || kind.is_char_device())
}

/// Whether an entry of this kind is copied: a regular file or a symlink; every other kind
/// has no bytes to copy.
#[cfg(not(unix))]
fn copies_as_entry(kind: std::fs::FileType) -> bool {
    kind.is_file() || kind.is_symlink()
}

/// Whether `relative` stays beneath the folder it is joined to: only named components,
/// none of them a `.git`.
fn stays_beneath(relative: &Path) -> bool {
    relative.components().all(|part| match part {
        Component::Normal(name) => !name.eq_ignore_ascii_case(".git"),
        Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir => {
            false
        }
    })
}

/// Makes the symlink `to` pointing at `target`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn link(target: &Path, to: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, to)
}

/// A target with no symlink call here refuses a folder that holds one.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn link(_target: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this platform copies no symlink",
    ))
}

/// Copies each of `entries` from `origin` to `tree`: files with their permission bits,
/// symlinks as links. An entry missing on disk (a tracked file the person deleted) is
/// skipped, so the copy is what is there.
fn copy_entries(
    origin: &Path,
    tree: &Path,
    entries: &[PathBuf],
    limits: Limits,
) -> Result<Copied, ExecError> {
    let mut copied = Copied { files: 0, bytes: 0 };
    for relative in entries {
        if !stays_beneath(relative) {
            return Err(ExecError::Workspace(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("a listed path leaves the folder: {}", relative.display()),
            )));
        }
        // `stays_beneath` is lexical only: an untracked symlink copied before a tracked
        // file beneath its old directory name would make the write follow the link out
        // (larry#335, CWE-59). Refuse any write beneath a symlink the copy already made.
        checked_target(tree, relative)?;
        let from = origin.join(relative);
        let to = tree.join(relative);
        let meta = match std::fs::symlink_metadata(&from) {
            Ok(meta) => meta,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => continue,
            Err(cause) => return Err(ExecError::Workspace(cause)),
        };
        let kind = meta.file_type();
        if !copies_as_entry(kind) {
            continue;
        }
        if copied.files >= limits.files {
            return Err(too_many("files", limits.files));
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(ExecError::Workspace)?;
        }
        if kind.is_symlink() {
            let target = std::fs::read_link(&from).map_err(ExecError::Workspace)?;
            link(&target, &to).map_err(ExecError::Workspace)?;
        } else {
            let wrote = std::fs::copy(&from, &to).map_err(ExecError::Workspace)?;
            copied.bytes = copied.bytes.saturating_add(wrote);
            if copied.bytes > limits.bytes {
                return Err(ExecError::SnapshotTooLarge {
                    what: "bytes",
                    limit: limits.bytes,
                });
            }
        }
        copied.files = copied.files.saturating_add(1);
    }
    Ok(copied)
}

impl Workspace {
    /// A new workspace named `task_id` holding a copy of the folder `origin`, committed
    /// as the base the session's change is measured against. The folder is only read.
    pub(crate) fn snapshot(
        state: &Path,
        origin: &Path,
        task_id: &str,
        path_env: &str,
    ) -> Result<(Workspace, Copied), ExecError> {
        Self::snapshot_within(state, origin, task_id, path_env, Limits::STANDARD)
    }

    /// [`Self::snapshot`] under `limits`.
    pub(crate) fn snapshot_within(
        state: &Path,
        origin: &Path,
        task_id: &str,
        path_env: &str,
        limits: Limits,
    ) -> Result<(Workspace, Copied), ExecError> {
        let root = state.join("ws").join(task_id);
        std::fs::create_dir_all(root.join("tmp")).map_err(ExecError::Workspace)?;
        // Built before any git call: a refused folder still leaves a directory to remove.
        let workspace = Workspace {
            tree: root.join("tree"),
            git_dir: root.join("git"),
            root,
            state: state.to_path_buf(),
            path_env: path_env.to_owned(),
        };
        let separate = flag("separate-git-dir", &workspace.git_dir);
        let init = workspace.git().output(
            None,
            &[
                OsStr::new("init"),
                OsStr::new("--quiet"),
                separate.as_os_str(),
                workspace.tree.as_os_str(),
            ],
        )?;
        super::require(init, "init")?;
        let entries = listing(&workspace.git(), origin, limits)?;
        let copied = copy_entries(origin, &workspace.tree, &entries, limits)?;
        super::require(
            workspace.bound(&[OsStr::new("add"), OsStr::new("-A")])?,
            "add",
        )?;
        let mut arguments: Vec<&OsStr> = Vec::new();
        for setting in BASE_CONFIG {
            arguments.push(OsStr::new("-c"));
            arguments.push(OsStr::new(setting));
        }
        arguments.extend([
            OsStr::new("commit"),
            OsStr::new("--quiet"),
            OsStr::new("--allow-empty"),
            OsStr::new("--message"),
            OsStr::new("base"),
        ]);
        super::require(workspace.bound(&arguments)?, "commit")?;
        Ok((workspace, copied))
    }

    /// The id of the commit HEAD names now. Read once, right after the base commit and
    /// before any command runs, so a later diff is against this id and nothing a command
    /// does to HEAD can hide its change.
    pub(crate) fn head(&self) -> Result<String, ExecError> {
        let out = super::require(
            self.bound(&[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("HEAD"),
            ])?,
            "rev-parse",
        )?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// The workspace's own temporary directory, outside the work tree so a command's
    /// scratch files never enter the diff.
    pub(crate) fn scratch(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Applies `diff`, the change this workspace's commands made, to the folder `origin`.
    /// The diff must be whole and still hash to its digest; git applies it atomically, so
    /// a refusal leaves the folder as it was, and a diff already applied is refused.
    pub(crate) fn apply(&self, origin: &Path, diff: &Diff) -> Result<(), ApplyFault> {
        if diff.truncated {
            return Err(ApplyFault::Truncated);
        }
        let mut hasher = lgwks_std::hash::Hasher::new();
        hasher.update(diff.text.as_bytes());
        if hasher.finalize().to_hex() != diff.digest {
            return Err(ApplyFault::DigestMismatch);
        }
        let patch = self.root.join("apply.patch");
        std::fs::write(&patch, diff.text.as_bytes()).map_err(ApplyFault::Io)?;
        let mut command = self.git().command(
            Some(origin),
            &guarded(&[
                OsStr::new("apply"),
                OsStr::new("--whitespace=nowarn"),
                patch.as_os_str(),
            ]),
        );
        // The folder is patched as a folder, never as part of a repository above it.
        let canonical = std::fs::canonicalize(origin).map_err(ApplyFault::Io)?;
        if let Some(parent) = canonical.parent() {
            command.env("GIT_CEILING_DIRECTORIES", parent);
        }
        let output = command.output().map_err(ApplyFault::Io)?;
        if output.status.success() {
            return Ok(());
        }
        let mut detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        clip(&mut detail, FAULT_BYTES);
        Err(ApplyFault::Git { detail })
    }
}

/// One file of a diff, as the review names it.
struct Touched {
    /// The path after the change.
    path: String,
    /// What happened to it: `new`, `deleted`, `renamed`, `link` or `changed`.
    kind: &'static str,
    /// Lines added.
    added: usize,
    /// Lines removed.
    removed: usize,
}

/// The lines the person reviews before they answer: the totals, then each file (at
/// most [`SUMMARY_FILES`]) with what happened to it and its added and removed lines.
pub(crate) fn summary(diff: &Diff) -> Vec<String> {
    let mut files: Vec<Touched> = Vec::new();
    let mut in_hunk = false;
    for line in diff.text.lines() {
        if let Some(names) = line.strip_prefix("diff --git ") {
            let path = match names.rsplit_once(" b/") {
                Some((_, after)) => after,
                None => names,
            };
            files.push(Touched {
                path: plain(path),
                kind: "changed",
                added: 0,
                removed: 0,
            });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk {
            if line.starts_with('+') {
                file.added = file.added.saturating_add(1);
            } else if line.starts_with('-') {
                file.removed = file.removed.saturating_add(1);
            }
        } else if let Some(entry) = KINDS.iter().find(|entry| line.starts_with(entry.0)) {
            file.kind = entry.1;
        }
    }
    let added: usize = files.iter().map(|file| file.added).sum();
    let removed: usize = files.iter().map(|file| file.removed).sum();
    let noun = if files.len() == 1 { "file" } else { "files" };
    let mut lines = vec![format!(
        "{} {noun} changed, +{added} -{removed}",
        files.len()
    )];
    for file in files.iter().take(SUMMARY_FILES) {
        lines.push(format!(
            "{}  {}  +{} -{}",
            file.path, file.kind, file.added, file.removed
        ));
    }
    let more = files.len().saturating_sub(SUMMARY_FILES);
    if more > 0 {
        lines.push(format!("… and {more} more"));
    }
    lines
}

/// Seeded simulations of the snapshot, the apply and the summary.
#[cfg(test)]
#[path = "sim_snapshot.rs"]
mod sim;
