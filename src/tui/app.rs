//! The frame's state machine (larry#239): keys, pastes, sizes, ticks and the
//! session's events go in; effects for the driver and one painted frame come
//! out. It reads no clock, no terminal and no thread, so a seeded script of
//! inputs gives the same frames on every run, and the same machine can sit
//! behind the Mac, iOS and Android shells the Director named after the TUI.
//!
//! Phases: idle (the composer takes a task), working (a session runs; Esc
//! interrupts), asking (the session waits on the person: y runs, a runs and
//! stops asking for this run, n declines, and Enter confirms the highlighted
//! answer, which starts at No), stopping (interrupted; the running command, if
//! any, ends first). A key read before the frame that paints a question answers
//! nothing: it is dropped and counted, so a key typed while the model worked
//! can never consent to a command nobody has read (larry#325). Esc never leaves
//! a phase it does not own: it closes the slash popup, declines a question or
//! interrupts a task.

use std::cell::Cell;
#[cfg(test)]
use std::collections::VecDeque;
use std::time::Duration;

use super::ask::offer;
use super::buffer::Frame;
use super::cells::{Entry, Header, split_observation};
use super::composer::Composer;
use super::layout::split;
use super::paint;
use super::pane::Pane;
use super::session_store::Transcript;
use super::sessions::Snapshot;
use super::style::{Style, sanitize};
use super::worker::{Decision, Ended, Event};

/// Most transcript entries kept; the oldest goes first.
pub(crate) const MAX_ENTRIES: usize = 512;
/// What `/help` prints.
const HELP: [&str; 6] = [
    "Enter sends the task. Ctrl+J adds a new line.",
    "Larry asks before each command: y runs it, a runs it and stops asking, n declines.",
    "Esc stops a task. PgUp and PgDn scroll. Ctrl+C on an empty box quits.",
    "/status shows the model, policy and folder. /clear clears the chat.",
    "/resume lists this folder's sessions, /resume 1 continues one; /fork copies one.",
    "/new starts fresh, /rename names it, /export writes Markdown, /recap summarises.",
];
/// One key, already read from the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    /// A printable character.
    Char(char),
    /// Enter.
    Enter,
    /// Ctrl+J, Alt+Enter: a new line in the composer.
    NewLine,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home, Ctrl+A.
    Home,
    /// End, Ctrl+E.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Tab.
    Tab,
    /// Esc.
    Esc,
    /// Ctrl+C.
    Interrupt,
    /// Ctrl+D.
    EndOfInput,
    /// Ctrl+U.
    KillStart,
    /// Ctrl+K.
    KillEnd,
    /// Ctrl+W.
    KillWord,
}

/// One input to the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Input {
    /// A key.
    Key(Key),
    /// Bracketed paste.
    Paste(String),
    /// The terminal's new size, columns then rows.
    Resize(usize, usize),
    /// Time since the frame started.
    Tick(Duration),
    /// An event from the running session.
    Worker(Event),
}

/// What the driver must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Start a session on this instruction.
    Start(String),
    /// Answer the running session's question.
    Decide(Decision),
    /// Stop the running session before its next model call or command.
    Interrupt,
    /// Leave: restore the terminal and exit 0.
    Quit,
    /// Start a fresh TUI session in the same process; the driver names it.
    SessionNew,
    /// List this folder's saved sessions; the driver prints them.
    SessionList,
    /// Continue the saved session this id, name prefix or listing number names.
    SessionResume(String),
    /// Copy a saved session (this one when empty) into a new session.
    SessionFork(Option<String>),
    /// Name this session.
    SessionRename(String),
    /// Write the transcript plus every receipt digest as Markdown.
    SessionExport,
    /// Summarise this session without changing it.
    SessionRecap,
}

/// Where the machine is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The composer takes a task.
    Idle,
    /// A session runs; the command, while one runs.
    Working(Option<String>),
    /// The session asks to run this command, or, with no command, whether to apply the
    /// change its copy of the folder holds (larry#243); the highlighted answer.
    Asking(Option<String>, Decision),
    /// Interrupted; whether a command still runs.
    Stopping(bool),
}

