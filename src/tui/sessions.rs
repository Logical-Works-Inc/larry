//! TUI sessions that survive a restart (larry#256, ADR 0133): each terminal
//! session is one row in the state database the executor already owns
//! (`executor.db`, never a stray file), holding its name, folder, model and the
//! whole transcript plus every receipt digest seen.
//!
//! The store layer ([`create`], [`save`], [`load`], [`rename`]) is a thin typed
//! shell over the ledger's `tui_sessions` table: the ledger owns the SQL, this
//! module owns the snapshot shape, the name rule and how a typed target names a
//! row ([`find`]). A snapshot is the live frame's own types
//! ([`Entry`], [`Pane`], [`Receipt`]) as JSON through `lgwks_std::json`, at
//! most [`MAX_STORED_BYTES`], so one transcript can never fill the store; at
//! most [`MAX_SESSIONS`] rows exist. One home holds the transcript shape: no
//! mirror types, so a row always reads back what the frame showed.

use lgwks_std::json::{self, Deserialize, Serialize};

use super::cells::Entry;
use super::pane::Pane;
use super::worker::Receipt;
use crate::executor::{ExecError, Ledger, SessionStoreError};

/// Most TUI sessions one tenant holds; creating past it is refused typed.
pub(crate) const MAX_SESSIONS: u32 = 10_000;
/// Most characters a session name holds.
pub(crate) const MAX_NAME_CHARS: usize = 64;
/// Most bytes one stored snapshot holds.
pub(crate) const MAX_STORED_BYTES: usize = 4_194_304;
/// Most receipt digests one snapshot remembers; the pane shows the newest.
pub(crate) const MAX_RECEIPTS_SEEN: usize = 256;

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

    /// This snapshot without as many of its oldest entries as it takes to fit
    /// [`MAX_STORED_BYTES`], so a long session keeps being saved. A pane and
    /// receipts past the bound by themselves are left for the store to refuse.
    ///
    /// # Errors
    ///
    /// [`SessionStoreError::Encode`] when the snapshot cannot be encoded.
    pub(crate) fn fitted(mut self) -> Result<Self, SessionStoreError> {
        let mut excess = self.to_json()?.len().saturating_sub(MAX_STORED_BYTES);
        let mut left_out = 0_usize;
        for entry in &self.entries {
            if excess == 0 {
                break;
            }
            let encoded = json::to_string(entry).map_err(SessionStoreError::Encode)?;
            // The entry and the comma after it.
            excess = excess.saturating_sub(encoded.len().saturating_add(1));
            left_out = left_out.saturating_add(1);
        }
        self.entries.drain(..left_out);
        Ok(self)
    }
}

/// The name the person gave, trimmed: 1 to [`MAX_NAME_CHARS`] characters, no
/// control character (a newline would break a one-row listing).
///
/// # Errors
///
/// [`SessionStoreError::BadName`] naming the refused text.
pub(crate) fn validate_name(name: &str) -> Result<String, SessionStoreError> {
    let kept = name.trim();
    let sized = (1..=MAX_NAME_CHARS).contains(&kept.chars().count());
    if !sized || kept.chars().any(char::is_control) {
        return Err(SessionStoreError::BadName {
            name: name.to_owned(),
        });
    }
    Ok(kept.to_owned())
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

/// Whose session a row is and what the welcome box showed for it.
pub(crate) struct Owner {
    /// The tenant the session belongs to, as every ledger row spells it.
    pub(crate) tenant: String,
    /// The folder the session runs in.
    pub(crate) folder: String,
    /// The model the session goes to.
    pub(crate) model: String,
}

/// Creates `owner`'s session row named `name`, holding `snapshot`, and returns
/// its id.
///
/// # Errors
///
/// [`SessionStoreError::Full`] past [`MAX_SESSIONS`] rows, [`SessionStoreError::BadName`]
/// for a refused name, [`SessionStoreError::TooLarge`] past [`MAX_STORED_BYTES`].
pub(crate) fn create(
    ledger: &Ledger,
    owner: &Owner,
    name: &str,
    updated: i64,
    snapshot: &Snapshot,
) -> Result<String, ExecError> {
    let kept = validate_name(name).map_err(ExecError::TuiSession)?;
    let id = mint_id()?;
    let transcript = bounded(&id, snapshot)?;
    let created = ledger.session_create(&crate::executor::NewSession {
        tenant: &owner.tenant,
        id: &id,
        name: &kept,
        folder: &owner.folder,
        model: &owner.model,
        updated,
        transcript: &transcript,
        bound: MAX_SESSIONS,
    })?;
    if !created {
        return Err(ExecError::TuiSession(SessionStoreError::Full {
            bound: MAX_SESSIONS,
        }));
    }
    Ok(id)
}

/// `snapshot` as the JSON of row `id`, refused past [`MAX_STORED_BYTES`].
fn bounded(id: &str, snapshot: &Snapshot) -> Result<String, ExecError> {
    let transcript = snapshot.to_json().map_err(ExecError::TuiSession)?;
    if transcript.len() > MAX_STORED_BYTES {
        return Err(ExecError::TuiSession(SessionStoreError::TooLarge {
            id: id.to_owned(),
            bytes: transcript.len(),
        }));
    }
    Ok(transcript)
}

/// The row this terminal holds: its id, its name and the stamp it last wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    /// The row's id.
    pub(crate) id: String,
    /// The row's name.
    pub(crate) name: String,
    /// The stamp a write must still find on the row.
    pub(crate) stamp: i64,
}

