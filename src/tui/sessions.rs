//! TUI sessions that survive a restart (larry#256, ADR 0130): each terminal
//! session is one row in the state database the executor already owns
//! (`executor.db`, never a stray file), holding its name, folder, model and the
//! whole transcript plus every receipt digest seen. `/resume` lists the rows for
//! this folder and continues one, `/fork` copies one into a new row, `/new`
//! starts fresh in the same process, `/rename` names it, `/export` writes the
//! transcript plus every receipt digest as Markdown, and `/recap` summarises
//! without compacting anything.
//!
//! The store layer ([`create`], [`list`], [`load`], [`rename`], [`save`],
//! [`fork`]) is a thin typed shell over the ledger's `tui_sessions` table: the
//! ledger owns the SQL, this module owns the snapshot shape, the name rule, the
//! Markdown and the recap. A snapshot is the live frame's own types
//! ([`Entry`], [`Pane`], [`Receipt`]) as JSON through `lgwks_std::json`, at
//! most [`MAX_STORED_BYTES`] per column, so one transcript can never fill the
//! store; at most [`MAX_SESSIONS`] rows exist, so listing stays a scan of a
//! bounded table. One home holds the transcript shape: no mirror types, so a
//! row always reads back what the frame showed.

use lgwks_std::json::{self, Deserialize, Serialize};

use super::cells::Entry;
use super::pane::Pane;
use super::worker::Receipt;
use crate::executor::{ExecError, Ledger, SessionStoreError};

/// Most TUI sessions one store holds; creating past it is refused typed, so
/// listing stays a scan of a bounded table (the 10,000-row sim proves it).
pub(crate) const MAX_SESSIONS: u32 = 10_000;
/// Most characters a session name holds.
pub(crate) const MAX_NAME_CHARS: usize = 64;
/// Most bytes one stored column (transcript or receipts) holds.
pub(crate) const MAX_STORED_BYTES: usize = 4_194_304;
/// Most receipt digests one snapshot remembers; the pane shows the newest.
pub(crate) const MAX_RECEIPTS_SEEN: usize = 256;
/// Most rows one listing returns.
pub(crate) const LIST_LIMIT: i64 = 10_000;

/// One TUI session, as stored: the transcript, the pane and every receipt
/// digest the session has seen, oldest first, in the frame's own types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub(crate) struct Snapshot {
    /// The transcript, oldest first.
    pub(crate) entries: Vec<Entry>,
    /// The artifact pane.
    pub(crate) pane: Pane,
    /// Every receipt digest seen, oldest first, at most [`MAX_RECEIPTS_SEEN`].
    pub(crate) receipts: Vec<Receipt>,
}

impl Snapshot {
    /// An empty snapshot: no entries, an empty pane, no receipt. The store
    /// sims start from it; the driver always snapshots a live frame.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
            pane: Pane::new(),
            receipts: Vec::new(),
        }
    }

    /// The snapshot as JSON, refused typed when it cannot be encoded.
    pub(crate) fn to_json(&self) -> Result<String, SessionStoreError> {
        json::to_string(self).map_err(SessionStoreError::Encode)
    }

    /// The snapshot `bytes` hold, or the typed damage refusal naming `id`.
    pub(crate) fn from_json(id: &str, bytes: &[u8]) -> Result<Self, SessionStoreError> {
        json::from_slice(bytes).map_err(|cause| SessionStoreError::Damaged {
            id: id.to_owned(),
            cause,
        })
    }
}

/// The name the person gave, trimmed: 1 to [`MAX_NAME_CHARS`] characters, no
/// control character (a newline would break the `/resume` listing and the
/// export file name).
///
/// # Errors
///
/// [`SessionStoreError::BadName`] naming the refused text.
pub(crate) fn validate_name(name: &str) -> Result<String, SessionStoreError> {
    let kept = name.trim().to_owned();
    if kept.is_empty()
        || kept.chars().count() > MAX_NAME_CHARS
        || kept.chars().any(|character| character.is_control())
    {
        return Err(SessionStoreError::BadName {
            name: name.to_owned(),
        });
    }
    Ok(kept)
}

/// A fresh session id: a UUID version 7, so no two sessions share an id across
/// processes and a fixed process id in a container cannot mint a killed
/// session's id (larry#182 defect 3, ADR 0037).
///
/// # Errors
///
/// [`ExecError::Id`] when the OS entropy or the clock refuses.
pub(crate) fn mint_id() -> Result<String, ExecError> {
    lgwks_std::id::Uuid::new_v7()
        .map(|id| id.to_string())
        .map_err(ExecError::Id)
}

