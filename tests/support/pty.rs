//! The shipped `larry` binary on a real PTY, and the screen it paints (larry#239).
//!
//! `script(1)` supplies the terminal, 32x120, `TERM=xterm-256color`, with only the PATH built
//! from where its tools resolved, TERM and the caller's settings set. The screen model keeps the
//! terminal's rules: CUP moves the cursor, ED clears, OSC is skipped, printable text lands at the
//! cursor. Source: ECMA-48 (CSI, OSC), POSIX `script(1)` and `kill(2)`. `tests/e2e_journey.rs`
//! includes this file.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

/// Failures propagate instead of panicking.
pub(crate) type Journey<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[path = "deadline.rs"]
pub(crate) mod deadline;

/// The directories the tool lookup searches, in the order it prefers them: the places `sh`,
/// `git`, `script`, `stty`, `cp` and `sleep` live on macOS, Linux, Homebrew and NixOS. The host's
/// PATH and shell configuration are never read (larry#269, larry#296).
const TOOL_DIRS: [&str; 6] = [
    "/usr/bin",
    "/bin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/nix/var/nix/profiles/default/bin",
    "/run/current-system/sw/bin",
];
/// The tools the journeys' terminal runs: the two this module starts, and the four a journey's
/// command calls by name.
const TOOLS: [&str; 6] = ["sh", "git", "script", "stty", "cp", "sleep"];

/// Why the terminal's tools could not be resolved into a PATH: a typed setup refusal that names
/// the tool, so a host without one says so instead of failing later with "not found" (larry#296).
#[derive(Debug, lgwks_macros::Error)]
pub(crate) enum SetupError {
    /// No searched directory holds the tool.
    #[error(
        "the terminal needs `{tool}`, which these directories do not hold: {}",
        joined(searched)
    )]
    Missing {
        /// The tool's name.
        tool: &'static str,
        /// Every directory the lookup searched, in order.
        searched: Vec<PathBuf>,
    },
    /// A candidate's metadata could not be read.
    #[error(
        "the terminal cannot read the candidate `{}` for `{tool}`: {cause}",
        path.display()
    )]
    Unreadable {
        /// The tool being looked up.
        tool: &'static str,
        /// The candidate path.
        path: PathBuf,
        /// The operating system's refusal.
        #[source]
        cause: io::Error,
    },
}

/// The searched directories as one `:`-joined text.
fn joined(dirs: &[PathBuf]) -> String {
    let shown: Vec<String> = dirs.iter().map(|dir| dir.display().to_string()).collect();
    shown.join(":")
}

/// The executable for `tool` in the first of `search` that holds one: a plain stat of each
/// candidate, never `which` and never a shell, so no host configuration is read (larry#296).
fn find_tool(tool: &'static str, search: &[PathBuf]) -> Result<PathBuf, SetupError> {
    for dir in search {
        let candidate = dir.join(tool);
        match is_executable(&candidate) {
            Ok(true) => return Ok(candidate),
            Ok(false) => {}
            Err(cause) => {
                return Err(SetupError::Unreadable {
                    tool,
                    path: candidate,
                    cause,
                });
            }
        }
    }
    Err(SetupError::Missing {
        tool,
        searched: search.to_vec(),
    })
}

/// Whether `path` is a file some user may execute; a path that is not there is the plain case,
/// and any other refusal propagates.
fn is_executable(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt as _;
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.is_file() && meta.permissions().mode() & 0o111 != 0),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(cause) => Err(cause),
    }
}

/// The PATH every terminal's shell gets: the directories the tools were found in, resolved once
/// from [`TOOL_DIRS`], never the host's PATH (larry#269, larry#296).
pub(crate) fn test_path() -> Result<&'static str, SetupError> {
    static RESOLVED: OnceLock<String> = OnceLock::new();
    let resolved = match RESOLVED.get() {
        Some(path) => path,
        None => {
            let built = build_test_path()?;
            RESOLVED.get_or_init(move || built)
        }
    };
    Ok(resolved.as_str())
}

