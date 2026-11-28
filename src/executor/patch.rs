//! A session's patch (larry#624): the Codex `apply_patch` shape (openai/codex
//! `apply_patch.lark`) over several files, all of them or none. `*** Add File: `
//! carries `+` lines, `*** Delete File: ` nothing, and `*** Update File: `, with
//! an optional `*** Move to: `, carries hunks of space, `-` and `+` lines. A
//! hunk names no line number: it sits after the lines its `@@ ` anchors name,
//! with no anchor in the one place its lines sit, and `*** End of File` pins it
//! to the end. Every file the patch changes is opened, held and checked against
//! its digest as an edit is ([`held`]), and planned before anything is written;
//! a write that fails puts every written file back. A kill between two writes
//! leaves the files already written: each file is whole, the set may not be.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, Permissions};
use std::io;
use std::iter::{Peekable, Zip};
use std::ops::RangeFrom;
use std::path::{Path, PathBuf};
use std::str::Split;

use lgwks_std::fs::capability::Dir;

use super::edit::{EditError, MAX_FILE_BYTES, digest_of, held, io_fault, open, places, replace};

/// Most files one patch names: with the reply bound on the patch text, one
/// report record stays under 12 KiB.
const MAX_PATCH_FILES: usize = 16;

/// How a hunk line may differ from the file's line, tried in this order.
const LEVELS: [fn(&str, &str) -> bool; 3] = [
    |have, want| have == want,
    |have, want| have.trim_end() == want.trim_end(),
    |have, want| have.trim() == want.trim(),
];

/// A patch as a session reply carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Patch {
    /// The text from `*** Begin Patch` to `*** End Patch`.
    pub(super) text: String,
    /// The digest of every file it changes or removes, as the model last saw it.
    pub(super) expect_digests: BTreeMap<String, String>,
}

/// What a patch did to one file: its digest on both sides, none on the side
/// where the file is not there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileReceipt {
    /// The repo-relative path.
    pub(super) path: String,
    /// The BLAKE3 hex of the file before the patch.
    pub(super) before_digest: Option<String>,
    /// The BLAKE3 hex of the file after it.
    pub(super) after_digest: Option<String>,
}

/// The patch lines not yet parsed, each with its 1-based number.
type Lines<'a> = Peekable<Zip<Split<'a, char>, RangeFrom<usize>>>;

/// One hunk of an update.
#[derive(Default)]
struct Hunk<'a> {
    /// The patch line it starts at.
    line: usize,
    /// The lines its `@@ ` headers name, each found after the one before.
    anchors: Vec<&'a str>,
    /// Its lines: the marker (a space, `-` or `+`) and the text.
    body: Vec<(char, &'a str)>,
    /// Whether `*** End of File` closed it.
    at_end: bool,
}

/// What a patch does to one file.
enum Action<'a> {
    /// Creates it with this text.
    Add(String),
    /// Removes it.
    Delete,
    /// Applies the hunks, and moves the file when it names a new path.
    Update {
        /// The path of `*** Move to: `.
        move_to: Option<&'a str>,
        /// The hunks in patch order.
        hunks: Vec<Hunk<'a>>,
    },
}

/// A refusal of the patch's shape at `line`.
fn syntax(line: usize, detail: &'static str) -> EditError {
    EditError::PatchSyntax { line, detail }
}

