//! The shipped `larry` binary driven through its user journeys on a real PTY.
//!
//! `script(1)` supplies the terminal, 32x120, `TERM=xterm-256color`. The first journey types
//! every slash command a user does and checks the transcript in order; the others port
//! `tools/test_native_cli.py` and `tools/probe_workspace.py` (larry#53 step 2, #147) onto a
//! screen model of the same rules: CUP moves the cursor, ED clears, OSC is skipped, printable
//! text lands at the cursor. Source: ECMA-48 (CSI, OSC), xterm SGR mouse reporting, POSIX
//! `kill(2)`. Requirement: the view paints, takes keys and the mouse, survives kill -9 with a
//! clean next start, stays quiet while idle, and exits 0 on quit.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

/// Failures propagate instead of panicking.
type Journey<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Upper bound on each wait; it only bounds a failure, never orders events.
const PATIENCE: Duration = Duration::from_secs(30);
/// Terminal rows.
const ROWS: usize = 32;
/// Terminal columns.
const COLUMNS: usize = 120;
/// Runaway-output bound: a 2026-09-19 8 fps frame-loop trial crossed it in under 30 seconds on
/// the same PTY. A crash closes the PTY and fails distinctly rather than reading as quiet.
const OUTPUT_BOUND: usize = 1_048_576;
/// The idle window the probe watches, the Python probe's minimum.
const IDLE: Duration = Duration::from_secs(5);
/// Input round trips the probe times.
const ROUND_TRIPS: usize = 20;

/// The commands typed once the workspace is open, in order.
const STEPS: [&str; 13] = [
    "/folders",
    "/agents",
    "/changes",
    "/next",
    "/inspect",
    "/feedback keep bounded",
    "/next",
    "/next",
    "/continue",
    "/fixture-continue",
    "/next",
    "/scenario empty",
    "/quit",
];

/// The boot screen's text; nothing is typed before it shows.
const BOOT_TEXT: &str = "Everything else stays quiet";
/// The workspace's text.
const WORKSPACE_TEXT: &str = "Awaiting Moo";
/// The boot screen's marker in the normalized transcript.
const BOOT: &str = "everythingelsestaysquiet";
/// The workspace's marker in the normalized transcript.
const WORKSPACE: &str = "awaitingmoo";

/// Markers over the normalized transcript, in journey order. Two pin rendering facts, not
/// typos: the CHANGES header emits no N byte on this path (stale-cell retention paints it), so
/// the stream reads "chagesfixture"; the fixture-03 step selects src/retry.rs:18, so its
/// marker is the file line rather than the fixture id.
const MARKERS: [&str; 13] = [
    BOOT,
    WORKSPACE,
    "filesfixture",
    "agentsfixture",
    "chagesfixture",
    "srcretryrs12",
    "tracefixture0",
    "heldclone1",
    "testsretryrs8",
    "heldwork",
    "nobackendactionexecuted",
    "srcretryrs18",
    "nochangesyet",
];

/// The screen the terminal shows, kept by the rules the Python suite used.
struct Screen {
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
    fn new() -> Self {
        Self {
            cells: vec![vec![' '; COLUMNS]; ROWS],
            column: 0,
            row: 0,
            pending: Vec::new(),
        }
    }

