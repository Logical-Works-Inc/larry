//! The terminal session's memory and its row (larry#256, ADR 0133): the
//! transcript the frame shows ([`Transcript`], bounded and snapshot as JSON)
//! and the driver-owned [`SessionStore`] that holds it in the state database's
//! `tui_sessions` table after every turn. The frame (`tui::app`) stays I/O-free.
//! A kill -9 loses at most the turn in flight; a refused store leaves one
//! notice and the frame runs unsaved.

use std::collections::VecDeque;
use std::path::Path;

use super::app::App;
use super::cells::{Entry, Header};
use super::pane::Pane;
use super::sessions::{self, Owner, Snapshot};
use super::style::Style;
use super::worker::Receipt;
use crate::executor::{ExecError, Ledger, Settings};

/// The frame's transcript: the entries oldest-first and every receipt digest
/// seen, oldest-first and bounded. The driver snapshots it after every turn.
#[derive(Debug, Clone)]
pub(crate) struct Transcript {
    /// The transcript, oldest first.
    entries: VecDeque<Entry>,
    /// Every receipt digest seen, oldest first.
    receipts: VecDeque<Receipt>,
}

impl Transcript {
    /// A fresh transcript showing `header`.
    pub(crate) fn new(header: Header) -> Self {
        Self {
            entries: VecDeque::from([Entry::Header(header)]),
            receipts: VecDeque::new(),
        }
    }

    /// Adds `entry`, dropping the oldest past the frame bound.
    pub(crate) fn push(&mut self, entry: Entry) {
        if self.entries.len() >= super::app::MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// The transcript entries, oldest first.
    pub(crate) fn entries(&self) -> &VecDeque<Entry> {
        &self.entries
    }

    /// Clears the chat keeping only the welcome box, the way `/clear` does.
    /// When no entry is a welcome box, the transcript ends up empty.
    pub(crate) fn clear_keeping_header(&mut self) {
        let header = self
            .entries
            .iter()
            .find(|entry| matches!(**entry, Entry::Header(_)))
            .cloned();
        self.entries = header.into_iter().collect();
    }

    /// The transcript as stored: the entries, `pane` and every receipt
    /// digest seen.
    pub(crate) fn snapshot(&self, pane: Pane) -> Snapshot {
        Snapshot {
            entries: self.entries.iter().cloned().collect(),
            pane,
            receipts: self.receipts.iter().cloned().collect(),
        }
    }

    /// Remembers `receipt`'s digest, oldest-first and bounded, so the row
    /// names every digest even though the pane shows only the newest.
    pub(crate) fn note_receipt(&mut self, receipt: &Receipt) {
        if self.receipts.len() >= sessions::MAX_RECEIPTS_SEEN {
            self.receipts.pop_front();
        }
        self.receipts.push_back(receipt.clone());
    }
}

/// The state database, open or not: a frame that runs unsaved always knows why.
enum Store {
    /// The open state database.
    Ready(Ledger),
    /// The database refused to open.
    Refused(ExecError),
    /// The settings were refused; the welcome box already names why.
    Unset,
}

/// The row of the TUI session this process runs, in the state database.
pub(crate) struct SessionStore {
    /// The open state database, or why the store runs unsaved.
    store: Store,
    /// Whose row it is: the tenant, the folder it is listed under, the model.
    owner: Owner,
    /// The row the frame is held in, empty until one is adopted.
    current: String,
    /// Whether the refused save already left its one notice.
    warned: bool,
}

impl SessionStore {
    /// Opens the store for `settings` in `cwd`, going to `model`: the ledger
    /// when the settings allowed one, else the refusal [`SessionStore::adopt`]
    /// reports.
    pub(crate) fn open(settings: &Result<Settings, ExecError>, cwd: &Path, model: &str) -> Self {
        let (store, tenant) = match *settings {
            Ok(ref settings) => {
                let store = match Ledger::open(settings.state_dir()) {
                    Ok(ledger) => Store::Ready(ledger),
                    Err(cause) => Store::Refused(cause),
                };
                (store, settings.tenant_text().to_owned())
            }
            // Nothing is written without settings, so no row needs a tenant.
            Err(_) => (Store::Unset, String::new()),
        };
        Self {
            store,
            owner: Owner {
                tenant,
                folder: cwd.to_string_lossy().into_owned(),
                model: model.to_owned(),
            },
            current: String::new(),
            warned: false,
        }
    }

    /// Wall-clock milliseconds since the Unix epoch. A clock that reads before
    /// the epoch stamps the epoch and one past the bound stamps its newest: the
    /// stamp orders rows only, never a replay.
    fn now_ms() -> i64 {
        let Ok(elapsed) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
            return 0;
        };
        let Ok(millis) = i64::try_from(elapsed.as_millis()) else {
            return i64::MAX;
        };
        millis
    }

    /// Creates the row of this process and adopts it. A refused store or row
    /// leaves one notice and the frame runs unsaved.
    pub(crate) fn adopt(&mut self, app: &mut App) {
        let ledger = match self.store {
            Store::Ready(ref ledger) => ledger,
            Store::Refused(ref cause) => return unsaved(app, Style::WARN, cause),
            Store::Unset => return,
        };
        match sessions::create(
            ledger,
            &self.owner,
            "Session 1",
            Self::now_ms(),
            &app.snapshot(),
        ) {
            Ok(id) => self.current = id,
            // No `warned` latch here: a failed adopt leaves `current` empty,
            // so every later save returns before consulting it; this notice
            // is the one refusal the failure leaves.
            Err(cause) => unsaved(app, Style::BAD, &cause),
        }
    }

    /// Holds the frame in its row, after every turn, so a kill -9 loses at
    /// most the turn in flight. A frame past the row's byte bound is held
    /// without its oldest entries. One refusal leaves one notice; the frame
    /// keeps running unsaved rather than nagging.
    pub(crate) fn save_app(&mut self, app: &mut App) {
        if self.current.is_empty() || self.warned {
            return;
        }
        let Store::Ready(ref ledger) = self.store else {
            return;
        };
        let held = app
            .snapshot()
            .fitted()
            .map_err(ExecError::TuiSession)
            .and_then(|snapshot| {
                sessions::save(
                    ledger,
                    &self.owner,
                    &self.current,
                    Self::now_ms(),
                    &snapshot,
                )
            });
        if let Err(cause) = held {
            self.warned = true;
            unsaved(app, Style::BAD, &cause);
        }
    }
}

/// Tells the person their session is not being saved, and why.
fn unsaved(app: &mut App, style: Style, cause: &ExecError) {
    app.transcript().push(Entry::Notice(
        style,
        format!("sessions are not saved: {cause}"),
    ));
}
