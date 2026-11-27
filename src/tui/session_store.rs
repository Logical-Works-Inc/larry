//! The terminal session's memory and its rows (larry#256, ADR 0130): the
//! transcript the frame shows ([`Transcript`], bounded and snapshot as JSON)
//! and the driver-owned [`SessionStore`] that holds it in the state database's
//! `tui_sessions` table after every turn. The frame (`tui::app`) stays I/O-free:
//! slash words dispatch to effects, and this module answers them against the
//! store. A kill -9 loses at most the turn in flight; a refused settings row
//! runs the frame unsaved and says so when a session command runs.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::app::App;
use super::cells::{Entry, Header};
use super::pane::Pane;
use super::sessions::{self, Snapshot};
use super::style::Style;
use super::worker::Receipt;
use crate::executor::{ExecError, Ledger, SessionHead, Settings};

/// The frame's transcript and session identity: the entries oldest-first, the
/// row they belong to, and every receipt digest seen, oldest-first and
/// bounded. The driver snapshots it after every turn; a resume restores it
/// whole.
#[derive(Debug, Clone)]
pub(crate) struct Transcript {
    /// The transcript, oldest first.
    entries: VecDeque<Entry>,
    /// The person's name for it, shown by `/resume` and `/export`.
    session_name: String,
    /// Every receipt digest seen, oldest first.
    receipts: VecDeque<Receipt>,
}

