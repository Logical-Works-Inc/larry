//! One acceptance command in its own process group, under a timeout (larry#85).

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvError, RecvTimeoutError};
use std::thread::ScopedJoinHandle;
use std::time::{Duration, Instant};

use lgwks_std::process::{identify_process, kill_process_group, process_group_exists};

use super::ExecError;
use super::ledger::Ledger;

/// Bytes of each output stream kept from its start in the report (larry#206).
const HEAD_BYTES: usize = 524_288;

/// Bytes of each output stream kept from its end in the report (larry#206): the end of a
/// failing log holds its error.
const TAIL_BYTES: usize = 524_288;

/// Bytes read from a pipe per piece.
const CAPTURE_CHUNK: usize = 8_192;

/// The kernel's error number for "no such process".
const ESRCH: i32 = 3;

/// The kernel's error number for "operation not permitted", the same on Linux and Darwin.
const EPERM: i32 = 1;

/// Probes of a process group before it counts as surviving: a second cap behind
/// [`GROUP_PROBE_DEADLINE`], so a fast loop cannot spin past the deadline's intent.
const GROUP_PROBES: u32 = 10_000;

/// Time a process group is probed before it counts as surviving (larry#214). Killed
/// members are briefly zombies, so the probes yield between checks; the bound is the
/// elapsed time, which a count of yields never measured.
const GROUP_PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// One captured stream: its first bytes, its last bytes, and how many bytes fell between them.
#[derive(Default)]
struct Captured {
    /// The first `HEAD_BYTES` of the stream.
    head: Vec<u8>,
    /// The last `TAIL_BYTES` of the stream, after the head.
    tail: Vec<u8>,
    /// The bytes between the head and the tail: counted, never kept.
    dropped: u64,
}

impl Captured {
    /// Adds the next bytes of the stream. The head fills first; the tail keeps the most recent
    /// `TAIL_BYTES`, and what it pushes out is counted as dropped.
    fn push(&mut self, bytes: &[u8]) -> io::Result<()> {
        let room = HEAD_BYTES.saturating_sub(self.head.len());
        let kept = room.min(bytes.len());
        self.head.extend_from_slice(&bytes[..kept]);
        self.tail.extend_from_slice(&bytes[kept..]);
        if self.tail.len() > TAIL_BYTES.saturating_mul(2) {
            self.trim()?;
        }
        Ok(())
    }

    /// Drops the tail's oldest bytes until it holds `TAIL_BYTES`, counting every one.
    fn trim(&mut self) -> io::Result<()> {
        let cut = self.tail.len().saturating_sub(TAIL_BYTES);
        self.tail.drain(..cut);
        let counted = u64::try_from(cut).map_err(io::Error::other)?;
        self.dropped = self
            .dropped
            .checked_add(counted)
            .ok_or_else(|| io::Error::other("the dropped byte count overflowed"))?;
        Ok(())
    }

    /// The kept bytes as text: the head, a marker line when bytes were dropped, then the tail.
    /// Returns the text and the dropped count.
    fn into_text(self) -> (String, u64) {
        let mut bytes = self.head;
        if self.dropped > 0 {
            let marker = format!("\n[larry: {} bytes dropped]\n", self.dropped);
            bytes.extend_from_slice(marker.as_bytes());
        }
        bytes.extend_from_slice(&self.tail);
        (String::from_utf8_lossy(&bytes).into_owned(), self.dropped)
    }
}

/// What one acceptance command did, as the report records it.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct CommandResult {
    /// The argv that ran.
    argv: Vec<String>,
    /// The process group the command led.
    pgid: i32,
    /// The exit code, or `None` when a signal ended the command.
    exit_code: Option<i32>,
    /// Whether the timeout fired and the group was killed.
    timed_out: bool,
    /// Standard output: its head and its tail, joined by a marker line, lossily decoded.
    stdout: String,
    /// Standard error: its head and its tail, joined by a marker line, lossily decoded.
    stderr: String,
    /// Bytes of standard output dropped between its head and its tail.
    stdout_dropped: u64,
    /// Bytes of standard error dropped between its head and its tail.
    stderr_dropped: u64,
    /// Whether either stream dropped bytes.
    truncated: bool,
    /// Wall-clock duration in milliseconds; measured, never used to control the run.
    duration_ms: u64,
}

impl CommandResult {
    /// The exit code, or `None` when a signal ended the command.
    pub(crate) const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// Whether the timeout fired and the group was killed.
    pub(crate) const fn timed_out(&self) -> bool {
        self.timed_out
    }

