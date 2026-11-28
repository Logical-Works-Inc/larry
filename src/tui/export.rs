//! `/export` and `/recap` (larry#609, ADR 0133): the session on screen as a Markdown file inside
//! its folder, or as a few lines in the frame. Neither calls a model or compacts the transcript.
//!
//! Every text a person, a model or a command wrote goes into a fenced block longer than any
//! backtick run it holds (`CommonMark` 0.31 section 4.5), each control or invisible character
//! shown as a marker. The file is created through held directory descriptors, never through a
//! link and never over a name that exists (CWE-22, CWE-59).

use std::fs::File;
use std::io::{self, Write};
use std::path::{Component, Path};

use lgwks_macros::Error;
use lgwks_std::fs::capability::Dir;

use super::cells::Entry;
use super::sessions::{MAX_STORED_BYTES, Snapshot};
use super::style::{ellipsize, sanitize_line, sanitize_shown};
use super::worker::Receipt;

/// Room kept for the line that counts the entries left out.
const NOTE_BYTES: usize = 128;
/// Most tasks `/recap` names; earlier ones are counted.
const RECAP_ASKS: usize = 5;
/// Columns of a task or an answer `/recap` shows.
const RECAP_COLUMNS: usize = 80;

/// Why `/export` wrote nothing. Each text opens with what the person has to know, so it is
/// read before the chat column wraps it.
#[derive(Debug, Error)]
pub(crate) enum ExportError {
    /// The path is absolute, climbs with `..` or names no file.
    #[error("{0} is outside this folder; /export takes a file inside it.")]
    Outside(String),
    /// The path goes into a git directory.
    #[error("{0} is inside .git; /export never writes there.")]
    Git(String),
    /// A file, a folder or a link already has the name.
    #[error("{0} already exists; /export never overwrites. Name another: /export <file>.")]
    Exists(String),
    /// A folder on the way is missing, is no folder, or is a link.
    #[error("/export never follows a link; the folder of {path} cannot be opened: {cause}")]
    Folder {
        /// The path asked for.
        path: String,
        /// The refusal.
        #[source]
        cause: io::Error,
    },
    /// The file was not created or not written whole.
    #[error("{path} was not written: {cause}")]
    Write {
        /// The path asked for.
        path: String,
        /// The refusal.
        #[source]
        cause: io::Error,
    },
}

/// Writes the session `snapshot` named `name` to `target` inside `folder`, `<name>.md` when no
/// target is given, and returns the lines that say so.
///
/// # Errors
///
/// [`ExportError`]; nothing is left behind unless its text says so.
pub(crate) fn export(
    folder: &Path,
    target: &str,
    name: &str,
    model: &str,
    snapshot: &Snapshot,
) -> Result<Vec<String>, ExportError> {
    let target = if target.is_empty() {
        file_name(name)
    } else {
        target.to_owned()
    };
    let about = format!("name: {name}\nfolder: {}\nmodel: {model}", folder.display());
    let (text, held, left_out) = markdown(&about, snapshot);
    create(folder, &target, text.as_bytes(), store)?;
    let mut lines = vec![format!(
        "exported to {target} ({held} entries, {} receipts).",
        snapshot.receipts.len()
    )];
    if left_out > 0 {
        lines.push(format!(
            "{left_out} older entries did not fit in 4 MiB and are left out."
        ));
    }
    Ok(lines)
}

/// The default file of the session `name`: its ASCII letters and digits, lowercase, the runs
/// between them one `-`.
fn file_name(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return String::from("session.md");
    }
    format!("{}.md", words.join("-").to_ascii_lowercase())
}

/// The file's text: what the session is, the newest entries that fit [`MAX_STORED_BYTES`] in
/// their order, and every receipt digest. Returns it with the entries it holds and the older
/// ones left out.
fn markdown(about: &str, snapshot: &Snapshot) -> (String, usize, usize) {
    let head = format!("# Larry session\n\n{}", block("session", about));
    let tail = block("receipts", &receipts(&snapshot.receipts));
    let mut bytes = head
        .len()
        .saturating_add(tail.len())
        .saturating_add(NOTE_BYTES);
    let total = snapshot
        .entries
        .iter()
        .filter(|entry| !matches!(**entry, Entry::Header(_)))
        .count();
    let kept: Vec<String> = snapshot
        .entries
        .iter()
        .rev()
        .filter_map(entry_block)
        .take_while(|text| {
            bytes = bytes.saturating_add(text.len());
            bytes <= MAX_STORED_BYTES
        })
        .collect();
    let held = kept.len();
    let left_out = total.saturating_sub(held);
    let note = if left_out > 0 {
        format!("{left_out} older entries are left out: this file holds at most 4 MiB.\n\n")
    } else {
        String::new()
    };
    let body: String = kept.into_iter().rev().collect();
    (format!("{head}{note}{body}{tail}"), held, left_out)
}

/// `body` under `## heading`, in a fence no run of backticks inside it can close.
fn block(heading: &str, body: &str) -> String {
    let body = sanitize_shown(body);
    let longest = body
        .split(|character| character != '`')
        .map(str::len)
        .fold(2, usize::max);
    let fence = "`".repeat(longest.saturating_add(1));
    format!("## {heading}\n\n{fence}text\n{body}\n{fence}\n\n")
}