/// Why a write that changed nothing did: the row is gone, or another terminal
/// wrote it.
fn lost(ledger: &Ledger, owner: &Owner, id: &str) -> ExecError {
    let id = id.to_owned();
    match ledger.session_load(&owner.tenant, &id) {
        Ok(Some(_)) => ExecError::TuiSession(SessionStoreError::Moved { id }),
        Ok(None) => ExecError::TuiSession(SessionStoreError::NoSuchSession { id }),
        Err(cause) => cause,
    }
}

/// Holds `snapshot` in `owner`'s row `held` and returns its new stamp.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming the id, [`SessionStoreError::Moved`]
/// when another terminal wrote the row, [`SessionStoreError::TooLarge`] past
/// [`MAX_STORED_BYTES`].
pub(crate) fn save(
    ledger: &Ledger,
    owner: &Owner,
    held: &Held,
    now: i64,
    snapshot: &Snapshot,
) -> Result<i64, ExecError> {
    let transcript = bounded(&held.id, snapshot)?;
    match ledger.session_save(&owner.tenant, &held.id, held.stamp, now, &transcript)? {
        Some(stamp) => Ok(stamp),
        None => Err(lost(ledger, owner, &held.id)),
    }
}

/// `owner`'s row `id`, as a row to hold and the frame it stored.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming the id, [`SessionStoreError::Damaged`]
/// when its bytes are not a snapshot.
pub(crate) fn load(
    ledger: &Ledger,
    owner: &Owner,
    id: &str,
) -> Result<(Held, Snapshot), ExecError> {
    let id = id.to_owned();
    let Some((stamp, name, transcript)) = ledger.session_load(&owner.tenant, &id)? else {
        return Err(ExecError::TuiSession(SessionStoreError::NoSuchSession {
            id,
        }));
    };
    match json::from_slice(transcript.as_bytes()) {
        Ok(snapshot) => Ok((Held { id, name, stamp }, snapshot)),
        Err(cause) => Err(ExecError::TuiSession(SessionStoreError::Damaged {
            id,
            cause,
        })),
    }
}

/// Names `owner`'s row `held` and returns the name as kept.
///
/// # Errors
///
/// [`SessionStoreError::BadName`] for a refused name, [`SessionStoreError::NoSuchSession`]
/// or [`SessionStoreError::Moved`] when the row is no longer this terminal's.
pub(crate) fn rename(
    ledger: &Ledger,
    owner: &Owner,
    held: &Held,
    name: &str,
) -> Result<String, ExecError> {
    let kept = validate_name(name).map_err(ExecError::TuiSession)?;
    if ledger.session_rename(&owner.tenant, &held.id, held.stamp, &kept)? {
        Ok(kept)
    } else {
        Err(lost(ledger, owner, &held.id))
    }
}

/// The id of the one row of `rows` (id, name) that `target` names: its exact
/// name, else the start of its id.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] when none is named,
/// [`SessionStoreError::Ambiguous`] when more than one is.
pub(crate) fn find(rows: &[(String, String)], target: &str) -> Result<String, SessionStoreError> {
    let mut named: Vec<&String> = rows
        .iter()
        .filter(|row| row.1 == target)
        .map(|row| &row.0)
        .collect();
    if named.is_empty() {
        named = rows
            .iter()
            .filter(|row| row.0.starts_with(target))
            .map(|row| &row.0)
            .collect();
    }
    match *named.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(SessionStoreError::NoSuchSession {
            id: target.to_owned(),
        }),
        _ => Err(SessionStoreError::Ambiguous {
            target: target.to_owned(),
        }),
    }
}

#[cfg(test)]
#[path = "sim_sessions.rs"]
mod tests;