impl Transcript {
    /// A fresh transcript showing `header`, unnamed as yet.
    pub(crate) fn new(header: Header) -> Self {
        Self {
            entries: VecDeque::from([Entry::Header(header)]),
            session_name: String::from("Session"),
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

    /// The person's name for this session.
    pub(crate) fn session_name(&self) -> &str {
        &self.session_name
    }

    /// Adopts the stored session named `name`.
    pub(crate) fn set_session(&mut self, name: &str) {
        name.clone_into(&mut self.session_name);
    }

    /// Names this session without touching the store; the driver saves.
    pub(crate) fn name_session(&mut self, name: &str) {
        name.clone_into(&mut self.session_name);
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

    /// Holds stored `entries` and `receipts`, oldest-first and bounded the way
    /// the live frame drops them.
    pub(crate) fn restore_entries(&mut self, entries: Vec<Entry>, receipts: Vec<Receipt>) {
        let mut kept: VecDeque<Entry> = entries.into_iter().collect();
        while kept.len() > super::app::MAX_ENTRIES {
            kept.pop_front();
        }
        self.entries = kept;
        let mut seen: VecDeque<Receipt> = receipts.into_iter().collect();
        while seen.len() > sessions::MAX_RECEIPTS_SEEN {
            seen.pop_front();
        }
        self.receipts = seen;
    }

    /// Starts fresh showing `header`: the transcript keeps only the welcome
    /// box and every digest is forgotten. The driver stores the snapshot under
    /// the new row it just created.
    pub(crate) fn begin(&mut self, header: Header) {
        self.entries = VecDeque::from([Entry::Header(header)]);
        self.receipts.clear();
    }

    /// Remembers `receipt`'s digest, oldest-first and bounded, so `/export`
    /// names every digest even though the pane shows only the newest.
    pub(crate) fn note_receipt(&mut self, receipt: &Receipt) {
        if self.receipts.len() >= sessions::MAX_RECEIPTS_SEEN {
            self.receipts.pop_front();
        }
        self.receipts.push_back(receipt.clone());
    }
}

/// The state database, open or refused: one or the other, never neither,
/// so a frame that runs unsaved always names why it is unsaved.
enum Store {
    /// The open state database.
    Ready(Ledger),
    /// Why the store runs unsaved.
    Disabled(String),
}

/// The TUI sessions saved in the state database: the current row, the folder
/// and model new rows carry, and the last listing `/resume 1` resolves
/// against. `store` is `Ready` when the settings allowed a database, else
/// the refusal a session command reports.
pub(crate) struct SessionStore {
    /// The open state database, or why the store runs unsaved.
    store: Store,
    /// The folder new rows are listed under.
    folder: String,
    /// Where `/export` writes.
    path: PathBuf,
    /// The model new rows name.
    model: String,
    /// The welcome box a fresh session shows.
    welcome: Header,
    /// The row the frame shows.
    current: String,
    /// Sessions started this process, naming `Session N`.
    counter: usize,
    /// Whether the refused save already left its one notice.
    warned: bool,
    /// The last listing, newest first, `/resume 1` resolving into it.
    last_list: Vec<SessionHead>,
}

impl SessionStore {
    /// Opens the store for `settings` in `cwd`, holding `welcome` for the
    /// fresh sessions `/new` starts: the ledger when the settings allowed one,
    /// else the refusal a session command reports.
    pub(crate) fn open(
        settings: &Result<Settings, ExecError>,
        cwd: &Path,
        model: &str,
        welcome: &Header,
    ) -> Self {
        let folder = cwd.to_string_lossy().into_owned();
        match *settings {
            Ok(ref settings) => match Ledger::open(settings.state_dir()) {
                Ok(ledger) => Self::with_store(Store::Ready(ledger), folder, cwd, model, welcome),
                Err(cause) => Self::with_store(
                    Store::Disabled(cause.to_string()),
                    folder,
                    cwd,
                    model,
                    welcome,
                ),
            },
            Err(ref cause) => Self::with_store(
                Store::Disabled(cause.to_string()),
                folder,
                cwd,
                model,
                welcome,
            ),
        }
    }

    /// A store holding `store`, named for `folder`, exporting beside `cwd`,
    /// going to `model` and welcoming with `welcome`.
    fn with_store(store: Store, folder: String, cwd: &Path, model: &str, welcome: &Header) -> Self {
        Self {
            store,
            folder,
            path: cwd.to_path_buf(),
            model: model.to_owned(),
            welcome: welcome.clone(),
            current: String::new(),
            counter: 0,
            warned: false,
            last_list: Vec::new(),
        }
    }

    /// The open ledger, when the store is ready and no notice is needed.
    fn ready(&self) -> Option<&Ledger> {
        match self.store {
            Store::Ready(ref ledger) => Some(ledger),
            Store::Disabled(_) => None,
        }
    }

    /// The open ledger, or the refusal a session command reports instead.
    fn ledger(&self, app: &mut App) -> Option<&Ledger> {
        match self.store {
            Store::Ready(ref ledger) => Some(ledger),
            Store::Disabled(ref cause) => {
                app.transcript().push(Entry::Notice(
                    Style::WARN,
                    format!("sessions are not saved: {cause}"),
                ));
                None
            }
        }
    }

    /// Wall-clock milliseconds since the Unix epoch, the listing order. A
    /// clock that reads before the epoch stamps the epoch (the listing's
    /// oldest) and one past the bound stamps its newest: the stamp orders a
    /// listing only, never a replay.
    fn now_ms() -> i64 {
        let Ok(elapsed) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
            return 0;
        };
        let Ok(millis) = i64::try_from(elapsed.as_millis()) else {
            return i64::MAX;
        };
        millis
    }

    /// Creates the first row of this process and adopts it.
    pub(crate) fn adopt(&mut self, app: &mut App) {
        let name = String::from("Session 1");
        let Some(ledger) = self.ready() else {
            return;
        };
        match sessions::create(
            ledger,
            &self.folder,
            &self.model,
            &name,
            Self::now_ms(),
            &app.snapshot(),
        ) {
            Ok(id) => {
                self.counter = 1;
                app.transcript().set_session(&name);
                self.current = id;
            }
            // No `warned` latch here: a failed adopt leaves `current` empty,
            // so every later save returns before consulting it; the notice
            // above is the one refusal this failure leaves.
            Err(cause) => {
                app.transcript()
                    .push(Entry::Notice(Style::BAD, cause.to_string()));
            }
        }
    }

    /// Holds the frame in its row, after every turn, so a kill -9 loses at
    /// most the turn in flight. One refusal leaves one notice; the frame keeps
    /// running unsaved rather than nagging.
    pub(crate) fn save_app(&mut self, app: &mut App) {
        if self.current.is_empty() || self.warned {
            return;
        }
        let Some(ledger) = self.ready() else {
            return;
        };
        let snapshot = app.snapshot();
        match sessions::save(ledger, &self.current, Self::now_ms(), &snapshot) {
            Ok(()) => {}
            Err(cause) => {
                self.warned = true;
                app.transcript()
                    .push(Entry::Notice(Style::BAD, cause.to_string()));
            }
        }
    }

    /// The saved session `arg` names: a 1-based number into the last listing,
    /// an id or name prefix from it, else a full id.
    fn resolve(&self, arg: &str) -> Option<String> {
        if let Ok(number) = arg.parse::<usize>()
            && (1..=self.last_list.len()).contains(&number)
        {
            return self
                .last_list
                .get(number.saturating_sub(1))
                .map(|head| head.id.clone());
        }
        if let Some(head) = self
            .last_list
            .iter()
            .find(|head| head.id.starts_with(arg) || head.name == arg)
        {
            return Some(head.id.clone());
        }
        Some(arg.to_owned())
    }

    /// Lists this folder's sessions; `/resume 1` continues the newest.
    pub(crate) fn list(&mut self, app: &mut App) {
        let Some(ledger) = self.ledger(app) else {
            return;
        };
        match sessions::list(ledger, &self.folder) {
            Err(cause) => app
                .transcript()
                .push(Entry::Notice(Style::BAD, cause.to_string())),
            Ok(rows) => {
                self.last_list = rows;
                if self.last_list.is_empty() {
                    app.transcript().push(Entry::Info(vec![String::from(
                        "no saved sessions for this folder",
                    )]));
                } else {
                    app.transcript().push(Entry::Info(
                        self.last_list
                            .iter()
                            .enumerate()
                            .map(|(index, head)| {
                                format!("{}. {} ({})", index.saturating_add(1), head.name, head.id)
                            })
                            .collect(),
                    ));
                }
            }
        }
    }

    /// Continues the saved session `arg` names: the transcript and the pane are
    /// back, and the fork the person may make next diverges from here.
    pub(crate) fn resume(&mut self, app: &mut App, arg: &str) {
        let Some(id) = self.resolve(arg) else {
            app.transcript().push(Entry::Notice(
                Style::WARN,
                String::from("usage: /resume lists, /resume 1 continues the newest"),
            ));
            return;
        };
        let Some(ledger) = self.ledger(app) else {
            return;
        };
        match sessions::load(ledger, &id) {
            Err(cause) => app
                .transcript()
                .push(Entry::Notice(Style::BAD, cause.to_string())),
            Ok((head, snapshot)) => {
                head.id.clone_into(&mut self.current);
                app.restore(snapshot);
                app.transcript().set_session(&head.name);
                app.transcript()
                    .push(Entry::Info(vec![format!("resumed {}.", head.name)]));
            }
        }
    }

    /// Copies a saved session into a new one: empty `arg` forks this frame,
    /// else the session `arg` names. The original row is untouched, so the
    /// fork diverges and the original is unchanged.
    pub(crate) fn fork(&mut self, app: &mut App, arg: Option<String>) {
        let from = match arg {
            None => {
                if self.current.is_empty() {
                    app.transcript().push(Entry::Notice(
                        Style::WARN,
                        String::from("nothing to fork yet"),
                    ));
                    return;
                }
                self.current.clone()
            }
            Some(arg) => match self.resolve(&arg) {
                Some(id) => id,
                None => {
                    app.transcript().push(Entry::Notice(
                        Style::WARN,
                        String::from(
                            "usage: /fork copies this session, /fork 1 copies the newest listed",
                        ),
                    ));
                    return;
                }
            },
        };
        let Some(ledger) = self.ledger(app) else {
            return;
        };
        let base = match sessions::load(ledger, &from) {
            Err(cause) => {
                app.transcript()
                    .push(Entry::Notice(Style::BAD, cause.to_string()));
                return;
            }
            Ok((head, _)) => head.name,
        };
        let mut name = format!("{base} fork");
        if name.chars().count() > sessions::MAX_NAME_CHARS {
            name = name.chars().take(sessions::MAX_NAME_CHARS).collect();
        }
        match sessions::fork(ledger, &from, &name, Self::now_ms()) {
            Err(cause) => app
                .transcript()
                .push(Entry::Notice(Style::BAD, cause.to_string())),
            Ok(id) => {
                // The copy holds the stored row; this turn's transcript, pane
                // and digests go on top, so the fork starts where the frame is.
                // The source row is untouched, so the fork diverges and the
                // original is unchanged.
                let snapshot = app.snapshot();
                if let Err(cause) = sessions::save(ledger, &id, Self::now_ms(), &snapshot) {
                    app.transcript()
                        .push(Entry::Notice(Style::BAD, cause.to_string()));
                }
                app.transcript().set_session(&name);
                self.current = id;
                app.transcript()
                    .push(Entry::Info(vec![format!("forked {base} as {name}.")]));
            }
        }
    }

    /// Names this session.
    pub(crate) fn rename(&mut self, app: &mut App, arg: &str) {
        if self.current.is_empty() {
            app.transcript().push(Entry::Notice(
                Style::WARN,
                String::from("nothing to rename yet"),
            ));
            return;
        }
        let Some(ledger) = self.ledger(app) else {
            return;
        };
        match sessions::rename(ledger, &self.current, arg, Self::now_ms()) {
            Err(cause) => app
                .transcript()
                .push(Entry::Notice(Style::BAD, cause.to_string())),
            Ok(()) => {
                app.transcript().name_session(arg.trim());
                app.transcript()
                    .push(Entry::Info(vec![format!("renamed to {}.", arg.trim())]));
            }
        }
    }

    /// Starts fresh in the same process: the frame keeps only its welcome box
    /// under a new row, and every earlier session stays under `/resume`.
    pub(crate) fn new_session(&mut self, app: &mut App) {
        if self.ledger(app).is_none() {
            return;
        }
        if !self.current.is_empty() {
            let snapshot = app.snapshot();
            if let Some(ledger) = self.ready()
                && let Err(cause) = sessions::save(ledger, &self.current, Self::now_ms(), &snapshot)
            {
                app.transcript()
                    .push(Entry::Notice(Style::BAD, cause.to_string()));
            }
        }
        self.counter = self.counter.saturating_add(1);
        let name = format!("Session {}", self.counter.max(1));
        app.begin_session(self.welcome.clone());
        let saved = match self.ready() {
            Some(ledger) => sessions::create(
                ledger,
                &self.folder,
                &self.model,
                &name,
                Self::now_ms(),
                &app.snapshot(),
            ),
            None => return,
        };
        match saved {
            Err(cause) => app
                .transcript()
                .push(Entry::Notice(Style::BAD, cause.to_string())),
            Ok(id) => {
                app.transcript().set_session(&name);
                self.current = id;
                app.transcript()
                    .push(Entry::Info(vec![format!("started {name}.")]));
            }
        }
    }

    /// Writes the transcript plus every receipt digest as Markdown beside the
    /// folder, and names the path.
    pub(crate) fn export(&mut self, app: &mut App) {
        if self.ledger(app).is_none() {
            return;
        }
        let snapshot = app.snapshot();
        let name = app.transcript().session_name().to_owned();
        let document = sessions::export_markdown(&name, &self.folder, &self.model, &snapshot);
        let path = self.path.join(format!("{}.md", sessions::slug(&name)));
        match std::fs::write(&path, document) {
            Err(cause) => app.transcript().push(Entry::Notice(
                Style::BAD,
                format!("export refused: {cause}"),
            )),
            Ok(()) => app.transcript().push(Entry::Info(vec![format!(
                "exported to {}.",
                path.display()
            )])),
        }
    }

    /// Summarises this session without changing it.
    pub(crate) fn recap(&mut self, app: &mut App) {
        if self.ledger(app).is_none() {
            return;
        }
        let lines = sessions::recap(&app.snapshot());
        app.transcript().push(Entry::Info(lines));
    }
}
