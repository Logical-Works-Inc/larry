//! One acceptance command in its own process group, under a timeout (larry#85).

use std::io::{self, Read};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvError, RecvTimeoutError};
use std::thread::ScopedJoinHandle;
use std::time::{Duration, Instant};

use lgwks_bot::clock::Clock;
use lgwks_std::process::{
    ProcessIdentity, identify_process, kill_process_group, process_group_exists,
};

use super::ExecError;
use super::confine::Confine;
use super::ledger::Ledger;

// A command's own process group is what a timeout or a cancel kills; a target without
// one has no safe way to run it, so the build refuses rather than run it ungrouped.
#[cfg(not(unix))]
compile_error!("the executor runs each command in its own process group: Unix targets only");

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

/// Time a process group is given before it counts as surviving (larry#214, larry#268).
/// Killed members are briefly zombies, so the probes yield between checks, and both group
/// waits end on this one bound read from the clock their caller injected: a count of yields
/// measured the scheduler, so a loaded host ended it early and an idle one late.
pub(super) const GROUP_PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// Longest one slice of the command wait sleeps before it checks the person's
/// cancel again (larry#242): it bounds Esc-to-kill latency for a running
/// command. `tui_journey_esc_kills_running_command` prints the whole path
/// (`esc_to_interrupted_ms`) against this 25 ms slice; the figure is
/// MEASURED, never a pass condition, because a loaded host moves it (larry#268).
const CANCEL_POLL: Duration = Duration::from_millis(25);

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
        let (head, tail) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(head);
        self.tail.extend_from_slice(tail);
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
    /// Whether the person cancelled the command and its group was killed
    /// (larry#242); never true together with `timed_out`.
    interrupted: bool,
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

    /// Whether the person cancelled the command and its group was killed (larry#242).
    pub(crate) const fn interrupted(&self) -> bool {
        self.interrupted
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
            "interrupted": self.interrupted,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "stdout_dropped": self.stdout_dropped,
            "stderr_dropped": self.stderr_dropped,
            "truncated": self.truncated,
            "duration_ms": self.duration_ms,
        })
    }
}

/// Where a command runs: a directory as it is, or a workspace the kernel confines it to
/// (larry#243).
#[derive(Clone, Copy)]
pub(crate) enum Place<'a> {
    /// This directory, open to whatever the command does in it.
    Open(&'a Path),
    /// This confinement's tree, behind its wrapper, with `TMPDIR` at its scratch directory.
    Confined(&'a Confine),
}

/// Runs `argv` in `place` in its own process group, with the environment cleared to
/// PATH, HOME (the workspace), and LANG=C. The leader's exit, a timeout, or the person's
/// cancel (`cancelled`, polled at most [`CANCEL_POLL`] apart, larry#242) kills the whole
/// group; a timeout and an interrupt are reported as `timed_out` and `interrupted`,
/// never both at once. The group waits read one deadline from the live clock this
/// builds (`larry_runtime::live_clock`, larry#268), so no wait is decided by a count of
/// yields.
pub(crate) fn run_command(
    argv: &[String],
    place: Place<'_>,
    path_env: &str,
    timeout: Duration,
    ledger: &Ledger,
    task_id: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<CommandResult, ExecError> {
    let started_at = Instant::now();
    // The one timeline the group waits read, built the way the gate builds its own (the
    // runtime's `live_clock`, larry#268), so a host whose clock reads before the epoch
    // refuses with nothing run.
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let (mut command, program) = command_for(argv, place, path_env)?;
    let program = program.as_str();
    // Computed before the spawn: an overflow found inside the scope below would leave
    // the live leader unkilled, and the scope would wait on it.
    let deadline = started_at.checked_add(timeout).ok_or_else(|| {
        spawn_failure(program, io::Error::other("the command deadline overflowed"))
    })?;
    let mut child = command
        .spawn()
        .map_err(|source| spawn_failure(program, source))?;
    let pgid = match i32::try_from(child.id()) {
        Ok(pid) => pid,
        Err(error) => {
            let refusal = spawn_failure(program, io::Error::other(error));
            return Err(discard(&mut child, refusal));
        }
    };
    let (Some(stdout_pipe), Some(stderr_pipe)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(abandon(
            &clock,
            &mut child,
            pgid,
            spawn_failure(program, io::Error::other("piped output is missing")),
        ));
    };
    let identity = track(&clock, &mut child, pgid, program, (ledger, task_id))?;

    let (status_sender, status_receiver) = mpsc::sync_channel::<io::Result<ExitStatus>>(1);
    let (status, timed_out, interrupted, stdout, stderr) = std::thread::scope(
        |scope| -> Result<(io::Result<ExitStatus>, bool, bool, Captured, Captured), ExecError> {
            let stdout_reader = scope.spawn(move || capture(stdout_pipe));
            let stderr_reader = scope.spawn(move || capture(stderr_pipe));
            // The receiver outlives the wait unless the wait already failed, and that
            // failure is what the scope returns, so a send with no receiver has no reader.
            scope.spawn(move || status_sender.send(child.wait()));
            let (status, timed_out, interrupted) =
                wait_for_status(&clock, &status_receiver, pgid, deadline, cancelled)?;
            // The leader is gone; a background member still holding the pipes must die too,
            // or the readers below would block forever.
            settle_group(&clock, pgid)?;
            let stdout = join_reader(stdout_reader, program)?;
            let stderr = join_reader(stderr_reader, program)?;
            Ok((status, timed_out, interrupted, stdout, stderr))
        },
    )?;
    let status = status.map_err(|source| spawn_failure(program, source))?;
    await_group_gone(&clock, pgid)?;
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
        interrupted,
        stdout: stdout_text,
        stderr: stderr_text,
        stdout_dropped,
        stderr_dropped,
        truncated: stdout_dropped > 0 || stderr_dropped > 0,
        duration_ms,
    })
}

