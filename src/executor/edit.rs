//! A session's read and anchored edit of one file (larry#277). [`read_file`]
//! shows a file with its BLAKE3 digest (larry#586). [`apply_edit`] (larry#606)
//! replaces one place of a file that still has the digest the model read: the
//! exact old text, else its lines with whitespace trimmed, else the block its
//! first and last lines bound. The file is held from the check to the
//! replacement, so two edits that saw one digest cannot both land, and the
//! replacement is whole or absent and keeps the mode. A refusal writes nothing.
//! The path is a writable repo-relative path through no symlink; the open and
//! the byte bound are the governance reader's (`crate::governance::read`). A
//! patch (`super::patch`, larry#624) holds each of its files through [`held`].

use std::ffi::OsStr;
use std::fs::{self, File, Permissions, TryLockError};
use std::io;
use std::path::Path;

use lgwks_macros::Error;
use lgwks_std::fs::capability::Dir;

use super::author::is_writable_path;
use crate::governance::read::{ReadFault, open_regular, read_open};

/// Longest file a read or an edit takes or leaves, in bytes: the brief's
/// per-file bound, so neither carries more than a prompt would.
pub(super) const MAX_FILE_BYTES: u64 = 262_144;

/// Most candidate lines an ambiguous refusal names: enough to pick a longer
/// anchor, however often the text repeats.
const MAX_CANDIDATES: usize = 16;

/// Why a read or an edit was refused. Nothing in the tree changes on any of them.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum EditError {
    /// The path is not a writable repo-relative path, or escapes the snapshot.
    #[error("edit path {path} escapes the snapshot")]
    PathOutside {
        /// The refused path.
        path: String,
    },
    /// The path names nothing regular in the snapshot.
    #[error("edit path {path} is not a regular file")]
    NotAFile {
        /// The refused path.
        path: String,
    },
    /// The file, or the file an edit would leave, is past the bound.
    #[error(
        "edit file {path} is {len} bytes, over the {}-byte bound",
        MAX_FILE_BYTES
    )]
    TooLarge {
        /// The refused path.
        path: String,
        /// The length in bytes.
        len: u64,
    },
    /// The file is not UTF-8 text.
    #[error("edit file {path} is not UTF-8 text")]
    NotText {
        /// The refused path.
        path: String,
        /// The decoding failure.
        #[source]
        cause: std::str::Utf8Error,
    },
    /// The file changed since the model read it.
    #[error("edit file {path} changed since it was read: its digest is {found}")]
    StaleDigest {
        /// The refused path.
        path: String,
        /// The digest the file holds now.
        found: String,
    },
    /// The old text matches nothing, at no level.
    #[error("edit file {path} holds no match for the old text")]
    NoMatch {
        /// The refused path.
        path: String,
    },
    /// The old text matches more than one place.
    #[error("edit anchor in {path} matches lines {candidates:?}; make the anchor longer")]
    Ambiguous {
        /// The refused path.
        path: String,
        /// The 1-based candidate start lines, in file order, at most 16.
        candidates: Vec<usize>,
    },
    /// Another edit holds the file, or replaced it as this one opened it.
    #[error("edit file {path} is being changed by another edit; read it again")]
    Busy {
        /// The refused path.
        path: String,
    },
    /// The file could not be read or written.
    #[error("edit file {path} could not be read or written: {cause}")]
    Io {
        /// The refused path.
        path: String,
        /// The filesystem failure.
        #[source]
        cause: io::Error,
    },
    /// The patch is not in the Codex shape.
    #[error("patch line {line}: {detail}")]
    PatchSyntax {
        /// The 1-based patch line.
        line: usize,
        /// What is wrong there.
        detail: &'static str,
    },
    /// A patch hunk or one of its anchors sits nowhere.
    #[error("patch hunk at line {line} matches nothing in {path}")]
    PatchMismatch {
        /// The file the hunk is for.
        path: String,
        /// The 1-based patch line the hunk starts at.
        line: usize,
    },
    /// A patch changes a file without naming the digest it saw.
    #[error("patch changes {path} but expect_digests names no digest for it")]
    NoDigest {
        /// The file without a digest.
        path: String,
    },
    /// A patch names the digest of a file it does not change.
    #[error("patch names a digest for {path}, which it does not change")]
    UnusedDigest {
        /// The file the patch does not change.
        path: String,
    },
    /// A patch creates a file that is already there.
    #[error("patch creates {path}, which already exists")]
    Exists {
        /// The existing path.
        path: String,
    },
    /// A patch write failed and a file could not be put back.
    #[error("patch failed and could not put back {left:?}: {cause}")]
    PartlyApplied {
        /// The files and directories still changed.
        left: Vec<String>,
        /// The write that failed.
        #[source]
        cause: Box<EditError>,
    },
}

