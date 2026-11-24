//! `larry` with no arguments: the human frontend (larry#239). The Director,
//! 2026-11-16: "all changes will be routed through a completed human frontend
//! (in tui, then Mac app then iOS and android apps)".
//!
//! One full-screen frame on the alternate screen: a phone-shaped chat column on
//! the left runs real terminal sessions (`executor::run_session`) and asks the
//! person before every command; the rest of a wide screen is kept for artifacts.
//! The machine in [`app`] holds all state and reads no clock; this driver owns
//! the terminal (raw mode, bracketed paste, the alternate screen, restored on
//! every exit path by [`Terminal`]'s drop), feeds it keys, sizes, ticks and the
//! session's events, and writes only the cells each frame changed ([`buffer`]).

mod app;
mod ask;
mod buffer;
mod cells;
mod composer;
mod layout;
mod pane;
#[cfg(test)]
mod sim_parts;
#[cfg(test)]
mod sim_tui;
mod style;
mod worker;

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use crate::executor::{ExecError, Isolation, Settings, process_env};
use app::{App, Effect, Input, Key};
use buffer::{Frame, render};
use cells::Header;
use worker::{Ended, Event, Worker};

/// How long the loop waits for a key while a session runs: the spinner's step.
const BUSY_WAIT: Duration = Duration::from_millis(100);
/// How long the loop waits for a key while idle; nothing repaints meanwhile.
const IDLE_WAIT: Duration = Duration::from_secs(1);
/// Columns of the reason `/status` gives for commands that are off.
const STATUS_REASON: usize = 100;
/// Session events one loop turn takes before it paints, so a burst cannot starve keys.
const EVENTS_PER_TURN: usize = worker::EVENTS;

/// Why the frontend stopped.
#[derive(Debug)]
#[non_exhaustive]
pub enum TuiError {
    /// The terminal refused raw mode, a size, a read or a write.
    Terminal(io::Error),
    /// The working directory could not be read.
    Directory(io::Error),
    /// The session running at quit had stopped on a panic; the text names it (larry#412).
    Session(String),
}

impl fmt::Display for TuiError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Terminal(ref cause) => write!(out, "terminal: {cause}"),
            Self::Directory(ref cause) => write!(out, "working directory: {cause}"),
            Self::Session(ref why) => write!(out, "session: {why}"),
        }
    }
}

impl std::error::Error for TuiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Terminal(ref cause) | Self::Directory(ref cause) => Some(cause),
            Self::Session(_) => None,
        }
    }
}

/// The terminal in raw mode on the alternate screen; dropping it restores both.
struct Terminal {
    /// Standard output.
    out: io::Stdout,
    /// Whether the terminal is still in raw mode.
    raw: bool,
}

/// What entering the frame writes: the alternate screen, bracketed paste, no cursor.
const ENTER: &[u8] = b"\x1b[?1049h\x1b[?2004h\x1b[?25l";
/// What leaving writes: plain style, the cursor, no paste mode, the main screen.
const LEAVE: &[u8] = b"\x1b[0m\x1b[?25h\x1b[?2004l\x1b[?1049l";

impl Terminal {
    /// Enters raw mode and the alternate screen.
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut held = Self {
            out: io::stdout(),
            raw: true,
        };
        held.out.write_all(ENTER)?;
        held.out.flush()?;
        Ok(held)
    }

    /// Writes `bytes` and flushes.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.out.flush()
    }

    /// Leaves the alternate screen and raw mode, reporting a failure.
    fn leave(&mut self) -> io::Result<()> {
        if !self.raw {
            return Ok(());
        }
        self.raw = false;
        let written = self.write(LEAVE);
        terminal::disable_raw_mode().and(written)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Every exit path but `leave` lands here: an error return or an unwind.
        // A terminal that cannot be written any more has nothing left to restore.
        if self.leave().is_err() {
            self.raw = false;
        }
    }
}

/// The frame's key for a terminal key, if it has one.
fn key(event: KeyEvent) -> Option<Key> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    let control = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    let shift = event.modifiers.contains(KeyModifiers::SHIFT);
    Some(match event.code {
        KeyCode::Char(character) if control => match character {
            'c' => Key::Interrupt,
            'd' => Key::EndOfInput,
            'u' => Key::KillStart,
            'k' => Key::KillEnd,
            'w' => Key::KillWord,
            'a' => Key::Home,
            'e' => Key::End,
            'j' => Key::NewLine,
            'h' => Key::Backspace,
            _ => return None,
        },
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Enter if alt || shift => Key::NewLine,
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Tab => Key::Tab,
        KeyCode::Esc => Key::Esc,
        _ => return None,
    })
}