/// The command that runs `argv` in `place`, and the program it starts, in its own process
/// group with the environment cleared to PATH, HOME (the workspace) and LANG=C. A confined
/// command is the platform's wrapper plus the model's argv, so the kernel confines it and
/// everything it forks; the result still names the model's argv.
fn command_for(
    argv: &[String],
    place: Place<'_>,
    path_env: &str,
) -> Result<(Command, String), ExecError> {
    let empty = || spawn_failure("", io::Error::other("acceptance command is empty"));
    if argv.is_empty() {
        return Err(empty());
    }
    let (workspace, scratch, wrapped) = match place {
        Place::Open(path) => (path, None, None),
        Place::Confined(confine) => (
            confine.tree(),
            Some(confine.scratch()),
            Some(confine.wrap(argv)),
        ),
    };
    let launch: &[String] = match wrapped.as_deref() {
        Some(prefixed) => prefixed,
        None => argv,
    };
    let Some((program, arguments)) = launch.split_first() else {
        return Err(empty());
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
        .stderr(Stdio::piped());
    if let Some(directory) = scratch {
        command.env("TMPDIR", directory);
    }
    #[cfg(unix)]
    command.process_group(0);
    Ok((command, program.clone()))
}

/// Identifies the leader of `pgid` and records it in the ledger under `task_id`; a
/// leader that cannot be tracked is killed with its group first.
fn track(
    clock: &Clock,
    child: &mut Child,
    pgid: i32,
    program: &str,
    (ledger, task_id): (&Ledger, &str),
) -> Result<ProcessIdentity, ExecError> {
    let identity = match identify_process(pgid) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            return Err(abandon(
                clock,
                child,
                pgid,
                spawn_failure(
                    program,
                    io::Error::other("the child vanished before it was identified"),
                ),
            ));
        }
        Err(source) => {
            return Err(abandon(clock, child, pgid, spawn_failure(program, source)));
        }
    };
    match ledger.insert(task_id, &identity) {
        Ok(()) => Ok(identity),
        Err(error) => Err(abandon(clock, child, pgid, error)),
    }
}