/// One file as a session read shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileView {
    /// The BLAKE3 hex of `text`: the digest an edit of this file names.
    pub(crate) digest: String,
    /// The whole file.
    pub(crate) text: String,
}

/// One anchored edit as a session reply asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// The repo-relative path the edit changes.
    pub(crate) path: String,
    /// The text that must sit in exactly one place.
    pub(crate) old: String,
    /// Its replacement; empty removes it.
    pub(crate) new: String,
    /// The BLAKE3 hex of the file as the model last saw it.
    pub(crate) expect_digest: String,
}

/// What one applied edit did: the file's digest on both sides, so the next
/// edit's precondition is this one's `after_digest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditReceipt {
    /// The BLAKE3 hex of the file before the edit.
    pub(crate) before_digest: String,
    /// The BLAKE3 hex of the file after it.
    pub(crate) after_digest: String,
}

/// A moment of a read or an edit at which a simulation acts; production does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Window {
    /// The edit has the file open and does not hold it yet.
    Opened,
    /// The file's length passed the bound and its bytes are not read yet.
    Statted,
    /// The edit is decided and the replacement not made yet.
    Decided,
}

/// What acts at each [`Window`].
pub(super) type Seam<'a> = dyn FnMut(Window) -> io::Result<()> + 'a;

/// The BLAKE3 hex of `text`.
pub(super) fn digest_of(text: &str) -> String {
    lgwks_std::hash::blake3(text.as_bytes()).to_hex()
}

/// The filesystem failure `cause` on `path`.
pub(super) fn io_fault(path: &str, cause: io::Error) -> EditError {
    EditError::Io {
        path: path.to_owned(),
        cause,
    }
}

/// The file `path` of the snapshot `root`, with its digest.
pub(crate) fn read_file(root: &Path, path: &str) -> Result<FileView, EditError> {
    read_file_with(root, path, &mut |_| Ok(()))
}

/// [`read_file`] with its windows, for a simulation.
fn read_file_with(root: &Path, path: &str, seam: &mut Seam<'_>) -> Result<FileView, EditError> {
    let (_, file) = open(root, path)?;
    let text = text_of(&file, path, seam)?;
    Ok(FileView {
        digest: digest_of(&text),
        text,
    })
}

/// The file `path` names inside the snapshot `root`, opened with the directory
/// that holds it: writable, beneath the root, through no symlink the tree
/// carries (CWE-59) and a regular file.
pub(super) fn open(root: &Path, path: &str) -> Result<(Dir, File), EditError> {
    if !is_writable_path(path) {
        return Err(EditError::PathOutside {
            path: path.to_owned(),
        });
    }
    open_regular(root, Path::new(path), |_| Ok(())).map_err(|fault| refusal(path, fault))
}

/// The whole of the open snapshot `file`, called `path`, as text, read no
/// further than one byte past the bound.
fn text_of(file: &File, path: &str, seam: &mut Seam<'_>) -> Result<String, EditError> {
    let bytes = read_open(file, MAX_FILE_BYTES, || seam(Window::Statted))
        .map_err(|fault| refusal(path, fault))?;
    String::from_utf8(bytes).map_err(|failure| EditError::NotText {
        path: path.to_owned(),
        cause: failure.utf8_error(),
    })
}

/// The reader's refusal of `path`, as this module names it.
fn refusal(path: &str, fault: ReadFault) -> EditError {
    let path = path.to_owned();
    match fault {
        ReadFault::Symlink => EditError::PathOutside { path },
        ReadFault::NotRegular { .. } => EditError::NotAFile { path },
        ReadFault::TooLarge { bytes } => EditError::TooLarge { path, len: bytes },
        ReadFault::Missing(cause) | ReadFault::Unreadable(cause) => EditError::Io { path, cause },
    }
}