    /// The visible text, rows joined by newlines.
    fn text(&self) -> String {
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
    fn ingest(&mut self, chunk: &[u8]) {
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
struct Pty<'scope> {
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
    /// view has answered, so text left on screen from before a key never passes for its reply.
    answered: bool,
    /// The screen when the last keys were typed, until a wait consumes it.
    before: Option<String>,
}

/// Whether `now` shows `text` as the reply to keys typed over `before`. Text already on the
/// screen before the keys is refused, because the first chunk of a late paint would pass for
/// the reply: on hosted ubuntu the board's card title "Workspace handoff" did, the next Esc
/// went out mid-paint and merged with the following key (larry#148 run 42366888296).
fn replied(before: Option<&str>, now: &str, text: &str) -> Result<bool, String> {
    if before.is_some_and(|screen| screen.contains(text)) {
        return Err(format!(
            "{text:?} was on screen before the keys; wait for text only the reply paints"
        ));
    }
    Ok(now.contains(text))
}

impl<'scope> Pty<'scope> {
    /// Starts the shipped binary in a fresh 32x120 terminal. The shell prints its own PID
    /// before it execs the binary, so the PID is the binary's.
    fn open<'env>(owner: &'scope Scope<'scope, 'env>) -> Journey<Self> {
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

    /// Reads until the view has answered the last keys and the screen shows `text`, which must
    /// not have been on screen when those keys were typed.
    fn wait_for(&mut self, text: &str) -> Journey {
        let deadline = Instant::now()
            .checked_add(PATIENCE)
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

    /// Reads whatever arrives for `window` and returns its byte count.
    fn drain(&mut self, window: Duration) -> Journey<usize> {
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
    fn send(&mut self, keys: &str) -> Journey {
        let stdin = self.stdin.as_mut().ok_or("terminal input closed")?;
        self.answered = false;
        self.before = Some(self.screen.text());
        stdin.write_all(keys.as_bytes())?;
        Ok(stdin.flush()?)
    }

    /// The binary's PID, as the shell printed it.
    fn pid(&self) -> Journey<String> {
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
    fn finish(mut self) -> Journey<(ExitStatus, Vec<u8>)> {
        let deadline = Instant::now()
            .checked_add(PATIENCE)
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
            reader
                .join()
                .map_err(|_| "terminal reader thread panicked")??;
        }
        Ok((status, std::mem::take(&mut self.raw)))
    }
}

impl Drop for Pty<'_> {
    fn drop(&mut self) {
        // Best effort on an early return: kill and reap so no terminal outlives the test.
        self.stdin = None;
        drop(self.child.kill());
        drop(self.child.wait());
        if let Some(reader) = self.reader.take() {
            drop(reader.join());
        }
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

/// Lowercased ASCII alphanumerics of `raw`, with escape sequences removed.
fn normalize(raw: &[u8]) -> String {
    let mut out = String::new();
    let mut at = 0_usize;
    while let Some(&byte) = raw.get(at) {
        at = at.saturating_add(1);
        if byte == 0x1b {
            at = escape_end(raw, at);
        } else if byte.is_ascii_alphanumeric() {
            out.push(char::from(byte.to_ascii_lowercase()));
        }
    }
    out
}

/// Index just past the escape sequence whose introducer byte is at `start`: OSC to BEL, CSI
/// parameters to a final letter, a charset designation, or one byte. A truncated sequence ends
/// where the input does.
fn escape_end(raw: &[u8], start: usize) -> usize {
    let after = start.saturating_add(1);
    let Some(rest) = raw.get(after..) else {
        return start;
    };
    match raw.get(start).copied() {
        Some(b']') => match rest.iter().position(|&byte| byte == 0x07) {
            Some(bell) => after.saturating_add(bell).saturating_add(1),
            None => after,
        },
        Some(b'[') => {
            let params = rest
                .iter()
                .take_while(|&&byte| byte.is_ascii_digit() || byte == b';' || byte == b'?')
                .count();
            let end = after.saturating_add(params);
            if raw.get(end).is_some_and(u8::is_ascii_alphabetic) {
                end.saturating_add(1)
            } else {
                after
            }
        }
        Some(b'(' | b')')
            if rest
                .first()
                .is_some_and(|&byte| byte.is_ascii_digit() || byte == b'A' || byte == b'B') =>
        {
            after.saturating_add(1)
        }
        Some(_) => after,
        None => start,
    }
}

/// The whole journey through the shipped binary on a real PTY.
#[test]
fn e2e_journey_through_the_shipped_binary() -> Journey {
    std::thread::scope(|owner| -> Journey {
        let mut pty = Pty::open(owner)?;
        pty.wait_for(BOOT_TEXT)?;
        pty.send("\r")?;
        pty.wait_for(WORKSPACE_TEXT)?;
        let typed: String = STEPS.iter().map(|step| format!("{step}\r")).collect();
        pty.send(&typed)?;
        let (status, raw) = pty.finish()?;
        assert!(
            status.success(),
            "the binary exits 0 after /quit, got {status}"
        );
        let clean = normalize(&raw);
        let mut from = 0_usize;
        for marker in MARKERS {
            let found = clean
                .get(from..)
                .and_then(|rest| rest.find(marker))
                .ok_or_else(|| format!("marker missing in journey order: {marker}"))?;
            from = from.saturating_add(found).saturating_add(marker.len());
        }
        Ok(())
    })
}

/// The stripper removes OSC, CSI, charset and single-byte escapes, keeps the text between
/// them, and survives a truncated sequence at the end.
#[test]
fn e2e_journey_normalize_strips_escapes() {
    assert_eq!(
        normalize(b"\x1b]0;title\x07Ab\x1b[1;31mC\x1b(Bd\x1b=e-f\x1b["),
        "abcdef",
        "escapes are removed and the text between them kept"
    );
    assert_eq!(normalize(b"\x1b"), "", "a lone escape yields nothing");
}

/// A reply is text the keys brought: text already on screen before them is refused, whatever
/// the screen shows now; with no keys typed since the last wait, presence alone answers.
#[test]
fn a_wait_refuses_text_that_was_on_screen_before_the_keys() {
    let board = "Review retry boundary  Workspace handoff";
    let opened = "HOME  /  MOO  /  Workspace handoff";
    assert_eq!(
        replied(Some(board), opened, "MOO  /  Workspace handoff"),
        Ok(true),
        "the breadcrumb is new: the reply"
    );
    assert_eq!(
        replied(Some(board), board, "MOO  /  Workspace handoff"),
        Ok(false),
        "not painted yet: keep reading"
    );
    assert!(
        replied(Some(board), opened, "Workspace handoff").is_err(),
        "a title the board already showed can never pass for the reply"
    );
    assert_eq!(
        replied(None, board, "Workspace handoff"),
        Ok(true),
        "no keys typed: presence answers"
    );
    assert_eq!(
        replied(None, board, "remember this"),
        Ok(false),
        "absent text is not a reply"
    );
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

#[test]
fn native_crash_and_fresh_terminal_recovery() -> Journey {
    std::thread::scope(|owner| -> Journey {
        let mut pty = Pty::open(owner)?;
        pty.wait_for(BOOT_TEXT)?;
        pty.send("\r")?;
        pty.wait_for(WORKSPACE_TEXT)?;
        pty.send("/changes\r")?;
        pty.wait_for("CHANGES / fixture")?;
        pty.send("/next\r")?;
        pty.wait_for("src/retry.rs:12")?;
        let pid = pty.pid()?;
        let killed = Command::new("kill").args(["-9", &pid]).status()?;
        assert!(killed.success(), "kill -9 {pid} was delivered");
        let (status, _) = pty.finish()?;
        assert!(
            !status.success(),
            "a killed binary does not report success: {status}"
        );
        let probe = Command::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()?;
        assert!(!probe.success(), "the killed binary {pid} was reaped");
        let mut fresh = Pty::open(owner)?;
        fresh.wait_for(BOOT_TEXT)?;
        fresh.send("q")?;
        let (status, _) = fresh.finish()?;
        assert!(
            status.success(),
            "a fresh terminal boots clean and quits 0: {status}"
        );
        Ok(())
    })
}

#[test]
fn board_truth_mouse_and_draft_recovery() -> Journey {
    let steps: [(&str, &str); 9] = [
        ("t", "SYNTHETIC DATA"),
        ("t", BOOT_TEXT),
        // Real xterm SGR mouse input, 1-based coordinates inside rail row 3.
        ("\x1b[<0;8;17M\x1b[<0;8;17m", WORKSPACE_TEXT),
        ("", "Calmer project home"),
        ("remember this", "remember this"),
        ("\x1b", BOOT_TEXT),
        // The breadcrumb, not the title: the board paints every card's title.
        ("2", "MOO  /  Workspace handoff"),
        ("\x1b", BOOT_TEXT),
        ("3", "remember this"),
    ];
    std::thread::scope(|owner| -> Journey {
        let mut pty = Pty::open(owner)?;
        pty.wait_for(BOOT_TEXT)?;
        for (keys, shows) in steps {
            if !keys.is_empty() {
                pty.send(keys)?;
            }
            pty.wait_for(shows)?;
        }
        pty.send("\x15/quit\r")?;
        let (status, _) = pty.finish()?;
        assert!(
            status.success(),
            "the draft is cleared and /quit exits 0: {status}"
        );
        Ok(())
    })
}

/// CPU seconds and RSS KiB of `pid`, from `ps -o time=,rss=`.
fn sample(pid: &str) -> Journey<(f64, u64)> {
    let output = Command::new("ps")
        .args(["-p", pid, "-o", "time=,rss="])
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    let mut fields = text.split_whitespace();
    let time = fields.next().ok_or("ps printed no time")?;
    let rss: u64 = fields.next().ok_or("ps printed no rss")?.parse()?;
    let mut seconds = 0.0_f64;
    for part in time.split(':') {
        seconds = seconds.mul_add(60.0, part.parse::<f64>()?);
    }
    Ok((seconds, rss))
}

/// The `rank`th percentile of `sorted`, by nearest rank, in milliseconds.
fn nearest_rank(sorted: &[Duration], rank: usize) -> Journey<f64> {
    let position = sorted
        .len()
        .saturating_mul(rank)
        .div_ceil(100)
        .saturating_sub(1);
    let value = sorted.get(position).ok_or("no samples")?;
    Ok(value.as_secs_f64() * 1000.0)
}

#[test]
fn workspace_probe_paints_answers_input_and_stays_quiet_idle() -> Journey {
    std::thread::scope(|owner| -> Journey {
        let started = Instant::now();
        let mut pty = Pty::open(owner)?;
        pty.wait_for(BOOT_TEXT)?;
        let first_paint = started.elapsed().as_secs_f64() * 1000.0;
        pty.drain(Duration::from_millis(100))?;
        let mut latencies = Vec::with_capacity(ROUND_TRIPS);
        for _ in 0..ROUND_TRIPS {
            let sent = Instant::now();
            pty.send("\r")?;
            pty.wait_for(WORKSPACE_TEXT)?;
            latencies.push(sent.elapsed());
            pty.drain(Duration::from_millis(50))?;
            pty.send("\x1b")?;
            pty.wait_for(BOOT_TEXT)?;
            pty.drain(Duration::from_millis(50))?;
        }
        latencies.sort();
        let pid = pty.pid()?;
        let (cpu_before, rss_before) = sample(&pid)?;
        let idle_started = Instant::now();
        let idle_bytes = pty.drain(IDLE)?;
        let idle_seconds = idle_started.elapsed().as_secs_f64();
        let (cpu_after, rss_after) = sample(&pid)?;
        let quit = Instant::now();
        pty.send("q")?;
        let (status, _) = pty.finish()?;
        let quit_ms = quit.elapsed().as_secs_f64() * 1000.0;
        assert!(status.success(), "q exits 0: {status}");
        assert!(
            idle_bytes < OUTPUT_BOUND,
            "idle output stays under 1 MiB: {idle_bytes}"
        );
        let mut figures = String::new();
        writeln!(
            figures,
            "{{\"scope\":\"debug binary, 120x32 PTY, synthetic data, disconnected Moo/model\",\
             \"first_paint_ms\":{first_paint:.1},\"input_samples\":{ROUND_TRIPS},\
             \"input_p50_ms\":{:.1},\"input_p95_ms\":{:.1},\"input_p99_ms\":{:.1},\
             \"idle_seconds\":{idle_seconds:.2},\"idle_cpu_percent\":{:.2},\
             \"idle_output_bytes\":{idle_bytes},\"sampled_max_rss_kib\":{},\
             \"quit_to_exit_ms\":{quit_ms:.1}}}",
            nearest_rank(&latencies, 50)?,
            nearest_rank(&latencies, 95)?,
            nearest_rank(&latencies, 99)?,
            (cpu_after - cpu_before) / idle_seconds * 100.0,
            rss_before.max(rss_after),
        )?;
        std::io::stderr().lock().write_all(figures.as_bytes())?;
        Ok(())
    })
}