/// The files `text` names and what it does to each, in patch order.
fn parse(text: &str) -> Result<Vec<(&str, Action<'_>)>, EditError> {
    let text = text.trim_end();
    let total = text.split('\n').count();
    let mut lines: Lines<'_> = text.split('\n').zip(1..).peekable();
    if lines.next().map(|(line, _)| line.trim_end()) != Some("*** Begin Patch") {
        return Err(syntax(1, "the patch opens with `*** Begin Patch`"));
    }
    let mut files = Vec::new();
    let mut named = BTreeSet::new();
    loop {
        let Some((line, number)) = lines.next() else {
            return Err(syntax(total, "the patch closes with `*** End Patch`"));
        };
        if line.trim_end() == "*** End Patch" {
            break;
        }
        let (path, action) = if let Some(path) = line.strip_prefix("*** Add File: ") {
            (path, addition(&mut lines, number)?)
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            (path, Action::Delete)
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            (path, update(&mut lines, number)?)
        } else {
            return Err(syntax(number, "a file header or `*** End Patch` goes here"));
        };
        let path = path.trim();
        let moved = match action {
            Action::Update { move_to, .. } => move_to,
            Action::Add(_) | Action::Delete => None,
        };
        for name in std::iter::once(path).chain(moved) {
            if !named.insert(name) {
                return Err(syntax(number, "the patch names one file twice"));
            }
        }
        if files.len() == MAX_PATCH_FILES {
            return Err(syntax(number, "the patch names more than 16 files"));
        }
        files.push((path, action));
    }
    if let Some((_, number)) = lines.next() {
        return Err(syntax(number, "nothing follows `*** End Patch`"));
    }
    if files.is_empty() {
        return Err(syntax(total, "the patch names no file"));
    }
    Ok(files)
}

/// The `+` lines of the addition whose header is `header`, as the file.
fn addition<'a>(lines: &mut Lines<'a>, header: usize) -> Result<Action<'a>, EditError> {
    let mut text = String::new();
    while let Some(line) = lines.peek().and_then(|&(line, _)| line.strip_prefix('+')) {
        text.push_str(line);
        text.push('\n');
        lines.next();
    }
    if text.is_empty() {
        return Err(syntax(header, "the addition holds no `+` line"));
    }
    Ok(Action::Add(text))
}

/// The last of `hunks` when `takes` it, else a new one that starts at `line`.
fn hunk_at<'h, 'a>(
    hunks: &'h mut Vec<Hunk<'a>>,
    line: usize,
    takes: impl FnOnce(&Hunk<'a>) -> bool,
) -> Option<&'h mut Hunk<'a>> {
    if !hunks.last().is_some_and(takes) {
        hunks.push(Hunk {
            line,
            ..Hunk::default()
        });
    }
    hunks.last_mut()
}

/// The move and the hunks of the update whose header is `header`.
fn update<'a>(lines: &mut Lines<'a>, header: usize) -> Result<Action<'a>, EditError> {
    let move_to = lines
        .peek()
        .and_then(|&(line, _)| line.strip_prefix("*** Move to: "))
        .map(str::trim);
    if move_to.is_some() {
        lines.next();
    }
    let mut hunks: Vec<Hunk<'a>> = Vec::new();
    while let Some((line, number)) = lines
        .next_if(|&(line, _)| !line.starts_with("*** ") || line.trim_end() == "*** End of File")
    {
        if line.trim_end() == "*** End of File" {
            let open = hunks
                .last_mut()
                .filter(|hunk| !hunk.body.is_empty() && !hunk.at_end);
            open.ok_or_else(|| syntax(number, "`*** End of File` closes a hunk"))?
                .at_end = true;
        } else if line == "@@" || line.starts_with("@@ ") {
            if let Some(hunk) = hunk_at(&mut hunks, number, |hunk| hunk.body.is_empty()) {
                hunk.anchors
                    .extend(line.get(3..).filter(|anchor| !anchor.trim().is_empty()));
            }
        } else {
            let marked = [' ', '-', '+']
                .into_iter()
                .find_map(|marker| line.strip_prefix(marker).map(|text| (marker, text)));
            // An empty line is a blank line the file holds.
            let Some(body) = marked.or_else(|| line.is_empty().then_some((' ', ""))) else {
                return Err(syntax(
                    number,
                    "a hunk line opens with a space, `-`, `+` or `@@`",
                ));
            };
            if let Some(hunk) = hunk_at(&mut hunks, number, |hunk| !hunk.at_end) {
                hunk.body.push(body);
            }
        }
    }
    if let Some(empty) = hunks.iter().find(|hunk| hunk.body.is_empty()) {
        return Err(syntax(empty.line, "the hunk holds no line"));
    }
    if hunks.is_empty() && move_to.is_none() {
        return Err(syntax(header, "the update holds no hunk"));
    }
    Ok(Action::Update { move_to, hunks })
}