/// Applies one anchored edit under `root`: the file must still have the digest
/// the model read and the old text must sit in exactly one place. The file is
/// unchanged on every refusal.
pub(crate) fn apply_edit(root: &Path, edit: &Edit) -> Result<EditReceipt, EditError> {
    apply_edit_with(root, edit, &mut |_| Ok(()))
}

/// [`apply_edit`] with its windows, for a simulation.
fn apply_edit_with(
    root: &Path,
    edit: &Edit,
    seam: &mut Seam<'_>,
) -> Result<EditReceipt, EditError> {
    let path = edit.path.as_str();
    // `file` holds the lock until this returns, after the replacement.
    let (directory, file, content) = held(root, path, &edit.expect_digest, seam)?;
    let changed = anchored(&content, &edit.old, &edit.new)
        .map_err(|candidates| EditError::Ambiguous {
            path: path.to_owned(),
            candidates,
        })?
        .ok_or_else(|| EditError::NoMatch {
            path: path.to_owned(),
        })?;
    let len =
        u64::try_from(changed.len()).map_err(|cause| io_fault(path, io::Error::other(cause)))?;
    if len > MAX_FILE_BYTES {
        return Err(EditError::TooLarge {
            path: path.to_owned(),
            len,
        });
    }
    seam(Window::Decided).map_err(|cause| io_fault(path, cause))?;
    let mode = file.metadata().map_err(|cause| io_fault(path, cause))?;
    replace(&directory, mode.permissions(), &root.join(path), &changed)
        .map_err(|cause| io_fault(path, cause))?;
    Ok(EditReceipt {
        before_digest: edit.expect_digest.clone(),
        after_digest: digest_of(&changed),
    })
}

/// The file `path` opened, held and read, refused unless its digest is
/// `expect`: the file is held until the returned handle drops.
pub(super) fn held(
    root: &Path,
    path: &str,
    expect: &str,
    seam: &mut Seam<'_>,
) -> Result<(Dir, File, String), EditError> {
    let (directory, file) = open(root, path)?;
    seam(Window::Opened).map_err(|cause| io_fault(path, cause))?;
    hold(&file, &root.join(path), path)?;
    let content = text_of(&file, path, seam)?;
    let found = digest_of(&content);
    if found != expect {
        return Err(EditError::StaleDigest {
            path: path.to_owned(),
            found,
        });
    }
    Ok((directory, file, content))
}

/// Holds the open `file` until it is dropped, refused when another edit holds it
/// or replaced it since it was opened: only the holder of the file now at
/// `target` may replace it (CWE-367).
fn hold(file: &File, target: &Path, path: &str) -> Result<(), EditError> {
    let busy = || EditError::Busy {
        path: path.to_owned(),
    };
    file.try_lock().map_err(|refused| match refused {
        TryLockError::WouldBlock => busy(),
        TryLockError::Error(cause) => io_fault(path, cause),
    })?;
    if still_named(file, target).map_err(|cause| io_fault(path, cause))? {
        Ok(())
    } else {
        Err(busy())
    }
}

/// Whether the open `file` is still the file at `target`.
#[cfg(unix)]
fn still_named(file: &File, target: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;

    Ok(file.metadata()?.ino() == fs::symlink_metadata(target)?.ino())
}

/// No file identity to compare here, so an edit is refused, never risked.
#[cfg(not(unix))]
fn still_named(_file: &File, _target: &Path) -> io::Result<bool> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Replaces `target` whole with `text` under `mode`: a reader, or a kill, sees
/// the old file or the new one and never a part, and a hard link of the old file
/// is left as it was. The temporary file is made in the held `directory`, so a
/// directory swapped for a link meanwhile receives nothing. The new file is
/// held from before it is named until the returned handle drops.
pub(super) fn replace(
    directory: &Dir,
    mode: Permissions,
    target: &Path,
    text: &str,
) -> io::Result<Option<File>> {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name().and_then(OsStr::to_str))
    else {
        return Err(io::Error::from(io::ErrorKind::InvalidFilename));
    };
    let mut kept = None;
    crate::replace_created(parent, name, text.as_bytes(), |temporary| {
        let made = directory.create_new(temporary)?;
        made.set_permissions(mode)?;
        made.lock()?;
        kept = Some(made.try_clone()?);
        Ok(made)
    })
    .map_err(|(_, cause)| cause)?;
    Ok(kept)
}

