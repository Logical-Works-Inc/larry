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
use std::collections::VecDeque;
use std::time::Duration;

use super::ask::{fit, marked, offer};
use super::buffer::{Buffer, Frame};
use super::cells::{Entry, Header, split_observation};
use super::composer::Composer;
use super::layout::split;
use super::pane::Pane;
use super::style::{Line, Span, Style, ellipsize, sanitize, sanitize_shown, text_width, wrap};
use super::worker::{Decision, Ended, Event};

/// Most transcript entries kept; the oldest goes first.
pub(crate) const MAX_ENTRIES: usize = 512;
/// The spinner's frames, one per 100 ms.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// What `/help` prints.
const HELP: [&str; 4] = [
    "Enter sends the task. Ctrl+J adds a new line.",
    "Larry asks before each command: y runs it, a runs it and stops asking, n declines.",
    "Esc stops a task. PgUp and PgDn scroll. Ctrl+C on an empty box quits.",
    "/status shows the model, policy and folder. /clear clears the chat.",
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
}

/// Where the machine is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
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
    /// The transcript, oldest first.
    entries: VecDeque<Entry>,
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

/// `elapsed` the way Codex's status row shows it: `12s`, `1m 05s`, `1h 02m`.
pub(crate) fn compact(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match (seconds.checked_div(60), seconds.checked_rem(60)) {
        (Some(0) | None, _) => format!("{seconds}s"),
        (Some(minutes), Some(rest)) if minutes < 60 => format!("{minutes}m {rest:02}s"),
        (Some(minutes), _) => match (minutes.checked_div(60), minutes.checked_rem(60)) {
            (Some(hours), Some(left)) => format!("{hours}h {left:02}m"),
            _ => format!("{minutes}m"),
        },
    }
}

