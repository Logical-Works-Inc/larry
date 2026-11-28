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

use super::ask::answer;
use super::buffer::Frame;
use super::cells::{Entry, Header, split_observation};
use super::composer::Composer;
use super::layout::split;
use super::paint;
use super::pane::Pane;
use super::pending::{Pending, QUEUE_BOUND};
use super::session_store::Transcript;
use super::sessions::Snapshot;
use super::style::{Style, sanitize};
use super::worker::{Decision, Ended, Event};

/// Most transcript entries kept; the oldest goes first.
pub(crate) const MAX_ENTRIES: usize = 512;
/// What `/help` prints.
const HELP: [&str; 8] = [
    "Enter sends the task. Ctrl+J adds a new line. Enter while working queues the next turn (8).",
    "Larry asks before each command: y runs it, a runs it and stops asking, n declines.",
    "Esc stops a task. PgUp and PgDn scroll. Ctrl+C on an empty box quits.",
    "/status shows the model, policy and folder. /clear clears the chat.",
    "/resume lists this folder's sessions; /resume 1 continues one. /new starts fresh.",
    "/fork copies this session; /fork 1 copies a listed one. /rename <name> names it.",
    "/ps lists background commands with their last lines. /stop ends them.",
    "/quit leaves Larry; with background commands running it asks first.",
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
    /// Answer a session command against the stored sessions.
    Session(SessionCommand),
}

/// A command over the stored sessions (larry#256). Only an idle frame sends one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionCommand {
    /// Start a fresh session in this process.
    New,
    /// Continue the named saved session; list them when empty.
    Resume(String),
    /// Copy the named saved session, or this one when empty.
    Fork(String),
    /// Name this session.
    Rename(String),
}

impl SessionCommand {
    /// The session command `text` is, its first word naming it.
    fn parse(text: &str) -> Option<Self> {
        let (word, rest) = match text.split_once(char::is_whitespace) {
            Some((word, rest)) => (word, rest.trim().to_owned()),
            None => (text, String::new()),
        };
        match word {
            "/new" => Some(Self::New),
            "/resume" => Some(Self::Resume(rest)),
            "/fork" => Some(Self::Fork(rest)),
            "/rename" => Some(Self::Rename(rest)),
            _ => None,
        }
    }
}