/// Waits for the command's status in slices of at most [`CANCEL_POLL`] until
/// `deadline`, checking `cancelled` after each slice: the person's Esc ends a
/// running command, not only the next step (larry#242). A cancel kills the
/// group and takes the status exactly as the deadline does, so exactly one of
/// `interrupted` and `timed_out` is set; a waiter that ends without a status is
/// the fault it always was.
fn wait_for_status(
    clock: &Clock,
    receiver: &mpsc::Receiver<io::Result<ExitStatus>>,
    pgid: i32,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<(io::Result<ExitStatus>, bool, bool), ExecError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            settle_group(clock, pgid)?;
            return Ok((flatten(receiver.recv()), true, false));
        }
        match receiver.recv_timeout(CANCEL_POLL.min(left)) {
            Ok(status) => return Ok((status, false, false)),
            Err(RecvTimeoutError::Timeout) => {
                if cancelled() {
                    settle_group(clock, pgid)?;
                    return Ok((flatten(receiver.recv()), false, true));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Ok((
                    Err(io::Error::other("the waiter ended without a status")),
                    false,
                    false,
                ));
            }
        }
    }
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
            Ok(read) => {
                let filled = chunk.get(..read).ok_or_else(|| {
                    io::Error::other("a read reported more bytes than its buffer holds")
                })?;
                captured.push(filled)?;
            }
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
        Err(panic) => Err(spawn_failure(
            argv0,
            io::Error::other(format!("output reader panicked: {panic:?}")),
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

/// Kills the group and the leader of a child that could not be tracked, then returns the
/// error, or the reap failure that names it when the group or the leader could not be ended.
fn abandon(clock: &Clock, child: &mut Child, pgid: i32, error: ExecError) -> ExecError {
    let settled = settle_group(clock, pgid);
    let waited = child.wait();
    if let Err(cause) = settled {
        return ExecError::Reap(io::Error::other(format!(
            "{error}; the group could not be settled: {cause}"
        )));
    }
    if let Err(cause) = waited {
        return ExecError::Reap(io::Error::other(format!(
            "{error}; the leader could not be reaped: {cause}"
        )));
    }
    error
}

/// Kills and reaps a child whose group id is unknown, then returns the error, or the reap
/// failure that names it when the child could not be ended.
fn discard(child: &mut Child, error: ExecError) -> ExecError {
    if let Err(cause) = child.kill() {
        return ExecError::Reap(io::Error::other(format!(
            "{error}; the child could not be killed: {cause}"
        )));
    }
    if let Err(cause) = child.wait() {
        return ExecError::Reap(io::Error::other(format!(
            "{error}; the child could not be reaped: {cause}"
        )));
    }
    error
}

/// Whether `clock` has reached [`GROUP_PROBE_DEADLINE`] past the wait's one start reading.
/// This is the only bound either group wait reads (larry#268): one reading of the injected
/// clock, never a count of yields.
fn past_deadline(clock: &Clock, started: Duration) -> bool {
    clock.now().saturating_sub(started) >= GROUP_PROBE_DEADLINE
}

/// SIGKILLs a process group; a group that is already gone is settled.
///
/// macOS refuses a signal to a group whose remaining members are zombies with
/// `EPERM` (openvax/mhctools#620), and BSD `kill(2)` sends nothing to any member
/// when one target fails the check, so one refusal can leave live members
/// unsignalled. The kill is therefore repeated while `EPERM` persists, until
/// [`GROUP_PROBE_DEADLINE`] has passed on `clock`, so it lands once the zombie is
/// reaped; refusing at the first `EPERM` failed 7 of 24 parallel runs of the timeout
/// journey (MEASURED, larry#131). A refusal that outlasts the bound is handed to
/// [`await_group_gone`], which never presumes the group gone and ends a member
/// that really refuses in [`ExecError::GroupSurvived`].
fn settle_group(clock: &Clock, pgid: i32) -> Result<(), ExecError> {
    settle_with(clock, || kill_process_group(pgid))
}

/// [`settle_group`] over a signal the caller supplies, so the refusal schedule can be
/// driven: an `EPERM` is repeated until [`GROUP_PROBE_DEADLINE`] has passed on `clock`, a
/// group the kernel says is gone settles at once, and any other refusal is typed.
fn settle_with(clock: &Clock, mut signal: impl FnMut() -> io::Result<()>) -> Result<(), ExecError> {
    let started = clock.now();
    loop {
        match signal() {
            Ok(()) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(ESRCH) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(EPERM) => {
                if past_deadline(clock, started) {
                    return Ok(());
                }
                std::thread::yield_now();
            }
            Err(error) => return Err(ExecError::Reap(error)),
        }
    }
}

/// Probes the group until it is gone, yielding between probes; fails once
/// [`GROUP_PROBE_DEADLINE`] has passed on `clock` with the group still there.
pub(super) fn await_group_gone(clock: &Clock, pgid: i32) -> Result<(), ExecError> {
    if await_gone(clock, || {
        process_group_exists(pgid).map_err(ExecError::Reap)
    })? {
        return Ok(());
    }
    Err(ExecError::GroupSurvived { pgid })
}

/// Waits for `still_there` to answer that the group is gone, yielding between probes, and
/// answers whether it did: `false` once [`GROUP_PROBE_DEADLINE`] has passed on `clock` with
/// the group still there. `still_there` is the probe's own answer, so the caller passes
/// `process_group_exists` and a `true` keeps waiting (larry#268); both decisions are the
/// probe's and one clock reading, so the seeded sim can drive them to the deadline and
/// before it on a virtual clock. The error type is the probe's, so a sim can report a
/// refused clock advance as itself. Visible to the executor so the ledger's reap and its
/// simulation share this one wait instead of a second copy (larry#308).
pub(super) fn await_gone<E>(
    clock: &Clock,
    mut still_there: impl FnMut() -> Result<bool, E>,
) -> Result<bool, E> {
    let started = clock.now();
    while still_there()? {
        if past_deadline(clock, started) {
            return Ok(false);
        }
        std::thread::yield_now();
    }
    Ok(true)
}

/// Seeded simulations of the bounded capture (larry#206).
#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;

/// Seeded simulations of the group wait's one deadline, on a virtual clock (larry#268).
#[cfg(test)]
#[path = "sim_group.rs"]
mod sim_group;