/// The export file stem for `name`: ASCII letters and digits kept, anything
/// else one `_`, cut to [`MAX_NAME_CHARS`] bytes on a character boundary.
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if out.len() >= MAX_NAME_CHARS {
            break;
        }
        if character.is_ascii_alphanumeric() {
            out.push(character);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        String::from("session")
    } else {
        trimmed
    }
}

/// Creates a session row for `folder` showing `model`, holding `snapshot`, and
/// returns its id.
///
/// # Errors
///
/// [`SessionStoreError::Full`] past [`MAX_SESSIONS`] rows, [`SessionStoreError::BadName`]
/// for a refused name, [`SessionStoreError::TooLarge`] past [`MAX_STORED_BYTES`].
pub(crate) fn create(
    ledger: &Ledger,
    folder: &str,
    model: &str,
    name: &str,
    updated: i64,
    snapshot: &Snapshot,
) -> Result<String, ExecError> {
    let kept = validate_name(name).map_err(ExecError::TuiSession)?;
    let id = mint_id()?;
    let transcript = snapshot.to_json().map_err(ExecError::TuiSession)?;
    if transcript.len() > MAX_STORED_BYTES {
        return Err(ExecError::TuiSession(SessionStoreError::TooLarge {
            id,
            bytes: transcript.len(),
        }));
    }
    let created = ledger.session_create(&crate::executor::NewSession {
        id: &id,
        name: &kept,
        folder,
        model,
        updated,
        transcript: &transcript,
        bound: MAX_SESSIONS,
    })?;
    if !created {
        return Err(ExecError::TuiSession(SessionStoreError::Full {
            count: session_total(ledger),
            bound: MAX_SESSIONS,
        }));
    }
    Ok(id)
}

/// The sessions saved for `folder`, newest first, at most [`LIST_LIMIT`].
pub(crate) fn list(
    ledger: &Ledger,
    folder: &str,
) -> Result<Vec<crate::executor::SessionHead>, ExecError> {
    ledger.session_list(folder, LIST_LIMIT)
}

/// The session `id` holds: its head and its snapshot.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming the id, [`SessionStoreError::Damaged`]
/// when its bytes are not a snapshot.
pub(crate) fn load(
    ledger: &Ledger,
    id: &str,
) -> Result<(crate::executor::SessionHead, Snapshot), ExecError> {
    let record = ledger.session_load(id)?.ok_or_else(|| {
        ExecError::TuiSession(SessionStoreError::NoSuchSession { id: id.to_owned() })
    })?;
    let snapshot =
        Snapshot::from_json(id, record.transcript.as_bytes()).map_err(ExecError::TuiSession)?;
    Ok((record.head, snapshot))
}

/// Names the session `id` holds `name`.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming the id, [`SessionStoreError::BadName`]
/// for a refused name.
pub(crate) fn rename(ledger: &Ledger, id: &str, name: &str, updated: i64) -> Result<(), ExecError> {
    let kept = validate_name(name).map_err(ExecError::TuiSession)?;
    let changed = ledger.session_rename(id, &kept, updated)?;
    if !changed {
        return Err(ExecError::TuiSession(SessionStoreError::NoSuchSession {
            id: id.to_owned(),
        }));
    }
    Ok(())
}

/// Holds `snapshot` in the session `id`.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming the id, [`SessionStoreError::TooLarge`]
/// past [`MAX_STORED_BYTES`].
pub(crate) fn save(
    ledger: &Ledger,
    id: &str,
    updated: i64,
    snapshot: &Snapshot,
) -> Result<(), ExecError> {
    let transcript = snapshot.to_json().map_err(ExecError::TuiSession)?;
    if transcript.len() > MAX_STORED_BYTES {
        return Err(ExecError::TuiSession(SessionStoreError::TooLarge {
            id: id.to_owned(),
            bytes: transcript.len(),
        }));
    }
    let changed = ledger.session_save(id, updated, &transcript)?;
    if !changed {
        return Err(ExecError::TuiSession(SessionStoreError::NoSuchSession {
            id: id.to_owned(),
        }));
    }
    Ok(())
}

/// Copies the session `from` into a new session named `name`, holding the same
/// bytes, and returns the new id. The original row is untouched, so a forked
/// session diverges and the original is unchanged.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming `from`, [`SessionStoreError::BadName`]
/// for a refused name, [`SessionStoreError::Full`] past [`MAX_SESSIONS`] rows.
pub(crate) fn fork(
    ledger: &Ledger,
    from: &str,
    name: &str,
    updated: i64,
) -> Result<String, ExecError> {
    let record = ledger.session_load(from)?.ok_or_else(|| {
        ExecError::TuiSession(SessionStoreError::NoSuchSession {
            id: from.to_owned(),
        })
    })?;
    let kept = validate_name(name).map_err(ExecError::TuiSession)?;
    let id = mint_id()?;
    let copied = ledger.session_fork(&id, &kept, updated, &record, MAX_SESSIONS)?;
    if !copied {
        return Err(ExecError::TuiSession(SessionStoreError::Full {
            count: session_total(ledger),
            bound: MAX_SESSIONS,
        }));
    }
    Ok(id)
}