/// The rows at or after `from` where the lines `wanted` (at least one) sit, at
/// the first level that finds any; only the file's end when `at_end`.
fn seek(rows: &[&str], from: usize, wanted: &[&str], at_end: bool) -> Vec<usize> {
    let from = if at_end {
        from.max(rows.len().saturating_sub(wanted.len()))
    } else {
        from
    };
    let Some(rest) = rows.get(from..) else {
        return Vec::new();
    };
    let found = LEVELS.iter().map(|level| {
        places(rest, wanted.len(), |window| {
            window
                .iter()
                .zip(wanted)
                .all(|(have, want)| level(have.trim_end_matches(['\r', '\n']), want))
        })
    });
    found
        .filter(|starts| !starts.is_empty())
        .take(1)
        .flatten()
        .map(|start| start.saturating_add(from))
        .collect()
}

/// `content` with every hunk applied, each sought after the one before. A kept
/// line keeps the file's own bytes, an added one ends as the file's first line
/// ends, and a file with no final newline stays so.
fn updated(path: &str, content: &str, hunks: &[Hunk<'_>]) -> Result<String, EditError> {
    let rows: Vec<&str> = content.split_inclusive('\n').collect();
    let crlf = rows.first().is_some_and(|row| row.ends_with("\r\n"));
    let newline = if crlf { "\r\n" } else { "\n" };
    let whole = |row: &str| {
        if row.ends_with('\n') {
            row.to_owned()
        } else {
            [row, newline].concat()
        }
    };
    let mut out: Vec<String> = Vec::new();
    let mut tail: Vec<String> = Vec::new();
    let mut copied = 0_usize;
    let mut from = 0_usize;
    for hunk in hunks {
        let mismatch = || EditError::PatchMismatch {
            path: path.to_owned(),
            line: hunk.line,
        };
        for anchor in &hunk.anchors {
            let found = seek(&rows, from, &[anchor], false);
            from = found.first().ok_or_else(mismatch)?.saturating_add(1);
        }
        let mut body = hunk.body.as_slice();
        loop {
            let wanted: Vec<&str> = body
                .iter()
                .filter(|line| line.0 != '+')
                .map(|line| line.1)
                .collect();
            if wanted.is_empty() {
                // A hunk of additions alone lands at the end of the file, as Codex applies it.
                tail.extend(body.iter().map(|line| [line.1, newline].concat()));
                break;
            }
            let found = seek(&rows, from, &wanted, hunk.at_end);
            let Some(&at) = found.first() else {
                // A blank line the model left after its hunk is not part of the file.
                let Some((&(' ', ""), shorter)) = body.split_last() else {
                    return Err(mismatch());
                };
                body = shorter;
                continue;
            };
            if found.len() > 1 && hunk.anchors.is_empty() {
                return Err(EditError::Ambiguous {
                    path: path.to_owned(),
                    candidates: found.iter().map(|start| start.saturating_add(1)).collect(),
                });
            }
            out.extend(rows.iter().take(at).skip(copied).map(|row| whole(row)));
            let mut cursor = at;
            for &(marker, text) in body {
                if marker == '+' {
                    out.push([text, newline].concat());
                    continue;
                }
                if marker == ' ' {
                    out.extend(rows.get(cursor).map(|row| whole(row)));
                }
                cursor = cursor.saturating_add(1);
            }
            copied = cursor;
            from = cursor;
            break;
        }
    }
    out.extend(rows.iter().skip(copied).map(|row| whole(row)));
    out.append(&mut tail);
    let mut text = out.concat();
    if !content.is_empty() && !content.ends_with('\n') {
        let ending = if text.ends_with("\r\n") {
            2
        } else {
            usize::from(text.ends_with('\n'))
        };
        text.truncate(text.len().saturating_sub(ending));
    }
    let len = u64::try_from(text.len()).map_err(|cause| io_fault(path, io::Error::other(cause)))?;
    if len > MAX_FILE_BYTES {
        return Err(EditError::TooLarge {
            path: path.to_owned(),
            len,
        });
    }
    Ok(text)
}

/// One file the patch writes or removes.
struct Change {
    /// The repo-relative path.
    path: String,
    /// The file as it is, held until the patch ends, with its text; none when
    /// the patch creates it.
    before: Option<(File, String)>,
    /// What to leave in it; none removes it.
    after: Option<String>,
    /// The mode a moved file keeps.
    mode: Option<Permissions>,
    /// Whether this patch made the file, so a rollback may remove it.
    created: bool,
}

impl Change {
    /// A change of `path` from `before` to `after`.
    fn new(path: &str, before: Option<(File, String)>, after: Option<String>) -> Self {
        Self {
            path: path.to_owned(),
            before,
            after,
            mode: None,
            created: false,
        }
    }

    /// Writes or removes the file, noting each directory it `made` and keeping
    /// in `kept` what holds the new file until the patch ends.
    fn write(
        &mut self,
        root: &Path,
        made: &mut Vec<PathBuf>,
        kept: &mut Vec<File>,
    ) -> io::Result<()> {
        let (directory, name) = parents(root, &self.path, made)?;
        let Some(ref after) = self.after else {
            return directory.remove_file(name);
        };
        let mode = if let Some((ref file, _)) = self.before {
            file.metadata()?.permissions()
        } else {
            // The name is claimed first, so a file that appeared is never replaced.
            let claimed = directory.create_new(name)?;
            self.created = true;
            claimed.lock()?;
            let own = claimed.metadata()?.permissions();
            kept.push(claimed);
            match self.mode {
                Some(ref moved) => moved.clone(),
                None => own,
            }
        };
        kept.extend(replace(&directory, mode, &root.join(&self.path), after)?);
        Ok(())
    }

    /// Puts the file back as it was before [`Self::write`].
    fn restore(&self, root: &Path) -> io::Result<()> {
        if self.before.is_none() && !self.created {
            return Ok(());
        }
        let (directory, name) = parents(root, &self.path, &mut Vec::new())?;
        let Some((ref file, ref text)) = self.before else {
            return directory.remove_file(name);
        };
        let mode = file.metadata()?.permissions();
        replace(&directory, mode, &root.join(&self.path), text).map(drop)
    }

    /// The receipt of the change once written.
    fn receipt(&self) -> FileReceipt {
        FileReceipt {
            path: self.path.clone(),
            before_digest: self.before.as_ref().map(|before| digest_of(&before.1)),
            after_digest: self.after.as_deref().map(digest_of),
        }
    }
}

/// A file or folder name as text.
fn utf8(name: Option<&OsStr>) -> io::Result<&str> {
    name.and_then(OsStr::to_str)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidFilename))
}