/// The block of one entry; the welcome box has none, the session block says what it said.
fn entry_block(entry: &Entry) -> Option<String> {
    Some(match *entry {
        Entry::Header(_) => return None,
        Entry::User(ref text) => block("task", text),
        Entry::Ran {
            ref command,
            ref status,
            ref output,
            hidden,
            passed,
        } => {
            let earlier = if hidden > 0 {
                format!("{hidden} earlier lines not kept\n")
            } else {
                String::new()
            };
            let heading = if passed { "ran, passed" } else { "ran, failed" };
            let body = format!("$ {command}\n{status}\n{earlier}{}", output.join("\n"));
            block(heading, &body)
        }
        Entry::Declined(ref command) => block("declined, never ran", command),
        Entry::Said(ref text) => block("larry", text),
        Entry::Notice(_, ref text) => block("notice", text),
        Entry::Info(ref rows) => block("info", &rows.join("\n")),
    })
}

/// One line for each receipt, oldest first.
fn receipts(held: &[Receipt]) -> String {
    if held.is_empty() {
        return String::from("none");
    }
    let lines: Vec<String> = held
        .iter()
        .map(|receipt| {
            let verdict = if receipt.accepted {
                "accepted"
            } else {
                "not accepted"
            };
            format!(
                "{} {verdict}, {} run, {} declined, {} interrupted, change {}",
                receipt.digest,
                receipt.commands,
                receipt.declined,
                receipt.interrupted,
                receipt.handoff
            )
        })
        .collect();
    lines.join("\n")
}

/// Creates `target` beneath `folder` holding `bytes`. Each folder on the way is opened from the
/// one before it and never through a link, and the name must be new. `fill` writes the bytes; a
/// write that fails removes the file it made.
fn create(
    folder: &Path,
    target: &str,
    bytes: &[u8],
    fill: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<(), ExportError> {
    let path = || target.to_owned();
    let mut names = parts(target)?;
    let Some(name) = names.pop() else {
        return Err(ExportError::Outside(path()));
    };
    let closed = |cause| ExportError::Folder {
        path: path(),
        cause,
    };
    let mut directory = Dir::open(folder).map_err(closed)?;
    for part in names {
        directory = directory.open_subdir(part).map_err(closed)?;
    }
    let mut file = directory.create_new(name).map_err(|cause| {
        if cause.kind() == io::ErrorKind::AlreadyExists {
            ExportError::Exists(path())
        } else {
            ExportError::Write {
                path: path(),
                cause,
            }
        }
    })?;
    let Err(cause) = fill(&mut file, bytes) else {
        return Ok(());
    };
    let cause = match directory.remove_file(name) {
        Ok(()) => cause,
        Err(left) => io::Error::new(left.kind(), format!("{cause}; a part file stays: {left}")),
    };
    Err(ExportError::Write {
        path: path(),
        cause,
    })
}

/// The names of `target` beneath the folder: every part a plain name, none of them `.git` in
/// any case (larry#335).
fn parts(target: &str) -> Result<Vec<&str>, ExportError> {
    let mut names = Vec::new();
    for part in Path::new(target).components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) => names.extend(name.to_str()),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExportError::Outside(target.to_owned()));
            }
        }
    }
    if names.iter().any(|name| name.eq_ignore_ascii_case(".git")) {
        return Err(ExportError::Git(target.to_owned()));
    }
    Ok(names)
}

/// Writes `bytes` and syncs them to the device.
fn store(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

/// The session in a few lines, changing nothing: the counts, the newest [`RECAP_ASKS`] tasks,
/// what Larry last answered and the receipts.
pub(crate) fn recap(snapshot: &Snapshot) -> Vec<String> {
    let mut asks = Vec::new();
    let mut answer = None;
    let (mut passed, mut failed, mut declined) = (0_usize, 0_usize, 0_usize);
    for entry in &snapshot.entries {
        match *entry {
            Entry::User(ref text) => {
                asks.push(text);
                answer = None;
            }
            Entry::Ran { passed: true, .. } => passed = passed.saturating_add(1),
            Entry::Ran { .. } => failed = failed.saturating_add(1),
            Entry::Declined(_) => declined = declined.saturating_add(1),
            Entry::Said(ref text) => answer = Some(text),
            Entry::Header(_) | Entry::Notice(..) | Entry::Info(_) => {}
        }
    }
    if asks.is_empty() {
        return vec![String::from(
            "nothing to recap: no task in this session yet.",
        )];
    }
    let mut lines = vec![format!(
        "recap: {}, {} ({passed} passed, {failed} failed), {declined} declined.",
        counted(asks.len(), "task"),
        counted(passed.saturating_add(failed), "command"),
    )];
    let earlier = asks.len().saturating_sub(RECAP_ASKS);
    if earlier > 0 {
        lines.push(format!("{} before these.", counted(earlier, "task")));
    }
    lines.extend(
        asks.iter()
            .skip(earlier)
            .map(|ask| format!("asked: {}", one_line(ask))),
    );
    lines.push(match answer {
        Some(text) => format!("last answer: {}", one_line(text)),
        None => String::from("the newest task has no answer."),
    });
    if let Some(newest) = snapshot.receipts.last() {
        let accepted = snapshot
            .receipts
            .iter()
            .filter(|receipt| receipt.accepted)
            .count();
        lines.push(format!(
            "{}, {accepted} accepted; newest {}, change {}.",
            counted(snapshot.receipts.len(), "receipt"),
            newest.digest,
            newest.handoff
        ));
    }
    lines
}

/// `count` of `noun`, plural for any count but one.
fn counted(count: usize, noun: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{plural}")
}

/// `text` as one safe line of at most [`RECAP_COLUMNS`] columns.
fn one_line(text: &str) -> String {
    ellipsize(&sanitize_line(&text.replace('\n', " ")), RECAP_COLUMNS)
}

#[cfg(test)]
#[path = "sim_export.rs"]
mod tests;