/// Where the machine is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Phase {
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
    /// The transcript and every receipt digest seen (larry#256).
    transcript: Transcript,
    /// The text box.
    pub(super) composer: Composer,
    /// Where the machine is.
    pub(super) phase: Phase,
    /// Turns queued while a session runs and the background commands it started (larry#262).
    pub(super) pending: Pending,
    /// Lines `/status` prints.
    status: Vec<String>,
    /// The welcome box a new session opens with.
    welcome: Header,
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
    pub(super) always: bool,
    /// Whether Ctrl+C asked to leave once the running session ends.
    pub(super) quitting: bool,
    /// The session's last message, shown as its summary or its refusal.
    said: Option<String>,
    /// A one-line hint over the footer until the next key.
    pub(super) hint: Option<String>,
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
            welcome: header.clone(),
            transcript: Transcript::new(header),
            composer: Composer::default(),
            phase: Phase::Idle,
            pending: Pending::new(),
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

    /// What the paint reads: shared refs to the frame's state, so the update
    /// and the render stay two jobs.
    pub(crate) fn view(&self) -> paint::View<'_> {
        paint::View {
            entries: self.transcript.entries(),
            phase: &self.phase,
            composer: &self.composer,
            pane: &self.pane,
            pending: &self.pending,
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

    /// Whether the person said to stop asking for this run.
    pub(crate) const fn always(&self) -> bool {
        self.always
    }

    /// The transcript the session store reads and writes notices into.
    pub(crate) fn transcript(&mut self) -> &mut Transcript {
        &mut self.transcript
    }

    /// The frame as stored (larry#256): the transcript, the pane and every
    /// receipt digest. The driver holds it in the state database after every
    /// turn, so a kill -9 loses at most the turn in flight.
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.transcript.snapshot(self.pane.snapshot())
    }

    /// What a session that has heard nothing holds: the welcome box alone.
    pub(crate) fn fresh(&self) -> Snapshot {
        Transcript::new(self.welcome.clone()).snapshot(Pane::new())
    }

    /// Continues the stored session `snapshot`: its transcript, pane and
    /// receipts, from the newest row. Nothing of the session left behind
    /// carries over, "stop asking" included.
    pub(crate) fn show(&mut self, snapshot: Snapshot) {
        let Snapshot {
            entries,
            pane,
            receipts,
        } = snapshot;
        self.transcript.restore(entries, receipts);
        self.pane = pane.resumed();
        self.pending = Pending::new();
        self.said = None;
        self.scroll = 0;
        self.always = false;
    }

    /// Adds `entry`, dropping the oldest past [`MAX_ENTRIES`].
    pub(super) fn push(&mut self, entry: Entry) {
        self.transcript.push(entry);
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
                self.pending.push_job(&command);
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
                } else if let Some(running) = self.pending.running_command() {
                    running
                } else {
                    String::from("(command)")
                };
                let (status, output) = split_observation(&observation);
                self.pending.finish_job(&command, &output, passed);
                self.pane.finished(&command, &status, &output, passed);
                self.push(Entry::ran(command, &status, output, passed));
                self.phase = match self.phase {
                    Phase::Stopping(_) => Phase::Stopping(false),
                    Phase::Idle | Phase::Working(_) | Phase::Asking(..) => Phase::Working(None),
                };
            }
            Event::Said(text) => self.said = Some(text),
            Event::Review(summary) => {
                // The names in a diff are the folder's, not Larry's: each line is made safe.
                self.push(Entry::Info(
                    summary.iter().map(|line| sanitize(line, false)).collect(),
                ));
                self.phase = Phase::Asking(None, Decision::Decline);
                self.painted.set(None);
                self.scroll = 0;
            }
            Event::Handed { line, applied } => {
                let style = if applied { Style::GOOD } else { Style::WARN };
                self.push(Entry::Notice(style, sanitize(&line, false)));
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
        self.pending.mark_all_done();
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
        self.push(entry);
        self.phase = Phase::Idle;
        if self.quitting {
            self.pending.clear_queue();
            return vec![Effect::Quit];
        }
        if let Some(next) = self.pending.pop_next() {
            self.push(Entry::User(next.clone()));
            self.phase = Phase::Working(None);
            self.started = self.now;
            self.said = None;
            self.scroll = 0;
            self.pane.begin();
            return vec![Effect::Start(next)];
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
            return answer(self, key, command, choice);
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
    pub(super) fn interrupt(&mut self, key: Key) -> Vec<Effect> {
        match self.phase {
            Phase::Working(ref command) => {
                self.phase = Phase::Stopping(command.is_some());
                vec![Effect::Interrupt]
            }
            Phase::Stopping(_) if key == Key::Interrupt => {
                self.quitting = true;
                self.hint = Some(self.quit_hint());
                Vec::new()
            }
            Phase::Idle | Phase::Asking(..) | Phase::Stopping(_) => Vec::new(),
        }
    }

    /// Enter: a session command typed whole with its argument, else the popup's
    /// selected command, else the text.
    fn enter(&mut self) -> Vec<Effect> {
        if let Some(command) = SessionCommand::parse(self.composer.text()) {
            return self.session(command);
        }
        if self.composer.selected().is_some() {
            return self.command();
        }
        self.submit()
    }

    /// Hands `command` to the driver. While a task runs the transcript is its
    /// own, so the command waits in the box and is never queued to the model.
    fn session(&mut self, command: SessionCommand) -> Vec<Effect> {
        if self.phase != Phase::Idle {
            self.hint = Some(String::from("Session commands wait until the task ends."));
            return Vec::new();
        }
        self.composer.take();
        vec![Effect::Session(command)]
    }

    /// Enter on the popup's selected slash command.
    fn command(&mut self) -> Vec<Effect> {
        let Some(name) = self.composer.selected() else {
            return self.submit();
        };
        if let Some(command) = SessionCommand::parse(name) {
            return self.session(command);
        }
        self.composer.take();
        match name {
            "/help" => self.push(Entry::Info(
                HELP.iter().map(|line| (*line).to_owned()).collect(),
            )),
            "/status" => self.push(Entry::Info(self.status.clone())),
            "/clear" => {
                self.transcript.clear_keeping_header();
                self.scroll = 0;
            }
            "/ps" => self.push(Entry::Info(self.pending.ps_rows())),
            "/stop" => return self.stop(),
            _ => return self.quit_or_leave(),
        }
        Vec::new()
    }

    /// Pushes the notice for a word starting `/` that is no command.
    fn unknown_command(&mut self, text: &str) {
        self.push(Entry::Notice(
            Style::WARN,
            format!("Unknown command {text}. Type / to see the commands."),
        ));
    }

    /// Enter on typed text: a task, a queued turn while a session runs, or a word
    /// starting `/` that is no command.
    fn submit(&mut self) -> Vec<Effect> {
        if self.composer.text().trim().is_empty() {
            return Vec::new();
        }
        if self.phase != Phase::Idle {
            let text = self.composer.text().to_owned();
            if text.starts_with('/') && !text.contains(char::is_whitespace) {
                if self.composer.selected().is_some() {
                    return self.command();
                }
                let unknown = self.composer.take();
                self.unknown_command(&unknown);
                return Vec::new();
            }
            if self.pending.queue_full() {
                self.hint = Some(format!(
                    "The queue is full ({QUEUE_BOUND}). The next turn sends first."
                ));
                return Vec::new();
            }
            let queued = self.composer.take();
            self.pending.push_turn(queued);
            return Vec::new();
        }
        if self.composer.text().trim().is_empty() {
            return Vec::new();
        }
        let text = self.composer.take();
        if text.starts_with('/') && !text.contains(char::is_whitespace) {
            self.unknown_command(&text);
            return Vec::new();
        }
        self.push(Entry::User(text.clone()));
        self.phase = Phase::Working(None);
        self.started = self.now;
        self.said = None;
        self.scroll = 0;
        self.pane.begin();
        vec![Effect::Start(text)]
    }
}