/// The directory holding `path` under `root`, reached through no symlink, with
/// every directory missing on the way made and noted in `made`, and the name of
/// `path` in it.
fn parents<'a>(root: &Path, path: &'a str, made: &mut Vec<PathBuf>) -> io::Result<(Dir, &'a str)> {
    let relative = Path::new(path);
    let mut directory = Dir::open(root)?;
    let mut at = root.to_path_buf();
    for part in relative.parent().into_iter().flat_map(Path::components) {
        let folder = utf8(Some(part.as_os_str()))?;
        at.push(folder);
        match directory.create_dir(folder) {
            Ok(()) => made.push(at.clone()),
            Err(cause) if cause.kind() == io::ErrorKind::AlreadyExists => {}
            Err(cause) => return Err(cause),
        }
        directory = directory.open_subdir(folder)?;
    }
    Ok((directory, utf8(relative.file_name())?))
}

/// The file `path` held for the patch, still with the digest `expect` names.
fn checked(
    root: &Path,
    path: &str,
    expect: &mut BTreeMap<&str, &str>,
) -> Result<(File, String), EditError> {
    let digest = expect.remove(path).ok_or_else(|| EditError::NoDigest {
        path: path.to_owned(),
    })?;
    let (_, file, content) = held(root, path, digest, &mut |_| Ok(()))?;
    Ok((file, content))
}

