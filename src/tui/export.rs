//! `/export` and `/recap` (larry#609, ADR 0133): the session on screen as a Markdown file inside
//! its folder, or as a few lines in the frame. Neither calls a model or compacts the transcript.
//!
//! Every text a person, a model or a command wrote goes into a fenced block longer than any
//! backtick run it holds (`CommonMark` 0.31 section 4.5), each control or invisible character
//! shown as a marker. The file is made through held directory descriptors, never through a
//! link and never over a name that exists (CWE-22, CWE-59), and it takes its name only once it
//! is whole: a killed process leaves at most a part file, which the next export of that folder
//! sweeps. [`crate::replace_file`] is not this write: its rename replaces what is there.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Component, Path};

use lgwks_macros::Error;
use lgwks_std::fs::capability::{Dir, OpenFlags};
use lgwks_std::id::Uuid;
use rustix::fs::{AtFlags, fsync, linkat};

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
/// How the name of a part file starts: it holds an export until the export takes its own name.
const PART_PREFIX: &str = ".larry-export-";
/// How the name of a part file ends.
const PART_SUFFIX: &str = ".part";
/// Most entries of a folder one sweep reads.
const SWEEP_ENTRIES: usize = 65_536;
/// Tries at a write whose part file another terminal's sweep took before it was locked.
const PUBLISH_TRIES: u8 = 3;

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
    /// The name is one a part file takes, which a later export would sweep.
    #[error("{0} is a name /export keeps for its part files. Name another: /export <file>.")]
    Part(String),
    /// A folder on the way is missing, is no folder, or is a link.
    #[error("/export never follows a link; the folder of {path} cannot be opened: {cause}")]
    Folder {
        /// The path asked for.
        path: String,
        /// The refusal.
        #[source]
        cause: io::Error,
    },
    /// The file was not written whole, so nothing has its name.
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
/// [`ExportError`]; nothing has the target's name that did not have it before.
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
    let untidied = create(folder, &target, text.as_bytes(), store)?;
    let mut lines = vec![format!(
        "exported to {target} ({held} entries, {} receipts).",
        snapshot.receipts.len()
    )];
    if left_out > 0 {
        lines.push(format!(
            "{left_out} older entries did not fit in 4 MiB and are left out."
        ));
    }
    if let Some(cause) = untidied {
        lines.push(format!(
            "the folder may keep a {PART_PREFIX}*{PART_SUFFIX} file: {cause}"
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

/// Creates `target` beneath `folder` holding `bytes`, whole or not at all, after sweeping the
/// part files its folder was left with. Each folder on the way is opened from the one before it
/// and never through a link, and the name must be new. `fill` writes the bytes. Returns why a
/// part file or the folder's sync was left undone.
fn create(
    folder: &Path,
    target: &str,
    bytes: &[u8],
    mut fill: impl FnMut(&mut File, &[u8]) -> io::Result<()>,
) -> Result<Option<io::Error>, ExportError> {
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
    if is_part(name) {
        return Err(ExportError::Part(path()));
    }
    let swept = sweep(&directory);
    match written(&directory, name, bytes, &mut fill) {
        Ok(untidied) => Ok(untidied.or(swept.err())),
        Err(cause) if cause.kind() == io::ErrorKind::AlreadyExists => {
            Err(ExportError::Exists(path()))
        }
        Err(cause) => Err(ExportError::Write {
            path: path(),
            cause,
        }),
    }
}

/// Whether `name` is the name of a part file.
fn is_part(name: &str) -> bool {
    name.starts_with(PART_PREFIX) && name.ends_with(PART_SUFFIX)
}

/// Removes the part files killed exports left in `directory`: regular files with a part name
/// that no running export holds locked. A link, a folder and every other name stay.
fn sweep(directory: &Dir) -> io::Result<()> {
    for entry in rustix::fs::Dir::read_from(directory.as_fd())?.take(SWEEP_ENTRIES) {
        let entry = entry?;
        // A name that is not UTF-8 is none this module made.
        let Ok(name) = entry.file_name().to_str() else {
            continue;
        };
        if !is_part(name) {
            continue;
        }
        // The open refuses a link and anything but a regular file.
        if let Ok(file) = directory.open_entry(name, OpenFlags::read())
            && file.try_lock().is_ok()
        {
            directory.remove_file(name)?;
        }
    }
    Ok(())
}

/// [`publish`], again while another terminal's sweep takes the part file between its creation
/// and its lock.
fn written(
    directory: &Dir,
    name: &str,
    bytes: &[u8],
    fill: &mut impl FnMut(&mut File, &[u8]) -> io::Result<()>,
) -> io::Result<Option<io::Error>> {
    for _ in 1..PUBLISH_TRIES {
        match publish(directory, name, bytes, fill) {
            Err(cause)
                if matches!(
                    cause.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::WouldBlock
                ) => {}
            outcome => return outcome,
        }
    }
    publish(directory, name, bytes, fill)
}

/// One try: `bytes` into a new part file held locked while it is filled, then a hard link of it
/// to `name`, which fails when the name is taken and so never replaces (POSIX `link(2)`). The
/// part file goes either way. Returns why that removal or the folder's sync failed once the
/// name was published.
fn publish(
    directory: &Dir,
    name: &str,
    bytes: &[u8],
    fill: &mut impl FnMut(&mut File, &[u8]) -> io::Result<()>,
) -> io::Result<Option<io::Error>> {
    let id = Uuid::new_v7().map_err(io::Error::other)?;
    let part = format!("{PART_PREFIX}{id}{PART_SUFFIX}");
    let mut file = directory.create_new(&part)?;
    let linked = file
        .try_lock()
        .map_err(io::Error::from)
        .and_then(|()| fill(&mut file, bytes))
        .and_then(|()| {
            let held = directory.as_fd();
            Ok(linkat(held, &part, held, name, AtFlags::empty())?)
        });
    let tidied = directory
        .remove_file(&part)
        .and_then(|()| Ok(fsync(directory.as_fd())?));
    linked.map(|()| tidied.err())
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