/// How many session rows the store holds; the bound check's evidence when a
/// create or fork lands past it. A count that cannot be read reports the
/// ceiling, never a smaller number that would read as room.
fn session_total(ledger: &Ledger) -> usize {
    match ledger.session_count() {
        Ok(count) => match usize::try_from(count) {
            Ok(total) => total,
            Err(_) => usize::MAX,
        },
        Err(_) => usize::MAX,
    }
}

/// The transcript plus every receipt digest as Markdown: one section per turn,
/// commands in code fences, then the receipt digests the session has seen.
pub(crate) fn export_markdown(
    name: &str,
    folder: &str,
    model: &str,
    snapshot: &Snapshot,
) -> String {
    let mut out = String::from("# ");
    out.push_str(name);
    out.push_str("\n\nfolder: ");
    out.push_str(folder);
    out.push_str("\nmodel: ");
    out.push_str(model);
    out.push_str("\n\n");
    for entry in &snapshot.entries {
        match *entry {
            Entry::Header(_) => {}
            Entry::User(ref text) => {
                out.push_str("## ask\n\n");
                out.push_str(text);
                out.push_str("\n\n");
            }
            Entry::Ran {
                ref command,
                ref status,
                ref output,
                hidden,
                passed,
            } => {
                out.push_str("## ran: ");
                out.push_str(command);
                out.push('\n');
                out.push_str(if passed { "passed" } else { "failed" });
                out.push_str(": ");
                out.push_str(status);
                out.push('\n');
                if hidden > 0 {
                    out.push_str("(plus ");
                    out.push_str(&hidden.to_string());
                    out.push_str(" earlier lines)\n");
                }
                out.push_str("```\n");
                for line in output {
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str("```\n\n");
            }
            Entry::Declined(ref command) => {
                out.push_str("## declined: ");
                out.push_str(command);
                out.push_str("\n\n");
            }
            Entry::Said(ref text) => {
                out.push_str("## said\n\n");
                out.push_str(text);
                out.push_str("\n\n");
            }
            Entry::Notice(_, ref text) => {
                out.push_str("> ");
                out.push_str(text);
                out.push_str("\n\n");
            }
            Entry::Info(ref rows) => {
                for row in rows {
                    out.push_str("- ");
                    out.push_str(row);
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }
    out.push_str("## receipts\n\n");
    if snapshot.receipts.is_empty() {
        out.push_str("none\n");
    }
    for receipt in &snapshot.receipts {
        out.push_str("- digest ");
        out.push_str(&receipt.digest);
        out.push_str(", verdict ");
        out.push_str(if receipt.accepted {
            "accepted"
        } else {
            "not accepted"
        });
        out.push_str(", commands ");
        out.push_str(&receipt.commands.to_string());
        out.push_str(" run, ");
        out.push_str(&receipt.declined.to_string());
        out.push_str(" declined, ");
        out.push_str(&receipt.interrupted.to_string());
        out.push_str(" interrupted, change ");
        out.push_str(&receipt.handoff);
        out.push('\n');
    }
    out
}

/// The session in a few lines, without changing it: turns, commands, declines
/// and receipts. `/recap` prints these; nothing is compacted.
pub(crate) fn recap(snapshot: &Snapshot) -> Vec<String> {
    let mut asks = 0_usize;
    let mut ran = 0_usize;
    let mut passed = 0_usize;
    let mut declined = 0_usize;
    for entry in &snapshot.entries {
        match *entry {
            Entry::User(_) => asks = asks.saturating_add(1),
            Entry::Ran { passed: true, .. } => {
                ran = ran.saturating_add(1);
                passed = passed.saturating_add(1);
            }
            Entry::Ran { .. } => ran = ran.saturating_add(1),
            Entry::Declined(_) => declined = declined.saturating_add(1),
            Entry::Header(_) | Entry::Said(_) | Entry::Notice(..) | Entry::Info(_) => {}
        }
    }
    let accepted = snapshot
        .receipts
        .iter()
        .filter(|receipt| receipt.accepted)
        .count();
    vec![
        format!("{asks} asks, {ran} commands ({passed} passed, {declined} declined)"),
        format!("{} receipts, {accepted} accepted", snapshot.receipts.len()),
    ]
}

#[cfg(test)]
#[path = "sim_sessions.rs"]
mod tests;
