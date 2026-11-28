//! TUI sessions that survive a restart (larry#256, ADR 0133): each terminal
//! session is one row in the state database the executor already owns
//! (`executor.db`, never a stray file), holding its name, folder, model and the
//! whole transcript plus every receipt digest seen.
//!
//! The store layer ([`create`], [`save`]) is a thin typed shell over the
//! ledger's `tui_sessions` table: the ledger owns the SQL, this module owns the
//! snapshot shape and the name rule. A snapshot is the live frame's own types
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
    let transcript = snapshot.to_json().map_err(ExecError::TuiSession)?;
    if transcript.len() > MAX_STORED_BYTES {
        return Err(ExecError::TuiSession(SessionStoreError::TooLarge {
            id,
            bytes: transcript.len(),
        }));
    }
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

/// Holds `snapshot` in `owner`'s session `id`.
///
/// # Errors
///
/// [`SessionStoreError::NoSuchSession`] naming the id, [`SessionStoreError::TooLarge`]
/// past [`MAX_STORED_BYTES`].
pub(crate) fn save(
    ledger: &Ledger,
    owner: &Owner,
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
    let changed = ledger.session_save(&owner.tenant, id, updated, &transcript)?;
    if !changed {
        return Err(ExecError::TuiSession(SessionStoreError::NoSuchSession {
            id: id.to_owned(),
        }));
    }
    Ok(())
}

#[cfg(test)]
#[path = "sim_sessions.rs"]
mod tests;
