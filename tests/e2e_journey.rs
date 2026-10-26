//! The shipped `rocco` binary driven through one user journey on a real PTY.
//!
//! `script(1)` supplies the terminal. The test waits for the boot screen,
//! continues into the workspace, opens the folders, agents and changes
//! panels, steps the fixture, holds and continues work, runs
//! fixture-continue (which executes no backend action in this disconnected
//! demo), switches scenario and quits. It then asserts the binary exited 0
//! and that every caller-visible marker appears in journey order in the
//! transcript, with escape sequences stripped.

use std::io::{Read, Write};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

/// Failures propagate instead of panicking.
type Journey<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Upper bound on each wait; it only bounds a failure, never orders events.
const PATIENCE: Duration = Duration::from_secs(30);

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

/// The boot screen's marker; the journey types nothing before it.
const BOOT: &str = "everythingelsestaysquiet";
/// The workspace's marker; the steps are typed once it shows.
const WORKSPACE: &str = "awaitingmoo";

/// Markers over the normalized transcript, in journey order. Two pin
/// rendering facts, not typos: the CHANGES header emits no N byte on this
/// path (stale-cell retention paints it), so the stream reads
/// "chagesfixture"; the fixture-03 step selects src/retry.rs:18, so its
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

/// `script(1)` running the shipped binary in a 32x120 terminal.
fn terminal() -> Command {
    let inner = format!(
        "stty rows 32 cols 120; exec '{}'",
        env!("CARGO_BIN_EXE_rocco")
    );
    let mut command = Command::new("script");
    if cfg!(target_os = "linux") {
        command.args(["-qec", &inner, "/dev/null"]);
    } else {
        command.args(["-q", "/dev/null", "sh", "-c", &inner]);
    }
    command.env("TERM", "xterm-256color");
    command
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

/// Index just past the escape sequence whose introducer byte is at `start`:
/// OSC to BEL, CSI parameters to a final letter, a charset designation, or
/// one byte. A truncated sequence ends where the input does.
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

/// Collects output until the normalized transcript contains `marker`, or,
/// for `None`, until the terminal closes its output.
fn wait(chunks: &Receiver<Vec<u8>>, seen: &mut Vec<u8>, marker: Option<&str>) -> Journey {
    let deadline = Instant::now()
        .checked_add(PATIENCE)
        .ok_or("deadline overflows")?;
    loop {
        if marker.is_some_and(|text| normalize(seen).contains(text)) {
            return Ok(());
        }
        match chunks.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(chunk) => seen.extend_from_slice(&chunk),
            Err(RecvTimeoutError::Disconnected) => {
                return match marker {
                    None => Ok(()),
                    Some(text) => Err(format!("terminal closed before {text}").into()),
                };
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("timed out waiting for {marker:?}").into());
            }
        }
    }
}

/// Forwards the terminal's output in chunks until it closes or the receiver
/// is gone.
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

/// The whole journey through the shipped binary on a real PTY.
#[test]
fn e2e_journey_through_the_shipped_binary() -> Journey {
    let mut child = terminal()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("terminal has no stdin")?;
    let stdout = child.stdout.take().ok_or("terminal has no stdout")?;
    let (sender, chunks) = mpsc::sync_channel(64);
    let seen = std::thread::scope(|owner| -> Journey<Vec<u8>> {
        let reader = owner.spawn(move || forward(stdout, &sender));
        let mut seen = Vec::new();
        wait(&chunks, &mut seen, Some(BOOT))?;
        stdin.write_all(b"\r")?;
        wait(&chunks, &mut seen, Some(WORKSPACE))?;
        for step in STEPS {
            stdin.write_all(step.as_bytes())?;
            stdin.write_all(b"\r")?;
        }
        stdin.flush()?;
        wait(&chunks, &mut seen, None)?;
        reader
            .join()
            .map_err(|_| "terminal reader thread panicked")??;
        Ok(seen)
    })?;
    drop(stdin);
    let status = child.wait()?;
    assert!(
        status.success(),
        "the binary exits 0 after /quit, got {status}"
    );
    let clean = normalize(&seen);
    let mut from = 0_usize;
    for marker in MARKERS {
        let found = clean
            .get(from..)
            .and_then(|rest| rest.find(marker))
            .ok_or_else(|| format!("marker missing in journey order: {marker}"))?;
        from = from.saturating_add(found).saturating_add(marker.len());
    }
    Ok(())
}

/// The stripper removes OSC, CSI, charset and single-byte escapes, keeps the
/// text between them, and survives a truncated sequence at the end.
#[test]
fn e2e_journey_normalize_strips_escapes() {
    assert_eq!(
        normalize(b"\x1b]0;title\x07Ab\x1b[1;31mC\x1b(Bd\x1b=e-f\x1b["),
        "abcdef",
        "escapes are removed and the text between them kept"
    );
    assert_eq!(normalize(b"\x1b"), "", "a lone escape yields nothing");
}