    /// The kept standard output.
    pub(crate) fn stdout(&self) -> &str {
        self.stdout.as_str()
    }

    /// The kept standard error.
    pub(crate) fn stderr(&self) -> &str {
        self.stderr.as_str()
    }

    /// Whether any output was not kept.
    pub(crate) const fn truncated(&self) -> bool {
        self.truncated
    }

    /// This result with each stream cut to its first `bytes` on a character boundary,
    /// `truncated` set when anything was cut: what a terminal session shows its model
    /// and keeps in its report (larry#93).
    pub(crate) fn clipped(mut self, bytes: usize) -> CommandResult {
        let out_cut = clip(&mut self.stdout, bytes);
        let err_cut = clip(&mut self.stderr, bytes);
        self.truncated = self.truncated || out_cut || err_cut;
        self
    }

    /// The report object for this command.
    #[must_use]
    pub(crate) fn into_json(self) -> lgwks_std::json::Value {
        lgwks_std::json::json!({
            "argv": self.argv,
            "pgid": self.pgid,
            "exit_code": self.exit_code,
            "timed_out": self.timed_out,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "stdout_dropped": self.stdout_dropped,
            "stderr_dropped": self.stderr_dropped,
            "truncated": self.truncated,
            "duration_ms": self.duration_ms,
        })
    }
}

/// Runs `argv` in `workspace` in its own process group, with the environment cleared to
/// PATH, HOME (the workspace), and LANG=C. A timeout or a leader's exit kills the whole group.
pub(crate) fn run_command(
    argv: &[String],
    workspace: &Path,
    path_env: &str,
    timeout: Duration,
    ledger: &Ledger,
    task_id: &str,
) -> Result<CommandResult, ExecError> {
    let started_at = Instant::now();
    let Some((program, arguments)) = argv.split_first() else {
        return Err(spawn_failure(
            "",
            io::Error::other("acceptance command is empty"),
        ));
    };
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(workspace)
        .env_clear()
        .env("PATH", path_env)
        .env("HOME", workspace)
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|source| spawn_failure(program, source))?;
    let pgid = match i32::try_from(child.id()) {
        Ok(pid) => pid,
        Err(error) => {
            drop(child.kill());
            drop(child.wait());
            return Err(spawn_failure(program, io::Error::other(error)));
        }
    };
    let (Some(stdout_pipe), Some(stderr_pipe)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(abandon(
            &mut child,
            pgid,
            spawn_failure(program, io::Error::other("piped output is missing")),
        ));
    };
    let identity = match identify_process(pgid) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            return Err(abandon(
                &mut child,
                pgid,
                spawn_failure(
                    program,
                    io::Error::other("the child vanished before it was identified"),
                ),
            ));
        }
        Err(source) => return Err(abandon(&mut child, pgid, spawn_failure(program, source))),
    };
    if let Err(error) = ledger.insert(task_id, &identity) {
        return Err(abandon(&mut child, pgid, error));
    }

    let (status_sender, status_receiver) = mpsc::sync_channel::<io::Result<ExitStatus>>(1);
    let (status, timed_out, stdout, stderr) = std::thread::scope(
        |scope| -> Result<(io::Result<ExitStatus>, bool, Captured, Captured), ExecError> {
            let stdout_reader = scope.spawn(move || capture(stdout_pipe));
            let stderr_reader = scope.spawn(move || capture(stderr_pipe));
            scope.spawn(move || drop(status_sender.send(child.wait())));
            let (status, timed_out) = match status_receiver.recv_timeout(timeout) {
                Ok(status) => (status, false),
                Err(RecvTimeoutError::Timeout) => {
                    settle_group(pgid)?;
                    (flatten(status_receiver.recv()), true)
                }
                Err(RecvTimeoutError::Disconnected) => (
                    Err(io::Error::other("the waiter ended without a status")),
                    false,
                ),
            };
            // The leader is gone; a background member still holding the pipes must die too,
            // or the readers below would block forever.
            settle_group(pgid)?;
            let stdout = join_reader(stdout_reader, program)?;
            let stderr = join_reader(stderr_reader, program)?;
            Ok((status, timed_out, stdout, stderr))
        },
    )?;
    let status = status.map_err(|source| spawn_failure(program, source))?;
    await_group_gone(pgid)?;
    ledger.remove(&identity)?;
    let (stdout_text, stdout_dropped) = stdout.into_text();
    let (stderr_text, stderr_dropped) = stderr.into_text();
    let elapsed = started_at.elapsed();
    let duration_ms = elapsed
        .as_secs()
        .saturating_mul(1_000)
        .saturating_add(u64::from(elapsed.subsec_millis()));
    Ok(CommandResult {
        argv: argv.to_vec(),
        pgid,
        exit_code: status.code(),
        timed_out,
        stdout: stdout_text,
        stderr: stderr_text,
        stdout_dropped,
        stderr_dropped,
        truncated: stdout_dropped > 0 || stderr_dropped > 0,
        duration_ms,
    })
}

