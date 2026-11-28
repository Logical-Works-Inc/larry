//! The terminal session's memory and its row (larry#256, ADR 0133): the
//! transcript the frame shows ([`Transcript`], bounded and snapshot as JSON)
//! and the driver-owned [`SessionStore`] that holds it in the state database's
//! `tui_sessions` table after every turn and answers `/resume`, `/new`, `/fork`
//! and `/rename`, and `/export` for the folder it runs in. The frame
//! (`tui::app`) stays I/O-free. A kill -9 loses at most the turn in flight; a
//! refused store leaves one notice and the frame runs unsaved.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::app::{App, SessionCommand};
use super::cells::{Entry, Header};
use super::export::export;
use super::pane::Pane;
use super::sessions::{self, Held, Owner, Snapshot};
use super::style::Style;
use super::worker::Receipt;
use crate::executor::{ExecError, Ledger, Settings};

/// Most sessions one `/resume` listing shows; the rest is counted, so a listing
/// never fills the transcript.
pub(crate) const LISTED: usize = 20;

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

    /// Holds a stored session's `entries` and `receipts`.
    pub(crate) fn restore(&mut self, entries: Vec<Entry>, receipts: Vec<Receipt>) {
        self.entries = entries.into();
        self.receipts = receipts.into();
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
    /// The folder `/export` writes inside, as the path the process runs in.
    folder: PathBuf,
    /// The row the frame is held in, none until one is adopted.
    current: Option<Held>,
    /// Whether the refused save already left its one notice.
    warned: bool,
    /// Sessions this process started, naming the next `Session N`.
    started: u32,
    /// The last `/resume` listing as (id, name), the numbers the person read.
    listed: Vec<(String, String)>,
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
            folder: cwd.to_path_buf(),
            current: None,
            warned: false,
            started: 0,
            listed: Vec::new(),
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
        match add(ledger, &self.owner, "Session 1", app.snapshot()) {
            Ok(held) => {
                self.started = 1;
                self.current = Some(held);
            }
            // No `warned` latch here: a failed adopt leaves no row, so every
            // later save returns before consulting it; this notice is the one
            // refusal the failure leaves.
            Err(cause) => unsaved(app, Style::BAD, &cause),
        }
    }

    /// Holds the frame in its row, after every turn, so a kill -9 loses at
    /// most the turn in flight. A frame past the row's byte bound is held
    /// without its oldest entries. One refusal leaves one notice; the frame
    /// keeps running unsaved rather than nagging.
    pub(crate) fn save_app(&mut self, app: &mut App) {
        if self.warned {
            return;
        }
        let Store::Ready(ref ledger) = self.store else {
            return;
        };
        let Some(held) = self.current.as_mut() else {
            return;
        };
        let saved = app
            .snapshot()
            .fitted()
            .map_err(ExecError::TuiSession)
            .and_then(|snapshot| {
                sessions::save(ledger, &self.owner, held, Self::now_ms(), &snapshot)
            });
        match saved {
            Ok(stamp) => held.stamp = stamp,
            Err(cause) => {
                self.warned = true;
                unsaved(app, Style::BAD, &cause);
            }
        }
    }

    /// Answers one session command: the frame switches rows only once the store
    /// has done its part, and a refusal leaves the frame and its row as they were.
    pub(crate) fn command(&mut self, app: &mut App, command: SessionCommand) {
        let said = match self.answer(app, command) {
            Ok(lines) => Entry::Info(lines),
            Err(cause) => Entry::Notice(Style::BAD, cause.to_string()),
        };
        app.transcript().push(said);
    }

    /// Writes the session on screen to `target` of the folder and says where,
    /// or why nothing was written. It needs no row, so a refused store changes
    /// nothing here.
    pub(crate) fn export(&self, app: &mut App, target: &str) {
        let name = match self.current {
            Some(ref held) => held.name.as_str(),
            None => "session",
        };
        let said = match export(
            &self.folder,
            target,
            name,
            &self.owner.model,
            &app.snapshot(),
        ) {
            Ok(lines) => Entry::Info(lines),
            Err(cause) => Entry::Notice(Style::BAD, cause.to_string()),
        };
        app.transcript().push(said);
    }

    /// Does what `command` asks and returns the lines that say so.
    fn answer(&mut self, app: &mut App, command: SessionCommand) -> Result<Vec<String>, ExecError> {
        let ledger = match self.store {
            Store::Ready(ref ledger) => ledger,
            Store::Refused(ref cause) => {
                return Ok(vec![format!("sessions are not saved: {cause}")]);
            }
            Store::Unset => {
                return Ok(vec![String::from(
                    "sessions are off until the settings are fixed; see /status.",
                )]);
            }
        };
        match command {
            SessionCommand::New => {
                let number = self.started.saturating_add(1);
                let name = format!("Session {number}");
                let snapshot = app.fresh();
                let held = add(ledger, &self.owner, &name, snapshot.clone())?;
                self.started = number;
                self.hold(app, held, snapshot);
                Ok(vec![format!("started {name}.")])
            }
            SessionCommand::Resume(target) if target.is_empty() => {
                self.listed = others(ledger, &self.owner, self.current.as_ref())?;
                Ok(listing(&self.listed))
            }
            SessionCommand::Resume(target) => {
                let (held, snapshot) = self.named(ledger, &target)?;
                let line = format!("resumed {}.", held.name);
                self.hold(app, held, snapshot);
                Ok(vec![line])
            }
            SessionCommand::Fork(target) => {
                let (base, snapshot) = if target.is_empty() {
                    let base = match self.current {
                        Some(ref held) => held.name.clone(),
                        None => String::from("Session"),
                    };
                    (base, app.snapshot())
                } else {
                    let (held, snapshot) = self.named(ledger, &target)?;
                    (held.name, snapshot)
                };
                let name: String = format!("{base} fork")
                    .chars()
                    .take(sessions::MAX_NAME_CHARS)
                    .collect();
                let held = add(ledger, &self.owner, &name, snapshot.clone())?;
                self.hold(app, held, snapshot);
                Ok(vec![format!("forked {base} as {name}.")])
            }
            SessionCommand::Rename(name) => {
                let Some(held) = self.current.as_mut() else {
                    return Ok(vec![String::from(
                        "this session is not saved; /new starts a saved one.",
                    )]);
                };
                held.name = sessions::rename(ledger, &self.owner, held, &name)?;
                Ok(vec![format!("renamed to {}.", held.name)])
            }
        }
    }

    /// The saved session `target` names: a number of the last listing, else a
    /// name or the start of an id among this folder's other sessions.
    fn named(&self, ledger: &Ledger, target: &str) -> Result<(Held, Snapshot), ExecError> {
        let rows = others(ledger, &self.owner, self.current.as_ref())?;
        let shown = if self.listed.is_empty() {
            &rows
        } else {
            &self.listed
        };
        // A target that is no number of the listing is a name or an id.
        let numbered = shown
            .iter()
            .zip(1_usize..)
            .find(|&(_, number)| number.to_string() == target);
        let id = match numbered {
            Some((row, _)) => row.0.clone(),
            None => sessions::find(&rows, target).map_err(ExecError::TuiSession)?,
        };
        sessions::load(ledger, &self.owner, &id)
    }

    /// Shows `snapshot` and holds the frame in `held` from this turn on.
    fn hold(&mut self, app: &mut App, held: Held, snapshot: Snapshot) {
        app.show(snapshot);
        self.current = Some(held);
        self.warned = false;
    }
}