/// A change that creates `path` with `text`, refused when anything is there.
fn creation(root: &Path, path: &str, text: String) -> Result<Change, EditError> {
    match open(root, path) {
        Ok(_) => Err(EditError::Exists {
            path: path.to_owned(),
        }),
        Err(EditError::Io { cause, .. }) if cause.kind() == io::ErrorKind::NotFound => {
            Ok(Change::new(path, None, Some(text)))
        }
        Err(refused) => Err(refused),
    }
}

/// Plans what `action` does to `path`: nothing is written.
fn stage(
    root: &Path,
    path: &str,
    action: Action<'_>,
    expect: &mut BTreeMap<&str, &str>,
    changes: &mut Vec<Change>,
) -> Result<(), EditError> {
    match action {
        Action::Add(text) => changes.push(creation(root, path, text)?),
        Action::Delete => {
            let before = checked(root, path, expect)?;
            changes.push(Change::new(path, Some(before), None));
        }
        Action::Update { move_to, hunks } => {
            let (file, content) = checked(root, path, expect)?;
            let mut after = Some(updated(path, &content, &hunks)?);
            if let Some(to) = move_to
                && let Some(text) = after.take()
            {
                let mut moved = creation(root, to, text)?;
                let metadata = file.metadata().map_err(|cause| io_fault(path, cause))?;
                moved.mode = Some(metadata.permissions());
                changes.push(moved);
            }
            changes.push(Change::new(path, Some((file, content)), after));
        }
    }
    Ok(())
}

/// What a simulation does before the write of a change; production does nothing.
type Seam<'a> = dyn FnMut(usize) -> io::Result<()> + 'a;

/// Writes every change in order. A failure puts the ones written back and
/// removes the directories made; what could not be put back is named.
fn commit(root: &Path, changes: &mut [Change], seam: &mut Seam<'_>) -> Result<(), EditError> {
    let mut made = Vec::new();
    // Holds every new file until the patch ends, written back or not.
    let mut kept = Vec::new();
    let failed = changes.iter_mut().enumerate().find_map(|(at, change)| {
        let written = seam(at).and_then(|()| change.write(root, &mut made, &mut kept));
        written
            .err()
            .map(|cause| (at, io_fault(&change.path, cause)))
    });
    let Some((at, cause)) = failed else {
        return Ok(());
    };
    let mut left: Vec<String> = changes
        .iter()
        .take(at.saturating_add(1))
        .rev()
        .filter(|change| change.restore(root).is_err())
        .map(|change| change.path.clone())
        .collect();
    left.extend(
        made.iter()
            .rev()
            .filter(|folder| fs::remove_dir(folder).is_err())
            .map(|folder| folder.display().to_string()),
    );
    if left.is_empty() {
        return Err(cause);
    }
    Err(EditError::PartlyApplied {
        left,
        cause: Box::new(cause),
    })
}

/// Applies `patch` under `root`, all of it or none: a receipt for each file it
/// wrote or removed, in write order.
pub(super) fn apply_patch(root: &Path, patch: &Patch) -> Result<Vec<FileReceipt>, EditError> {
    apply_patch_with(root, patch, &mut |_| Ok(()))
}

/// [`apply_patch`] with its seam, for a simulation.
fn apply_patch_with(
    root: &Path,
    patch: &Patch,
    seam: &mut Seam<'_>,
) -> Result<Vec<FileReceipt>, EditError> {
    let mut expect: BTreeMap<&str, &str> = patch
        .expect_digests
        .iter()
        .map(|(path, digest)| (path.as_str(), digest.as_str()))
        .collect();
    let mut changes = Vec::new();
    for (path, action) in parse(&patch.text)? {
        stage(root, path, action, &mut expect, &mut changes)?;
    }
    if let Some(path) = expect.keys().next() {
        return Err(EditError::UnusedDigest {
            path: (*path).to_owned(),
        });
    }
    commit(root, &mut changes, seam)?;
    Ok(changes.iter().map(Change::receipt).collect())
}

#[cfg(test)]
#[path = "sim_patch.rs"]
mod sim;