/// Cuts `text` to at most `bytes` on a character boundary and reports whether it cut.
pub(crate) fn clip(text: &mut String, bytes: usize) -> bool {
    if text.len() <= bytes {
        return false;
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.truncate(end);
    true
}

/// Keeps the first `HEAD_BYTES` and the last `TAIL_BYTES` of a pipe, and counts the bytes
/// between them. The pipe is read to its end in pieces, so memory is bounded by the two bounds.
fn capture(mut pipe: impl Read) -> io::Result<Captured> {
    let mut captured = Captured::default();
    let mut chunk = [0_u8; CAPTURE_CHUNK];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => captured.push(&chunk[..read])?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    if captured.tail.len() > TAIL_BYTES {
        captured.trim()?;
    }
    Ok(captured)
}

/// Waits for one reader thread and turns its failure into a typed error.
fn join_reader(
    reader: ScopedJoinHandle<'_, io::Result<Captured>>,
    argv0: &str,
) -> Result<Captured, ExecError> {
    match reader.join() {
        Ok(Ok(captured)) => Ok(captured),
        Ok(Err(source)) => Err(spawn_failure(argv0, source)),
        Err(_) => Err(spawn_failure(
            argv0,
            io::Error::other("output reader panicked"),
        )),
    }
}

/// Flattens a received wait status, mapping a closed channel to an I/O error.
fn flatten(received: Result<io::Result<ExitStatus>, RecvError>) -> io::Result<ExitStatus> {
    match received {
        Ok(status) => status,
        Err(error) => Err(io::Error::other(error)),
    }
}

/// Builds a spawn failure that names the program.
fn spawn_failure(argv0: &str, source: io::Error) -> ExecError {
    ExecError::Spawn {
        argv0: argv0.to_owned(),
        source,
    }
}

/// Kills the group and the leader of a child that could not be tracked, then returns the error.
fn abandon(child: &mut Child, pgid: i32, error: ExecError) -> ExecError {
    drop(settle_group(pgid));
    drop(child.wait());
    error
}

/// SIGKILLs a process group; a group that is already gone is settled.
///
/// macOS refuses a signal to a group whose remaining members are zombies with
/// `EPERM` (openvax/mhctools#620), and BSD `kill(2)` sends nothing to any member
/// when one target fails the check, so one refusal can leave live members
/// unsignalled. The kill is therefore repeated while `EPERM` persists, until
/// [`GROUP_PROBE_DEADLINE`] has passed or [`GROUP_PROBES`] yields, so it lands once
/// the zombie is reaped; refusing at the
/// first `EPERM` failed 7 of 24 parallel runs of the timeout journey (MEASURED,
/// larry#131). A refusal that outlasts the probes is handed to
/// [`await_group_gone`], which never presumes the group gone and ends a member
/// that really refuses in [`ExecError::GroupSurvived`].
fn settle_group(pgid: i32) -> Result<(), ExecError> {
    let started = Instant::now();
    let mut refusals = 0_u32;
    loop {
        match kill_process_group(pgid) {
            Ok(()) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(ESRCH) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(EPERM) => {
                if refusals >= GROUP_PROBES || started.elapsed() >= GROUP_PROBE_DEADLINE {
                    return Ok(());
                }
                refusals = refusals.saturating_add(1);
                std::thread::yield_now();
            }
            Err(error) => return Err(ExecError::Reap(error)),
        }
    }
}

/// Probes the group until it is gone, yielding between probes; fails once
/// [`GROUP_PROBE_DEADLINE`] or [`GROUP_PROBES`] runs out first.
pub(super) fn await_group_gone(pgid: i32) -> Result<(), ExecError> {
    let started = Instant::now();
    let mut probes = 0_u32;
    while process_group_exists(pgid).map_err(ExecError::Reap)? {
        if probes >= GROUP_PROBES || started.elapsed() >= GROUP_PROBE_DEADLINE {
            return Err(ExecError::GroupSurvived { pgid });
        }
        probes = probes.saturating_add(1);
        std::thread::yield_now();
    }
    Ok(())
}

/// Seeded simulations of the bounded capture (larry#206).
#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