/// Builds the terminal's PATH from where each of [`TOOLS`] resolves, in [`TOOL_DIRS`]' order.
fn build_test_path() -> Result<String, SetupError> {
    let search: Vec<PathBuf> = TOOL_DIRS.iter().map(|dir| PathBuf::from(*dir)).collect();
    let mut found: Vec<PathBuf> = Vec::new();
    for tool in TOOLS {
        found.push(find_tool(tool, &search)?);
    }
    let dirs: Vec<String> = search
        .iter()
        .filter(|dir| {
            found
                .iter()
                .any(|tool| tool.parent() == Some(dir.as_path()))
        })
        .map(|dir| dir.display().to_string())
        .collect();
    Ok(dirs.join(":"))
}
/// Terminal rows.
const ROWS: usize = 32;
/// Terminal columns.
const COLUMNS: usize = 120;
/// Runaway-output bound: a 2026-09-19 8 fps frame-loop trial crossed it in under 30 seconds on
/// the same PTY. A crash closes the PTY and fails distinctly rather than reading as quiet.
const OUTPUT_BOUND: usize = 1_048_576;

/// The screen the terminal shows.
pub(crate) struct Screen {
    /// Each row's cells.
    cells: Vec<Vec<char>>,
    /// Cursor column.
    column: usize,
    /// Cursor row.
    row: usize,
    /// Bytes of an escape or character not yet complete.
    pending: Vec<u8>,
}

impl Screen {
    /// A blank 32x120 screen.
    pub(crate) fn new() -> Self {
        Self {
            cells: vec![vec![' '; COLUMNS]; ROWS],
            column: 0,
            row: 0,
            pending: Vec::new(),
        }
    }

    /// The visible text, rows joined by newlines.
    pub(crate) fn text(&self) -> String {
        let rows: Vec<String> = self.cells.iter().map(|row| row.iter().collect()).collect();
        rows.join("\n")
    }

    /// CUP: `row;column`, 1-based, each defaulting to 1, clamped to the screen.
    fn move_cursor(&mut self, params: &[u8]) {
        let mut fields = params.split(|&byte| byte == b';');
        let number = |field: Option<&[u8]>| -> usize {
            let parsed = field
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|text| text.parse::<usize>().ok());
            match parsed {
                Some(value) if value > 0 => value,
                Some(_) | None => 1,
            }
        };
        let row = number(fields.next());
        let column = number(fields.next());
        self.row = row.saturating_sub(1).min(ROWS.saturating_sub(1));
        self.column = column.saturating_sub(1).min(COLUMNS.saturating_sub(1));
    }

    /// Writes one printable character at the cursor and advances it, wrapping at the edge.
    fn put(&mut self, character: char) {
        if !character.is_control()
            && let Some(cell) = self
                .cells
                .get_mut(self.row)
                .and_then(|row| row.get_mut(self.column))
        {
            *cell = character;
        }
        self.column = self.column.saturating_add(1);
        if self.column >= COLUMNS {
            self.column = 0;
            self.row = self.row.saturating_add(1).min(ROWS.saturating_sub(1));
        }
    }

    /// The index just past the CSI or OSC sequence at `at`, after applying a CSI; `None` when
    /// the sequence is not complete yet.
    fn escape(&mut self, at: usize) -> Option<usize> {
        let start = at.checked_add(2)?;
        match *self.pending.get(at.checked_add(1)?)? {
            b'[' => {
                let rest = self.pending.get(start..)?;
                let offset = rest.iter().position(|byte| (0x40..=0x7e).contains(byte))?;
                let end = start.checked_add(offset)?;
                let params = self.pending.get(start..end)?.to_vec();
                match self.pending.get(end).copied() {
                    Some(b'H' | b'f') => self.move_cursor(&params),
                    Some(b'J') => self.cells = vec![vec![' '; COLUMNS]; ROWS],
                    Some(_) | None => {}
                }
                end.checked_add(1)
            }
            b']' => {
                let rest = self.pending.get(start..)?;
                let bell = rest.iter().position(|&byte| byte == 0x07);
                let terminator = rest.windows(2).position(|pair| pair == b"\x1b\\");
                let skipped = match (bell, terminator) {
                    (Some(bel), Some(st)) if st < bel => st.checked_add(2)?,
                    (Some(bel), _) => bel.checked_add(1)?,
                    (None, Some(st)) => st.checked_add(2)?,
                    (None, None) => rest.len(),
                };
                start.checked_add(skipped)
            }
            _ => at.checked_add(1),
        }
    }

    /// Applies `chunk` to the screen; an incomplete sequence waits for the next chunk.
    pub(crate) fn ingest(&mut self, chunk: &[u8]) {
        self.pending.extend_from_slice(chunk);
        let mut index = 0_usize;
        while let Some(&current) = self.pending.get(index) {
            if current == 0x1b {
                if self.pending.get(index.saturating_add(1)).is_none() {
                    break;
                }
                match self.escape(index) {
                    Some(next) => index = next,
                    None => break,
                }
                continue;
            }
            if current < 0x80 {
                match current {
                    b'\r' => self.column = 0,
                    b'\n' => self.row = self.row.saturating_add(1).min(ROWS.saturating_sub(1)),
                    0x20.. => self.put(char::from(current)),
                    _ => {}
                }
                index = index.saturating_add(1);
                continue;
            }
            let width = match current {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            let end = index.saturating_add(width);
            let Some(bytes) = self.pending.get(index..end) else {
                break;
            };
            let decoded = String::from_utf8_lossy(bytes).chars().next();
            if let Some(character) = decoded {
                self.put(character);
            }
            index = end;
        }
        self.pending.drain(..index);
    }
}