/// Footer hints joined by ` · ` to fit `width`; a hint that does not fit is
/// dropped whole and `…` marks the cut (Codex `footer_hint.rs`).
pub(crate) fn hints(items: &[&str], width: usize) -> Line {
    let mut text = String::new();
    for (index, item) in items.iter().enumerate() {
        let sep = if index == 0 { "" } else { " · " };
        let more = index.saturating_add(1) < items.len();
        let next = text_width(&text)
            .saturating_add(text_width(sep))
            .saturating_add(text_width(item));
        if next.saturating_add(if more { 2 } else { 0 }) > width {
            text.push_str(if index == 0 { "…" } else { " …" });
            break;
        }
        text.push_str(sep);
        text.push_str(item);
    }
    vec![Span::new(text, Style::MUTED)]
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
            entries: VecDeque::from([Entry::Header(header)]),
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

    /// Whether the person said to stop asking for this run.
    pub(crate) const fn always(&self) -> bool {
        self.always
    }

    /// Adds `entry`, dropping the oldest past [`MAX_ENTRIES`].
    fn push(&mut self, entry: Entry) {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// The transcript entries, oldest first; the bound sims read it.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &VecDeque<Entry> {
        &self.entries
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
            Event::Receipt(receipt) => self.pane.set_receipt(receipt),
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
        self.push(entry);
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
                self.scroll = self
                    .scroll
                    .saturating_add(self.page())
                    .min(self.max_scroll());
                return Vec::new();
            }
            Key::PageDown => {
                self.scroll = self.scroll.saturating_sub(self.page());
                return Vec::new();
            }
            Key::Esc | Key::Interrupt if self.phase != Phase::Idle => return self.interrupt(key),
            Key::Esc if self.composer.popup().is_some() => self.composer.clear(),
            Key::Interrupt if !self.composer.is_empty() => self.composer.clear(),
            Key::Interrupt | Key::EndOfInput if self.composer.is_empty() => {
                return vec![Effect::Quit];
            }
            Key::Enter if self.composer.selected().is_some() => return self.command(),
            Key::Enter => return self.submit(),
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
                .for_each(|command| self.push(Entry::Declined(command))),
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

    /// Enter on the popup's selected slash command.
    fn command(&mut self) -> Vec<Effect> {
        let Some(name) = self.composer.selected() else {
            return self.submit();
        };
        self.composer.take();
        match name {
            "/help" => self.push(Entry::Info(
                HELP.iter().map(|line| (*line).to_owned()).collect(),
            )),
            "/status" => self.push(Entry::Info(self.status.clone())),
            "/clear" => {
                let header = self
                    .entries
                    .iter()
                    .find(|entry| matches!(**entry, Entry::Header(_)))
                    .cloned();
                self.entries = header.into_iter().collect();
                self.scroll = 0;
            }
            _ => return vec![Effect::Quit],
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
            self.push(Entry::Notice(
                Style::WARN,
                format!("Unknown command {text}. Type / to see the commands."),
            ));
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

    /// Rows one Page Up or Page Down moves.
    fn page(&self) -> usize {
        self.height.checked_div(2).into_iter().sum::<usize>().max(1)
    }

    /// The chat column's inner width: one cell of margin each side.
    fn inner(&self) -> usize {
        split(self.width, self.height)
            .chat
            .width
            .saturating_sub(2)
            .max(1)
    }

    /// Every transcript row at the inner width, a blank row between entries.
    fn transcript(&self) -> Vec<Line> {
        let width = self.inner();
        let mut rows = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                rows.push(Vec::new());
            }
            rows.extend(entry.lines(width));
        }
        rows
    }

    /// The most rows the transcript can scroll up.
    fn max_scroll(&self) -> usize {
        self.transcript().len()
    }

    /// The status row while a session runs.
    fn status_row(&self) -> Option<Line> {
        let tick = self
            .now
            .as_millis()
            .checked_div(100)
            .and_then(|step| step.checked_rem(10))
            .and_then(|step| usize::try_from(step).ok());
        let spin = match tick.and_then(|step| SPINNER.get(step)) {
            Some(frame) => *frame,
            None => "•",
        };
        let elapsed = compact(self.now.saturating_sub(self.started));
        let running = format!(" ({elapsed} • esc to interrupt)");
        let (label, tail) = match self.phase {
            Phase::Idle | Phase::Asking(..) => return None,
            Phase::Working(None) => (String::from("Working"), running),
            Phase::Working(Some(ref command)) => {
                let first: String = command.lines().take(1).collect();
                (format!("Running {first}"), running)
            }
            Phase::Stopping(true) => (
                String::from("Stopping"),
                String::from(" (the running command ends first)"),
            ),
            Phase::Stopping(false) => (String::from("Stopping"), String::new()),
        };
        let room = self
            .inner()
            .saturating_sub(2)
            .saturating_sub(text_width(&tail));
        Some(vec![
            Span::new(format!("{spin} "), Style::ACCENT),
            Span::new(
                ellipsize(&sanitize(&label, false), room),
                Style::PLAIN.bold(),
            ),
            Span::new(tail, Style::MUTED),
        ])
    }

    /// The bottom block (status, popup or question, composer, footer), the cursor's column and
    /// row in it, and the leading rows a crop must keep (the question's title and first row).
    fn bottom(&self, width: usize) -> (Vec<Line>, Option<(usize, usize)>, usize) {
        let mut rows: Vec<Line> = Vec::new();
        if let Some(status) = self.status_row() {
            rows.push(status);
        }
        // A screen too narrow for the pane keeps the change as one line (larry#244); while a
        // question is open its title and first command row outrank that line (larry#325).
        if !matches!(self.phase, Phase::Asking(..))
            && split(self.width, self.height).artifact.is_none()
            && let Some(summary) = self.pane.summary()
        {
            rows.push(vec![Span::new(ellipsize(&summary, width), Style::MUTED)]);
        }
        let mut cursor = None;
        let mut keep = 0_usize;
        let footer: &[&str] = if let Phase::Asking(ref command, choice) = self.phase {
            let (title, answers, keys) = offer(command.is_some());
            rows.push(vec![Span::new(title, Style::WARN.bold())]);
            let body = rows.len();
            let lead = vec![Span::new("$ ", Style::MUTED)];
            let rest = vec![Span::new("  ", Style::PLAIN)];
            for text in command.iter().map(|command| sanitize_shown(command)) {
                for line in text.split_terminator('\n') {
                    rows.extend(wrap(&[Span::new(line, Style::PLAIN)], width, &lead, &rest));
                }
            }
            keep = if rows.len() > body {
                body.saturating_add(1)
            } else {
                body
            };
            rows.push(
                answers
                    .iter()
                    .map(|&(decision, label)| {
                        let (mark, style) = marked(decision == choice);
                        Span::new(format!("{mark} {label}  "), style)
                    })
                    .collect(),
            );
            keys
        } else {
            if let Some(popup) = self.composer.popup() {
                for (index, (name, about)) in popup.into_iter().enumerate() {
                    let (mark, style) = marked(index == self.composer.selected_row());
                    rows.push(vec![
                        Span::new(format!("{mark} {name}  "), style),
                        Span::new(about, Style::MUTED),
                    ]);
                }
            }
            rows.push(vec![Span::new("─".repeat(width), Style::MUTED)]);
            let (lines, (column, row)) = self.composer.render(width, "Ask Larry to do something");
            cursor = Some((column, rows.len().saturating_add(row)));
            rows.extend(lines);
            if self.phase == Phase::Idle {
                &["enter send", "/ commands", "ctrl+j newline", "ctrl+c quit"]
            } else {
                &["esc interrupt", "pgup/pgdn scroll"]
            }
        };
        let foot = match self.hint {
            Some(ref hint) => vec![Span::new(ellipsize(hint, width), Style::WARN)],
            None if self.scroll > 0 => {
                let mut items = vec!["↓ pgdn newer"];
                items.extend_from_slice(footer);
                hints(&items, width)
            }
            None => hints(footer, width),
        };
        rows.push(foot);
        (rows, cursor, keep)
    }

    /// The frame for the current state.
    pub(crate) fn draw(&self) -> Frame {
        if self.painted.get().is_none() && matches!(self.phase, Phase::Asking(..)) {
            self.painted.set(Some(self.frames.get()));
        }
        self.frames.set(self.frames.get().saturating_add(1));
        let mut buffer = Buffer::new(self.width, self.height);
        let regions = split(buffer.width(), buffer.height());
        let chat = regions.chat;
        let left = chat.x.saturating_add(1);
        let right = chat.right().saturating_sub(1).max(left.saturating_add(1));
        let width = right.saturating_sub(left);
        let (bottom, cursor, keep) = self.bottom(width);
        let bottom = fit(bottom, keep, chat.height);
        let bottom_rows = bottom.len().min(chat.height);
        let area = chat.height.saturating_sub(bottom_rows);
        let rows = self.transcript();
        let lift = self.scroll.min(rows.len().saturating_sub(area));
        let end = rows.len().saturating_sub(lift);
        let start = end.saturating_sub(area);
        for (offset, line) in rows.get(start..end).into_iter().flatten().enumerate() {
            buffer.put_line(left, chat.y.saturating_add(offset), right, line);
        }
        let top = chat.y.saturating_add(area);
        let skip = bottom.len().saturating_sub(bottom_rows);
        for (offset, line) in bottom.iter().skip(skip).enumerate() {
            buffer.put_line(left, top.saturating_add(offset), right, line);
        }
        if let Some(pane) = regions.artifact {
            self.pane.paint(&mut buffer, pane);
        }
        let cursor = cursor.and_then(|(column, row)| {
            let y = top.saturating_add(row.checked_sub(skip)?);
            let x = left.saturating_add(column);
            (x < buffer.width() && y < buffer.height()).then_some((x, y))
        });
        Frame { buffer, cursor }
    }
}