/// The frame's whole state.
#[derive(Debug, Clone)]
pub(crate) struct App {
    /// The transcript: the entries, the session they belong to and every
    /// receipt digest seen (larry#256).
    transcript: Transcript,
    /// The text box.
    composer: Composer,
    /// Where the machine is.
    phase: Phase,
    /// Lines `/status` prints.
    status: Vec<String>,
    /// When the running task started.
    started: Duration,
    /// The latest tick.
    now: Duration,
    /// Columns.
    width: usize,
    /// Rows.
    height: usize,
    /// Transcript rows scrolled up from the bottom.
    scroll: usize,
    /// Whether the person said to stop asking for this run.
    always: bool,
    /// Whether Ctrl+C asked to leave once the running session ends.
    quitting: bool,
    /// The session's last message, shown as its summary or its refusal.
    said: Option<String>,
    /// A one-line hint over the footer until the next key.
    hint: Option<String>,
    /// The artifact pane's state: the task's output, diff and receipt (larry#244).
    pane: Pane,
    /// Frames painted since the frame started (larry#325).
    frames: Cell<u64>,
    /// The frame that painted the question now asked, `None` until one carries it.
    painted: Cell<Option<u64>>,
    /// Keys dropped because they were read before the question's frame was painted.
    early: usize,
}

/// `said`, or `fallback` when the session said nothing.
fn or_said(said: Option<String>, fallback: &str) -> String {
    match said {
        Some(text) => text,
        None => fallback.to_owned(),
    }
}

impl App {
    /// A fresh frame showing `header`, `/status` printing `status`.
    pub(crate) fn new(header: Header, status: Vec<String>, width: usize, height: usize) -> Self {
        Self {
            transcript: Transcript::new(header),
            composer: Composer::default(),
            phase: Phase::Idle,
            status,
            started: Duration::ZERO,
            now: Duration::ZERO,
            width,
            height,
            scroll: 0,
            always: false,
            quitting: false,
            said: None,
            hint: None,
            pane: Pane::new(),
            frames: Cell::new(0),
            painted: Cell::new(None),
            early: 0,
        }
    }

    /// Whether a session is running, so the driver ticks the spinner.
    pub(crate) fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// The transcript and the session it belongs to (larry#256).
    pub(crate) fn transcript(&mut self) -> &mut Transcript {
        &mut self.transcript
    }