/// The shipped binary under `script(1)`, its output read by one thread the test's scope owns.
pub(crate) struct Pty<'scope> {
    /// The `script` process; the binary is its child, exec'd from the shell.
    child: Child,
    /// The terminal's input.
    stdin: Option<ChildStdin>,
    /// Output chunks from the reader thread.
    chunks: Receiver<Vec<u8>>,
    /// The reader thread, joined on finish, on drop, or by its scope.
    reader: Option<ScopedJoinHandle<'scope, std::io::Result<()>>>,
    /// Every byte the terminal wrote.
    raw: Vec<u8>,
    /// The screen those bytes paint.
    screen: Screen,
    /// Whether output arrived since the last keys were typed: a match only counts once the
    /// frame has answered, so text left on screen from before a key never passes for its reply.
    answered: bool,
    /// The screen when the last keys were typed, until a wait consumes it.
    before: Option<String>,
}

/// Whether `now` shows `text` as the reply to keys typed over `before`. Text already on the
/// screen before the keys is refused, because the first chunk of a late paint would pass for
/// the reply (larry#148 run 42366888296).
pub(crate) fn replied(before: Option<&str>, now: &str, text: &str) -> Result<bool, String> {
    if before.is_some_and(|screen| screen.contains(text)) {
        return Err(format!(
            "{text:?} was on screen before the keys; wait for text only the reply paints"
        ));
    }
    Ok(now.contains(text))
}