/// `path` with the home directory written `~`.
fn tilde(path: &Path, home: Option<&str>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => String::from("~"),
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

/// The welcome box and the `/status` lines for `settings` in `cwd`.
fn header(
    settings: &Result<Settings, ExecError>,
    cwd: &Path,
    home: Option<&str>,
) -> (Header, Vec<String>) {
    let directory = tilde(cwd, home);
    let (model, blocked) = match *settings {
        Ok(ref settings) => {
            let model = match settings.arm_route() {
                Ok(route) => Ok(route.model().to_owned()),
                Err(cause) => Err(format!("none ({cause})")),
            };
            (model, Isolation::workspace().blocked(settings))
        }
        Err(ref cause) => (Err(String::from("none")), Some(cause.to_string())),
    };
    let status = vec![
        format!(
            "model: {}",
            match model {
                Ok(ref name) => name.as_str(),
                Err(ref why) => why.as_str(),
            }
        ),
        format!("directory: {directory}"),
        match blocked {
            Some(ref why) => format!(
                "commands: off ({})",
                style::ellipsize(&style::sanitize(why, false), STATUS_REASON)
            ),
            None => String::from("commands: asked before each one, run in an isolated copy"),
        },
    ];
    let header = Header {
        version: String::from(env!("CARGO_PKG_VERSION")),
        model,
        directory,
        blocked,
    };
    (header, status)
}

/// Runs the frontend until the person quits.
///
/// # Errors
///
/// [`TuiError`] when the terminal or the working directory fails, or when the
/// session still running at quit had stopped on a panic; every other failure of
/// a session is shown in the frame, never returned.
pub fn run() -> Result<(), TuiError> {
    let cwd = std::env::current_dir().map_err(TuiError::Directory)?;
    // A refused environment is the settings' error, shown in the welcome box.
    let (settings, home) = match process_env() {
        Ok(read) => {
            let home = read("HOME");
            (Settings::from_env(read), home)
        }
        Err(cause) => (Err(cause), None),
    };
    let (welcome, status) = header(&settings, &cwd, home.as_deref());
    let (width, height) = terminal::size().map_err(TuiError::Terminal)?;
    let mut app = App::new(welcome, status, usize::from(width), usize::from(height));
    // Declared before the terminal, so an unwind restores the screen before the worker's
    // drop reaps the session.
    let mut worker: Option<Worker> = None;
    let mut screen = Terminal::enter().map_err(TuiError::Terminal)?;
    let outcome = drive(&mut app, &mut screen, &mut worker, &settings, &cwd);
    // Every return reaps the session here, so its panic is reported, never dropped (larry#412).
    let panicked = match worker.take().and_then(Worker::finish) {
        Some(why) => Err(TuiError::Session(why)),
        None => Ok(()),
    };
    let left = screen.leave().map_err(TuiError::Terminal);
    outcome.and(panicked).and(left)
}

/// The loop: paint, wait for a key or a tick, take the session's events, act.
fn drive(
    app: &mut App,
    screen: &mut Terminal,
    worker: &mut Option<Worker>,
    settings: &Result<Settings, ExecError>,
    cwd: &Path,
) -> Result<(), TuiError> {
    let clock = Instant::now();
    let mut shown: Option<Frame> = None;
    let mut bytes = Vec::new();
    loop {
        let frame = app.draw();
        bytes.clear();
        render(shown.as_ref(), &frame, &mut bytes);
        if !bytes.is_empty() {
            screen.write(&bytes).map_err(TuiError::Terminal)?;
        }
        shown = Some(frame);
        let wait = if app.busy() { BUSY_WAIT } else { IDLE_WAIT };
        let mut effects = Vec::new();
        if event::poll(wait).map_err(TuiError::Terminal)? {
            let input = match event::read().map_err(TuiError::Terminal)? {
                TermEvent::Key(pressed) => key(pressed).map(Input::Key),
                TermEvent::Paste(text) => Some(Input::Paste(text)),
                TermEvent::Resize(columns, rows) => {
                    Some(Input::Resize(usize::from(columns), usize::from(rows)))
                }
                TermEvent::FocusGained | TermEvent::FocusLost | TermEvent::Mouse(_) => None,
            };
            if let Some(input) = input {
                effects.extend(app.handle(input));
            }
        }
        for _ in 0..EVENTS_PER_TURN {
            let Some(event) = worker.as_mut().and_then(Worker::next) else {
                break;
            };
            let last = matches!(event, Event::Ended(_));
            effects.extend(app.handle(Input::Worker(event)));
            if last {
                // The session ended: reap its thread now.
                *worker = None;
            }
        }
        effects.extend(app.handle(Input::Tick(clock.elapsed())));
        for effect in effects {
            match effect {
                Effect::Quit => return Ok(()),
                Effect::Start(instruction) => {
                    *worker = start(app, settings, instruction, cwd);
                }
                Effect::Decide(decision) => {
                    if let Some(running) = worker.as_ref() {
                        running.decide(decision);
                    }
                }
                Effect::Interrupt => {
                    if let Some(running) = worker.as_ref() {
                        running.interrupt();
                    }
                }
            }
        }
    }
}

/// Starts a session, or tells the frame why it cannot.
fn start(
    app: &mut App,
    settings: &Result<Settings, ExecError>,
    instruction: String,
    cwd: &Path,
) -> Option<Worker> {
    let failed = match *settings {
        Ok(ref settings) => {
            let dir: PathBuf = cwd.to_path_buf();
            let isolation = Isolation::workspace();
            match Worker::spawn(instruction, dir, settings.clone(), isolation, app.always()) {
                Ok(running) => return Some(running),
                Err(cause) => format!("Larry could not start the session: {cause}"),
            }
        }
        Err(ref cause) => cause.to_string(),
    };
    app.handle(Input::Worker(Event::Ended(Ended::Failed(failed))));
    None
}