/// `content` with the one place `old` sits replaced by `new`: `Ok(None)` when it
/// sits nowhere, and the candidate lines when it sits in more than one. The
/// exact bytes decide first, then whitespace-trimmed lines, then the block the
/// first and last lines of `old` bound.
fn anchored(content: &str, old: &str, new: &str) -> Result<Option<String>, Vec<usize>> {
    let exact = exact_offsets(content, old);
    match *exact.as_slice() {
        [] => {}
        [_] => return Ok(Some(content.replacen(old, new, 1))),
        _ => return Err(exact.iter().map(|&at| line_of(content, at)).collect()),
    }
    let rows: Vec<&str> = content.split_inclusive('\n').collect();
    let wanted: Vec<&str> = old.lines().collect();
    let Some(core) = core_of(&wanted) else {
        return Ok(None);
    };
    let mut span = wanted.len();
    let mut starts = places(&rows, span, |window| {
        window
            .iter()
            .zip(&wanted)
            .all(|(have, want)| have.trim() == want.trim())
    });
    if starts.is_empty() {
        span = core.len();
        let bounds = ends(core);
        starts = places(&rows, span, |window| ends(window) == bounds);
    }
    match *starts.as_slice() {
        [] => Ok(None),
        [start] => Ok(spliced(&rows, start, span, new)),
        _ => Err(starts.iter().map(|start| start.saturating_add(1)).collect()),
    }
}

/// The byte offsets where `old` occurs in `content`, overlapping ones too, at
/// most [`MAX_CANDIDATES`]; none for an empty `old`.
fn exact_offsets(content: &str, old: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let Some(first) = old.chars().next() else {
        return found;
    };
    let mut from = 0_usize;
    while found.len() < MAX_CANDIDATES
        && let Some(at) = content.get(from..).and_then(|rest| rest.find(old))
    {
        let at = from.saturating_add(at);
        found.push(at);
        from = at.saturating_add(first.len_utf8());
    }
    found
}

/// The 1-based line holding byte `offset` of `content`.
fn line_of(content: &str, offset: usize) -> usize {
    content
        .bytes()
        .take(offset)
        .filter(|byte| *byte == b'\n')
        .count()
        .saturating_add(1)
}

/// `lines` from the first that is not blank to the last; `None` when all are.
fn core_of<'a, 'line>(lines: &'a [&'line str]) -> Option<&'a [&'line str]> {
    let held = |line: &&str| !line.trim().is_empty();
    let first = lines.iter().position(held)?;
    let last = lines.iter().rposition(held)?;
    lines.get(first..=last)
}

/// The first and last of `lines`, trimmed: what bounds an anchored block.
fn ends<'line>(lines: &[&'line str]) -> [Option<&'line str>; 2] {
    [lines.first(), lines.last()].map(|line| line.map(|line| line.trim()))
}

/// The 0-based rows where a window of `span` (at least one) rows satisfies
/// `holds`, in file order, at most [`MAX_CANDIDATES`].
pub(super) fn places(rows: &[&str], span: usize, holds: impl Fn(&[&str]) -> bool) -> Vec<usize> {
    rows.windows(span)
        .enumerate()
        .filter(|&(_, window)| holds(window))
        .map(|(start, _)| start)
        .take(MAX_CANDIDATES)
        .collect()
}

/// `rows`, each with its line ending, with the `span` rows at `start` replaced
/// by the lines of `new` (none when it is empty), ending as those rows end.
fn spliced(rows: &[&str], start: usize, span: usize, new: &str) -> Option<String> {
    let end = start.checked_add(span)?;
    let replaced = rows.get(start..end)?;
    let between = if replaced.first()?.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut fresh = new.lines().collect::<Vec<_>>().join(between);
    if !new.is_empty() {
        let last = replaced.last()?;
        fresh.push_str(last.get(last.trim_end_matches(['\r', '\n']).len()..)?);
    }
    Some(
        [
            rows.get(..start)?.concat(),
            fresh,
            rows.get(end..)?.concat(),
        ]
        .concat(),
    )
}

#[cfg(test)]
#[path = "sim_edit.rs"]
mod sim;