impl<'scope> Pty<'scope> {
    /// Starts the shipped binary in a fresh 32x120 terminal in `cwd`, with only the PATH its
    /// tools resolved into, TERM and `env` set. The shell prints its own PID before it execs the
    /// binary, so the PID is the binary's.
    pub(crate) fn open<'env>(
        owner: &'scope Scope<'scope, 'env>,
        cwd: &Path,
        env: &[(&str, OsString)],
    ) -> Journey<Self> {
        let path = test_path()?;
        let inner = format!(
            "stty rows {ROWS} cols {COLUMNS}; printf 'LARRYPID:%s:END' $$; exec '{}'",
            env!("CARGO_BIN_EXE_larry")
        );
        let mut command = Command::new("script");
        if cfg!(target_os = "linux") {
            command.args(["-qec", &inner, "/dev/null"]);
        } else {
            command.args(["-q", "/dev/null", "sh", "-c", &inner]);
        }
        command.env_clear().env("PATH", path).current_dir(cwd);
        for &(name, ref value) in env {
            command.env(name, value);
        }
        let mut child = command
            .env("TERM", "xterm-256color")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or("terminal has no stdin")?;
        let stdout = child.stdout.take().ok_or("terminal has no stdout")?;
        let (sender, chunks) = mpsc::sync_channel(64);
        let reader = owner.spawn(move || forward(stdout, &sender));
        Ok(Self {
            child,
            stdin: Some(stdin),
            chunks,
            reader: Some(reader),
            raw: Vec::new(),
            screen: Screen::new(),
            answered: false,
            before: None,
        })
    }

    /// Takes one chunk within `wait`: `Ok(false)` when none came, an error once the terminal
    /// closed or the output bound is crossed.
    fn take(&mut self, wait: Duration) -> Journey<bool> {
        match self.chunks.recv_timeout(wait) {
            Ok(chunk) => {
                self.screen.ingest(&chunk);
                self.raw.extend_from_slice(&chunk);
                self.answered = true;
                if self.raw.len() > OUTPUT_BOUND {
                    return Err("terminal output exceeded 1 MiB".into());
                }
                Ok(true)
            }
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => Err("terminal closed".into()),
        }
    }

    /// Reads until the frame has answered the last keys and the screen shows `text`, which
    /// must not have been on screen when those keys were typed.
    pub(crate) fn wait_for(&mut self, text: &str) -> Journey {
        let deadline = Instant::now()
            .checked_add(deadline::PATIENCE)
            .ok_or("deadline overflows")?;
        let before = self.before.take();
        while !(self.answered && replied(before.as_deref(), &self.screen.text(), text)?) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !self.take(left)? {
                return Err(format!("missing {text:?}; screen={:?}", self.screen.text()).into());
            }
        }
        Ok(())
    }

    /// The visible text of the screen right now, its rows joined by newlines.
    pub(crate) fn screen_text(&self) -> String {
        self.screen.text()
    }

    /// Reads whatever arrives for `window` and returns its byte count.
    pub(crate) fn drain(&mut self, window: Duration) -> Journey<usize> {
        let before = self.raw.len();
        let deadline = Instant::now()
            .checked_add(window)
            .ok_or("deadline overflows")?;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(self.raw.len().saturating_sub(before));
            }
            self.take(left)?;
        }
    }

    /// Types `keys`.
    pub(crate) fn send(&mut self, keys: &str) -> Journey {
        let stdin = self.stdin.as_mut().ok_or("terminal input closed")?;
        self.answered = false;
        self.before = Some(self.screen.text());
        stdin.write_all(keys.as_bytes())?;
        Ok(stdin.flush()?)
    }

    /// The binary's PID, as the shell printed it.
    pub(crate) fn pid(&self) -> Journey<String> {
        let text = String::from_utf8_lossy(&self.raw);
        let digits = text
            .split_once("LARRYPID:")
            .and_then(|(_, rest)| rest.split_once(":END"))
            .map(|(pid, _)| pid)
            .ok_or("the shell printed no PID")?;
        Ok(digits.parse::<u32>()?.to_string())
    }

    /// Reads to the end of the output, then reaps the terminal and its reader; returns the
    /// exit status and the whole transcript.
    pub(crate) fn finish(mut self) -> Journey<(ExitStatus, Vec<u8>)> {
        let deadline = Instant::now()
            .checked_add(deadline::PATIENCE)
            .ok_or("deadline overflows")?;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.chunks.recv_timeout(left) {
                Ok(chunk) => self.raw.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => return Err("the terminal did not close".into()),
            }
        }
        self.stdin = None;
        let status = self.child.wait()?;
        if let Some(reader) = self.reader.take() {
            match reader.join() {
                Ok(read) => read?,
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
                    return Err(format!("terminal reader thread panicked: {message:?}").into());
                }
            }
        }
        Ok((status, std::mem::take(&mut self.raw)))
    }
}

impl Drop for Pty<'_> {
    fn drop(&mut self) {
        // Best effort on an early return: kill and reap so no terminal outlives the test.
        self.stdin = None;
        let stopped = match self.child.try_wait() {
            Ok(Some(_status)) => Ok(()),
            Ok(None) => self
                .child
                .kill()
                .and_then(|()| self.child.wait().map(|_status| ())),
            Err(cause) => Err(cause),
        };
        let read_out = match self.reader.take() {
            Some(reader) => matches!(reader.join(), Ok(Ok(()))),
            None => true,
        };
        assert!(
            std::thread::panicking() || (stopped.is_ok() && read_out),
            "the terminal and its reader stop with the test: {stopped:?}, reader ok {read_out}"
        );
    }
}