/// Adds a row named `name` holding as much of `snapshot` as a row takes, and
/// returns it held.
fn add(ledger: &Ledger, owner: &Owner, name: &str, snapshot: Snapshot) -> Result<Held, ExecError> {
    let stamp = SessionStore::now_ms();
    let fitted = snapshot.fitted().map_err(ExecError::TuiSession)?;
    let id = sessions::create(ledger, owner, name, stamp, &fitted)?;
    Ok(Held {
        id,
        name: name.to_owned(),
        stamp,
    })
}

/// The folder's saved sessions as (id, name), newest first, without the one
/// this terminal holds.
fn others(
    ledger: &Ledger,
    owner: &Owner,
    current: Option<&Held>,
) -> Result<Vec<(String, String)>, ExecError> {
    let mut rows = ledger.session_list(&owner.tenant, &owner.folder)?;
    rows.retain(|row| current.is_none_or(|held| held.id != row.0));
    Ok(rows)
}

/// The `/resume` listing of `rows`: the newest [`LISTED`] by number, the rest counted.
fn listing(rows: &[(String, String)]) -> Vec<String> {
    let mut lines: Vec<String> = rows
        .iter()
        .take(LISTED)
        .enumerate()
        .map(|(at, row)| format!("{}. {} ({})", at.saturating_add(1), row.1, row.0))
        .collect();
    if lines.is_empty() {
        lines.push(String::from("no other saved session for this folder."));
    }
    let more = rows.len().saturating_sub(LISTED);
    if more > 0 {
        lines.push(format!("{more} older; /resume <name or id> reaches them."));
    }
    lines
}

/// Tells the person their session is not being saved, and why.
fn unsaved(app: &mut App, style: Style, cause: &ExecError) {
    app.transcript().push(Entry::Notice(
        style,
        format!("sessions are not saved: {cause}"),
    ));
}