    /// What the paint reads: shared refs to the frame's state, so the update
    /// and the render stay two jobs.
    pub(crate) fn view(&self) -> paint::View<'_> {
        paint::View {
            entries: self.transcript.entries(),
            phase: &self.phase,
            composer: &self.composer,
            pane: &self.pane,
            scroll: self.scroll,
            width: self.width,
            height: self.height,
            started: self.started,
            now: self.now,
            hint: &self.hint,
            frames: &self.frames,
            painted: &self.painted,
        }
    }

    /// The frame for the current state, painted from [`App::view`].
    pub(crate) fn draw(&self) -> Frame {
        self.view().draw()
    }

    /// The frame as stored rows, through the transcript and the pane.
    /// The driver holds it in the state database after every turn, so a kill
    /// -9 loses at most the turn in flight.
    pub(crate) fn snapshot(&self) -> Snapshot {
        let pane = self.pane.snapshot();
        self.transcript.snapshot(pane)
    }

    /// Holds `snapshot`: the transcript, the pane and every receipt digest.
    pub(crate) fn restore(&mut self, snapshot: Snapshot) {
        let Snapshot {
            entries,
            pane,
            receipts,
        } = snapshot;
        self.pane.restore(&pane);
        self.transcript.restore_entries(entries, receipts);
        self.scroll = 0;
    }

    /// Starts a fresh session showing `header`. The driver stores the snapshot
    /// under the new row it just created.
    pub(crate) fn begin_session(&mut self, header: Header) {
        self.transcript.begin(header);
        self.composer.clear();
        self.phase = Phase::Idle;
        self.said = None;
        self.hint = None;
        self.scroll = 0;
        self.always = false;
        self.pane.begin();
    }

    /// Whether the person said to stop asking for this run.
    pub(crate) const fn always(&self) -> bool {
        self.always
    }

    /// The transcript entries, oldest first; the bound sims read it.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &VecDeque<Entry> {
        self.transcript.entries()
    }

    /// Takes one input and returns what the driver must do.
    pub(crate) fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Tick(now) => {
                self.now = now;
                Vec::new()
            }
            Input::Resize(width, height) => {
                self.width = width;
                self.height = height;
                Vec::new()
            }
            Input::Paste(text) => {
                if !matches!(self.phase, Phase::Asking(..)) && !self.composer.insert(&text) {
                    self.hint = Some(String::from("The paste was cut at 64 KiB."));
                }
                Vec::new()
            }
            Input::Worker(event) => self.event(event),
            Input::Key(key) => {
                self.hint = None;
                self.key(key)
            }
        }
    }

    /// One session event.
    fn event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Ask(command) => {
                self.phase = Phase::Asking(Some(command), Decision::Decline);
                self.painted.set(None);
                self.scroll = 0;
            }
            Event::Started(command) => {
                self.pane.started(&command);
                self.phase = if matches!(self.phase, Phase::Working(_)) {
                    Phase::Working(Some(command))
                } else {
                    Phase::Stopping(true)
                };
            }
            Event::Stream(chunk) => self.pane.stream(&chunk),
            Event::StreamDropped(chunks) => self.pane.stream_dropped(chunks),
            Event::Finished {
                observation,
                passed,
            } => {
                let command = if let Phase::Working(Some(ref command)) = self.phase {
                    command.clone()
                } else {
                    String::from("(command)")
                };
                let (status, output) = split_observation(&observation);
                self.pane.finished(&command, &status, &output, passed);
                self.transcript
                    .push(Entry::ran(command, &status, output, passed));
                self.phase = match self.phase {
                    Phase::Stopping(_) => Phase::Stopping(false),
                    Phase::Idle | Phase::Working(_) | Phase::Asking(..) => Phase::Working(None),
                };
            }
            Event::Said(text) => self.said = Some(text),
            Event::Review(summary) => {
                // The names in a diff are the folder's, not Larry's: each line is made safe.
                self.transcript.push(Entry::Info(
                    summary.iter().map(|line| sanitize(line, false)).collect(),
                ));
                self.phase = Phase::Asking(None, Decision::Decline);
                self.painted.set(None);
                self.scroll = 0;
            }
            Event::Handed { line, applied } => {
                let style = if applied { Style::GOOD } else { Style::WARN };
                self.transcript
                    .push(Entry::Notice(style, sanitize(&line, false)));
            }
            Event::Diff(text) => self.pane.set_diff(&text),
            Event::Receipt(receipt) => {
                self.transcript.note_receipt(&receipt);
                self.pane.set_receipt(receipt);
            }
            Event::Ended(ended) => return self.ended(ended),
        }
        Vec::new()
    }

    /// The session ended.
    fn ended(&mut self, ended: Ended) -> Vec<Effect> {
        let said = self.said.take();
        let entry = match ended {
            Ended::Done => Entry::Said(or_said(said, "Done.")),
            Ended::StepBound => Entry::Notice(
                Style::WARN,
                String::from("Stopped after 256 commands, the bound of one task."),
            ),
            Ended::Refused => Entry::Notice(Style::BAD, or_said(said, "The task was refused.")),
            Ended::Interrupted => {
                Entry::Notice(Style::WARN, String::from("Interrupted. Nothing more runs."))
            }
            Ended::Failed(why) => Entry::Notice(Style::BAD, why),
        };
        self.transcript.push(entry);
        self.phase = Phase::Idle;
        if self.quitting {
            return vec![Effect::Quit];
        }
        Vec::new()
    }

    /// One key.
    fn key(&mut self, key: Key) -> Vec<Effect> {
        if self.pane_key(key) {
            return Vec::new();
        }
        if let Phase::Asking(ref command, choice) = self.phase {
            let command = command.clone();
            if self.painted.get().is_none() {
                self.early = self.early.saturating_add(1);
                self.hint = Some(format!(
                    "A key typed before the question was shown was ignored ({} so far).",
                    self.early
                ));
                return Vec::new();
            }
            return self.answer(key, command, choice);
        }
        match key {
            Key::PageUp => {
                let step = self.view().page();
                let end = self.view().max_scroll();
                self.scroll = self.scroll.saturating_add(step).min(end);
                return Vec::new();
            }
            Key::PageDown => {
                let step = self.view().page();
                self.scroll = self.scroll.saturating_sub(step);
                return Vec::new();
            }
            Key::Esc | Key::Interrupt if self.phase != Phase::Idle => return self.interrupt(key),
            Key::Esc if self.composer.popup().is_some() => self.composer.clear(),
            Key::Interrupt if !self.composer.is_empty() => self.composer.clear(),
            Key::Interrupt | Key::EndOfInput if self.composer.is_empty() => {
                return vec![Effect::Quit];
            }
            Key::Enter => return self.enter(),
            Key::Char(character) => {
                let mut buffer = [0_u8; 4];
                self.composer.insert(character.encode_utf8(&mut buffer));
            }
            Key::NewLine => {
                self.composer.insert("\n");
            }
            Key::Backspace => self.composer.backspace(),
            Key::Delete => self.composer.delete(),
            Key::Left => self.composer.left(),
            Key::Right => self.composer.right(),
            Key::Up => self.composer.up(),
            Key::Down => self.composer.down(),
            Key::Home => self.composer.home(),
            Key::End => self.composer.end(),
            Key::Tab => self.composer.complete(),
            Key::KillStart => self.composer.kill_start(),
            Key::KillEnd => self.composer.kill_end(),
            Key::KillWord => self.composer.kill_word(),
            Key::Esc | Key::Interrupt | Key::EndOfInput => {}
        }
        Vec::new()
    }

    /// PgUp, PgDn and Tab belong to the pane while the screen is wide enough to show it,
    /// the question included, so the diff can be read before the answer; Tab still
    /// completes while the slash popup is open.
    fn pane_key(&mut self, key: Key) -> bool {
        if split(self.width, self.height).artifact.is_none() {
            return false;
        }
        let body = Pane::body(self.height);
        match key {
            Key::PageUp => self.pane.page(true, body),
            Key::PageDown => self.pane.page(false, body),
            Key::Tab if self.composer.popup().is_none() => self.pane.next_tab(),
            _ => return false,
        }
        true
    }

    /// Esc or Ctrl+C while a session runs.
    fn interrupt(&mut self, key: Key) -> Vec<Effect> {
        match self.phase {
            Phase::Working(ref command) => {
                self.phase = Phase::Stopping(command.is_some());
                vec![Effect::Interrupt]
            }
            Phase::Stopping(_) if key == Key::Interrupt => {
                self.quitting = true;
                self.hint = Some(String::from("Larry quits when the session ends."));
                Vec::new()
            }
            Phase::Idle | Phase::Asking(..) | Phase::Stopping(_) => Vec::new(),
        }
    }

    /// A key while the session asks about `command`, `choice` highlighted.
    /// The same keys answer the review of a session's change (no `command`), which has no
    /// "always": its `a` falls to the ignored keys.
    fn answer(&mut self, key: Key, command: Option<String>, choice: Decision) -> Vec<Effect> {
        let decision = match key {
            Key::Char('y' | 'Y') => Decision::Run,
            Key::Char('a' | 'A') if command.is_some() => Decision::Always,
            Key::Char('n' | 'N') | Key::Esc | Key::Interrupt => Decision::Decline,
            Key::Enter => choice,
            Key::Left | Key::Right => {
                let order = offer(command.is_some()).1;
                let at = order.iter().position(|&(item, _)| item == choice);
                let next = match (key, at) {
                    (Key::Left, Some(index)) => index.checked_sub(1),
                    (_, Some(index)) => index.checked_add(1),
                    (_, None) => None,
                };
                if let Some(&(picked, _)) = next.and_then(|index| order.get(index)) {
                    self.phase = Phase::Asking(command, picked);
                }
                return Vec::new();
            }
            Key::Char(_)
            | Key::NewLine
            | Key::Backspace
            | Key::Delete
            | Key::Up
            | Key::Down
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::Tab
            | Key::EndOfInput
            | Key::KillStart
            | Key::KillEnd
            | Key::KillWord => return Vec::new(),
        };
        match decision {
            Decision::Decline => command
                .into_iter()
                .for_each(|command| self.transcript.push(Entry::Declined(command))),
            Decision::Always => self.always = true,
            Decision::Run => {}
        }
        self.phase = Phase::Working(None);
        let mut effects = vec![Effect::Decide(decision)];
        if key == Key::Interrupt {
            self.phase = Phase::Stopping(false);
            effects.push(Effect::Interrupt);
        }
        effects
    }

    /// Enter on the popup's selected slash command, or on a slash word with its
    /// Enter on the popup's selected slash command.
    fn command(&mut self) -> Vec<Effect> {
        let Some(name) = self.composer.selected() else {
            return self.submit();
        };
        self.composer.take();
        self.dispatch(name, "")
    }

    /// Enter on typed text: a slash command however it names itself — the
    /// whole word with arguments typed out, or the popup's highlighted row —
    /// else the old path, where a word starting `/` that is no command falls
    /// through to a task.
    fn enter(&mut self) -> Vec<Effect> {
        const KNOWN: [&str; 10] = [
            "/help", "/status", "/clear", "/quit", "/resume", "/fork", "/rename", "/new",
            "/export", "/recap",
        ];
        let raw = self.composer.text().to_owned();
        if let Some(word) = raw.split_whitespace().next()
            && KNOWN.contains(&word)
        {
            let arg: String = match raw.split_once(char::is_whitespace) {
                Some((_, rest)) => rest.trim().to_owned(),
                None => String::new(),
            };
            self.composer.take();
            return self.dispatch(word, &arg);
        }
        if self.composer.selected().is_some() {
            return self.command();
        }
        self.submit()
    }

    /// Runs the slash command `word` with its typed argument `arg`.
    fn dispatch(&mut self, word: &str, arg: &str) -> Vec<Effect> {
        match word {
            "/help" => self.transcript.push(Entry::Info(
                HELP.iter().map(|line| (*line).to_owned()).collect(),
            )),
            "/status" => self.transcript.push(Entry::Info(self.status.clone())),
            "/clear" => {
                self.transcript.clear_keeping_header();
                self.scroll = 0;
            }
            "/new" if arg.is_empty() => return vec![Effect::SessionNew],
            "/resume" if arg.is_empty() => return vec![Effect::SessionList],
            "/resume" => return vec![Effect::SessionResume(arg.to_owned())],
            "/fork" if arg.is_empty() => return vec![Effect::SessionFork(None)],
            "/fork" => return vec![Effect::SessionFork(Some(arg.to_owned()))],
            "/rename" if !arg.is_empty() => return vec![Effect::SessionRename(arg.to_owned())],
            "/export" if arg.is_empty() => return vec![Effect::SessionExport],
            "/recap" if arg.is_empty() => return vec![Effect::SessionRecap],
            "/quit" => return vec![Effect::Quit],
            "/rename" => self.transcript.push(Entry::Notice(
                Style::WARN,
                String::from("Usage: /rename <name>"),
            )),
            _ => self.transcript.push(Entry::Notice(
                Style::WARN,
                format!("{word} takes no argument"),
            )),
        }
        Vec::new()
    }

    /// Enter on typed text: a task, or a word starting `/` that is no command.
    fn submit(&mut self) -> Vec<Effect> {
        if self.phase != Phase::Idle {
            self.hint = Some(String::from("A task is running. Press Esc to stop it."));
            return Vec::new();
        }
        if self.composer.text().trim().is_empty() {
            return Vec::new();
        }
        let text = self.composer.take();
        if text.starts_with('/') && !text.contains(char::is_whitespace) {
            self.transcript.push(Entry::Notice(
                Style::WARN,
                format!("Unknown command {text}. Type / to see the commands."),
            ));
            return Vec::new();
        }
        self.transcript.push(Entry::User(text.clone()));
        self.phase = Phase::Working(None);
        self.started = self.now;
        self.said = None;
        self.scroll = 0;
        self.pane.begin();
        vec![Effect::Start(text)]
    }
}