/// Forwards the terminal's output in chunks until it closes or the receiver is gone.
fn forward(mut stdout: ChildStdout, sender: &SyncSender<Vec<u8>>) -> std::io::Result<()> {
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stdout.read(&mut buffer)?;
        let Some(chunk) = buffer.get(..read).filter(|bytes| !bytes.is_empty()) else {
            return Ok(());
        };
        if sender.send(chunk.to_vec()).is_err() {
            return Ok(());
        }
    }
}

/// A reply is text the keys brought: text already on screen before them is refused, whatever
/// the screen shows now; with no keys typed since the last wait, presence alone answers.
#[test]
fn a_wait_refuses_text_that_was_on_screen_before_the_keys() {
    let board = "Review retry boundary  Workspace handoff";
    let opened = "HOME  /  MOO  /  Workspace handoff";
    assert_eq!(
        replied(Some(board), opened, "MOO  /  Workspace handoff"),
        Ok(true)
    );
    assert_eq!(
        replied(Some(board), board, "MOO  /  Workspace handoff"),
        Ok(false)
    );
    assert!(replied(Some(board), opened, "Workspace handoff").is_err());
    assert_eq!(replied(None, board, "Workspace handoff"), Ok(true));
    assert_eq!(replied(None, board, "remember this"), Ok(false));
}

/// A tool lookup finds a `git` under a prefix of its own and refuses a missing one by name, so a
/// host that keeps git outside the standard directories says which tool it lacks instead of
/// failing later with "not found" (larry#296). The prefix is the integration tests' one temp
/// guard; nothing here reads the clock, the host's PATH or its shell configuration.
#[test]
fn a_tool_lookup_finds_a_git_under_its_own_prefix_and_names_a_missing_one()
-> Result<(), Box<dyn std::error::Error>> {
    let prefix = crate::origin::TempDir::new()?;
    let bin = prefix.path().join("bin");
    std::fs::create_dir_all(&bin)?;
    let git = bin.join("git");
    std::fs::write(&git, "#!/bin/sh\nexit 0\n")?;
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755))?;
    }
    let search = [bin, PathBuf::from("/usr/bin")];
    match find_tool("git", &search) {
        Ok(found) => assert_eq!(found, git, "the lookup finds the git under its own prefix"),
        Err(cause) => return Err(format!("the lookup refused a present git: {cause}").into()),
    }
    let empty = prefix.path().join("empty");
    std::fs::create_dir_all(&empty)?;
    let Err(refusal) = find_tool("git", &[empty]) else {
        return Err("the lookup found a git where no directory holds one".into());
    };
    match refusal {
        SetupError::Missing { tool: "git", .. } => {}
        SetupError::Missing { tool, .. } => {
            return Err(format!("the refusal names `{tool}`, not git").into());
        }
        SetupError::Unreadable {
            tool, ref cause, ..
        } => {
            return Err(format!("the lookup could not read `{tool}`: {cause}").into());
        }
    }
    assert!(
        refusal.to_string().contains("git"),
        "the refusal names the tool: {refusal}"
    );
    Ok(())
}

/// The screen model places text by CUP, clears on ED, skips OSC by BEL or ST, decodes UTF-8
/// split across chunks and wraps at the right edge.
#[test]
fn the_screen_model_follows_the_terminal_rules() {
    let mut screen = Screen::new();
    screen.ingest(b"\x1b]0;title\x07\x1b[2;3Hab\x1b]8;;x\x1b\\c\xc3");
    screen.ingest(b"\xa9");
    let text = screen.text();
    let second = text.lines().nth(1).map(str::trim_end);
    assert_eq!(second, Some("  abc\u{e9}"), "row 2: {text:?}");
    screen.ingest(b"\x1b[J\x1b[1;120Hxy");
    let wrapped = screen.text();
    let first = wrapped.lines().next().map(str::trim);
    assert_eq!(first, Some("x"), "x at the edge: {wrapped:?}");
    let next = wrapped.lines().nth(1).map(str::trim);
    assert_eq!(next, Some("y"), "y wraps: {wrapped:?}");
}
