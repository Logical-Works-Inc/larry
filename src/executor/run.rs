//! One acceptance command in its own process group and, where the host has `setsid`, its own
//! session, under a timeout (larry#85, larry#323).

use std::io::{self, Read};
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::ScopedJoinHandle;
use std::time::{Duration, Instant};

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::process::{ProcessIdentity, identify_process};

use super::ExecError;
use super::confine::{Confine, SETSID};
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

/// Maximum bytes one live-output callback receives.
pub(crate) const STREAM_CHUNK: usize = 4_096;

/// A nonblocking observer for live command output. The callback runs on the bounded drain
/// readers, so it must never wait on the command or another reader.
pub(crate) type StreamCallback = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Hooks a command exposes while it runs: cancellation and optional live output.
#[derive(Clone, Copy)]
pub(crate) struct RunHooks<'a> {
    /// Whether the owner has cancelled the command.
    cancelled: &'a dyn Fn() -> bool,
    /// The nonblocking live-output callback, when a frontend owns one.
    stream: Option<&'a StreamCallback>,
}

impl<'a> RunHooks<'a> {
    /// Builds the hooks for one command.
    pub(crate) const fn new(
        cancelled: &'a dyn Fn() -> bool,
        stream: Option<&'a StreamCallback>,
    ) -> Self {
        Self { cancelled, stream }
    }
}

#[cfg(test)]
pub(super) use group::GROUP_PROBE_DEADLINE;
pub(super) use group::await_gone;
use group::{abandon, await_group_gone, discard, settle_group, status_after_kill};

/// Longest one slice of the command wait sleeps before it checks the person's cancel again
/// (larry#242): it bounds Esc-to-kill latency. `tui_journey_esc_kills_running_command` prints
/// the whole path (`esc_to_interrupted_ms`) against this 25 ms slice; the figure is MEASURED,
/// never a pass condition, because a loaded host moves it (larry#268).
const CANCEL_POLL: Duration = Duration::from_millis(25);

/// How long a reader may keep draining once the group is settled: past it, a stream still
/// receiving bytes, or silent, belongs to a child that left the group, and the run ends typed
/// rather than joining with no deadline (larry#334). Silence is read for the grace too: a
/// sibling's fork holds the writer until its `exec` (larry#454).
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// One captured stream: its first bytes, its last bytes, and how many bytes fell between them.
#[derive(Default)]
struct Captured {
    /// The first `HEAD_BYTES` of the stream.
    head: Vec<u8>,
    /// The last `TAIL_BYTES` of the stream, after the head.
    tail: Vec<u8>,
    /// The bytes between the head and the tail: counted, never kept.
    dropped: u64,
    /// Every byte the stream yielded: the head, the dropped bytes and the tail together. The
    /// report states it, so a person can check that kept plus dropped is what the command wrote
    /// and no byte went unaccounted (larry#319).
    bytes: u64,
}

impl Captured {
    /// Adds the next bytes of the stream. The head fills first; the tail keeps the most recent
    /// `TAIL_BYTES`, and what it pushes out is counted as dropped. Every byte added is counted in
    /// `bytes`, so kept plus dropped is the stream's own length (larry#319).
    fn push(&mut self, bytes: &[u8]) -> io::Result<()> {
        let added = u64::try_from(bytes.len()).map_err(io::Error::other)?;
        self.bytes = self
            .bytes
            .checked_add(added)
            .ok_or_else(|| io::Error::other("the stream byte count overflowed"))?;
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
    /// Every byte of standard output the command wrote, kept or dropped (larry#319).
    stdout_bytes: u64,
    /// Every byte of standard error the command wrote, kept or dropped (larry#319).
    stderr_bytes: u64,
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
            "stdout_bytes": self.stdout_bytes,
            "stderr_bytes": self.stderr_bytes,
            "truncated": self.truncated,
            "duration_ms": self.duration_ms,
        })
    }
}

/// Where a command runs: a directory as it is, a read-only probe of one with its
/// home beside it, or a workspace the kernel confines it to (larry#243).
#[derive(Clone, Copy)]
pub(crate) enum Place<'a> {
    /// This directory, open to whatever the command does in it.
    Open(&'a Path),
    /// A read-only probe of `cwd`: the command runs there, but its home is
    /// `home`, outside the probed tree, so a tool that initializes a per-user
    /// home (a toolchain, a build cache) never writes into the tree and its
    /// diff (larry#274). The toolchain homes stay resolved from the host's
    /// real home (read only) when it names one, so a missing toolchain is a
    /// fast typed failure, never a download into the tree.
    Probe {
        /// The tree the probe reads.
        cwd: &'a Path,
        /// The probe's own home, outside the tree.
        home: &'a Path,
        /// Where the probe child's `RUSTUP_HOME` points, when the host names one.
        rustup_home: Option<&'a Path>,
        /// Where the probe child's `CARGO_HOME` points, when the host names one.
        cargo_home: Option<&'a Path>,
    },
    /// This confinement's tree, behind its wrapper, with `TMPDIR` at its scratch directory.
    Confined(&'a Confine),
}

/// Runs `argv` in `place` in a process group of its own, and a session of its own where the host
/// has `setsid`, with the environment cleared to PATH, HOME and LANG=C: HOME is the workspace
/// for an open or confined command and the probe's own home beside the tree for a probe, which
/// also sets `RUSTUP_AUTO_INSTALL=0` and, when the host names them, `RUSTUP_HOME` and
/// `CARGO_HOME` resolved from the host's real home (read only), so a missing toolchain is a
/// fast typed failure, never a download (larry#274). The
/// leader's exit, a timeout, or the person's cancel (`cancelled`, polled at most
/// [`CANCEL_POLL`] apart, larry#242) kills the whole group; a timeout and an interrupt are reported as `timed_out` and `interrupted`,
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
    hooks: RunHooks<'_>,
) -> Result<CommandResult, ExecError> {
    let started_at = Instant::now();
    // The one timeline the group waits read, built the way the gate builds its own (the
    // runtime's `live_clock`, larry#268), so a host whose clock reads before the epoch
    // refuses with nothing run.
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let (mut command, program) = command_for(argv, place, path_env)?;
    let program = program.as_str();
    // Each stream is a socket pair, not a pipe: the parent's end has a read timeout, so the
    // drain can end on the command's status instead of blocking on a writer that left the
    // group (larry#334).
    let (stdout_pipe, stdout_child) = capture_pair(program)?;
    let (stderr_pipe, stderr_child) = capture_pair(program)?;
    command
        .stdout(Stdio::from(OwnedFd::from(stdout_child)))
        .stderr(Stdio::from(OwnedFd::from(stderr_child)));
    // Computed before the spawn: an overflow found inside the scope below would leave
    // the live leader unkilled, and the scope would wait on it.
    let deadline = clock.now().checked_add(timeout).ok_or_else(|| {
        spawn_failure(program, io::Error::other("the command deadline overflowed"))
    })?;
    let spawned = command.spawn();
    // The parent's copies of the child's stream ends would keep every stream open for the
    // whole run, so the drain could never see their end (larry#334).
    drop(command);
    let mut child = spawned.map_err(|source| spawn_failure(program, source))?;
    let pgid = match i32::try_from(child.id()) {
        Ok(pid) => pid,
        Err(error) => {
            let refusal = spawn_failure(program, io::Error::other(error));
            return Err(discard(&mut child, refusal));
        }
    };
    let identity = track(&clock, &mut child, pgid, program, (ledger, task_id))?;

    let (status_sender, status_receiver) = mpsc::sync_channel::<io::Result<ExitStatus>>(1);
    let settled = Arc::new(AtomicBool::new(false));
    let (status, timed_out, interrupted, stdout, stderr) = std::thread::scope(
        |scope| -> Result<(io::Result<ExitStatus>, bool, bool, Drained, Drained), ExecError> {
            let out_settled = Arc::clone(&settled);
            let out_stream = hooks.stream.cloned();
            let stdout_reader = scope
                .spawn(move || capture_bounded(stdout_pipe, &out_settled, out_stream.as_ref()));
            let err_settled = Arc::clone(&settled);
            let err_stream = hooks.stream.cloned();
            let stderr_reader = scope
                .spawn(move || capture_bounded(stderr_pipe, &err_settled, err_stream.as_ref()));
            // The receiver outlives the wait unless the wait already failed, and that
            // failure is what the scope returns, so a send with no receiver has no reader.
            scope.spawn(move || status_sender.send(child.wait()));
            let (status, timed_out, interrupted) =
                match wait_for_status(&clock, &status_receiver, pgid, deadline, hooks.cancelled) {
                    Ok(waited) => waited,
                    Err(cause) => {
                        // The readers stop at the grace past this flag, so the scope ends with
                        // the failure instead of waiting on a stream nobody closed.
                        settled.store(true, Ordering::Relaxed);
                        return Err(cause);
                    }
                };
            // The leader is gone; a background member still in the group must die too, and the
            // group is confirmed gone before the readers are told to stop: a stream still open
            // past that belongs to a child that left the group (larry#334).
            if let Err(cause) =
                settle_group(&clock, pgid).and_then(|()| await_group_gone(&clock, pgid))
            {
                settled.store(true, Ordering::Relaxed);
                return Err(cause);
            }
            settled.store(true, Ordering::Relaxed);
            let stdout = join_reader(stdout_reader, program)?;
            let stderr = join_reader(stderr_reader, program)?;
            Ok((status, timed_out, interrupted, stdout, stderr))
        },
    )?;
    let status = status.map_err(|source| spawn_failure(program, source))?;
    ledger.remove(&identity)?;
    if stdout.open || stderr.open {
        // A child that left the group and kept a stream open is reported, and the run ends:
        // the report would otherwise claim whole streams it could not read (larry#334).
        return Err(output_held(program));
    }
    let (stdout_bytes, stderr_bytes) = (stdout.captured.bytes, stderr.captured.bytes);
    let (stdout_text, stdout_dropped) = stdout.captured.into_text();
    let (stderr_text, stderr_dropped) = stderr.captured.into_text();
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
        stdout_bytes,
        stderr_bytes,
        truncated: stdout_dropped > 0 || stderr_dropped > 0,
        duration_ms,
    })
}

/// The command that runs `argv` in `place`, and the program it starts, in a process group of its
/// own (a session too where the host has `setsid`) with the environment cleared to PATH, HOME
/// and LANG=C: HOME is the workspace for an open or confined command, and the probe's own
/// home beside the tree for a probe (with the toolchain homes and the no-install switch),
/// so the probe never writes into what it reads.
/// A confined command is the platform's wrapper plus the model's argv,
/// so the kernel confines it and everything it forks; the result still names the model's argv.
pub(super) fn command_for(
    argv: &[String],
    place: Place<'_>,
    path_env: &str,
) -> Result<(Command, String), ExecError> {
    let empty = || spawn_failure("", io::Error::other("acceptance command is empty"));
    if argv.is_empty() {
        return Err(empty());
    }
    let (workspace, home, scratch, wrapped) = match place {
        Place::Open(path) => (path, path, None, None),
        Place::Probe { cwd, home, .. } => (cwd, home, None, None),
        Place::Confined(confine) => (
            confine.tree(),
            confine.tree(),
            Some(confine.scratch()),
            Some(confine.wrap(argv)),
        ),
    };
    // util-linux `setsid` gives the command a session of its own, with no controlling terminal.
    // A confined command takes it from its wrapper, which on Linux always starts with it
    // (larry#386); macOS has none and its profile refuses `/dev/tty` (larry#323). Exactly one:
    // a second would find a session leader, fork, and leave the executor waiting on its parent.
    let launch: Vec<String> = match wrapped {
        Some(prefixed) => prefixed,
        None if Path::new(SETSID).is_file() => [&[String::from(SETSID)], argv].concat(),
        None => argv.to_vec(),
    };
    let session = launch.first().is_some_and(|word| word == SETSID);
    let Some((program, arguments)) = launch.split_first() else {
        return Err(empty());
    };
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(workspace)
        .env_clear()
        .env("PATH", path_env)
        .env("HOME", home)
        .env("LANG", "C")
        .stdin(Stdio::null());
    if let Place::Probe {
        rustup_home,
        cargo_home,
        ..
    } = place
    {
        if let Some(rustup) = rustup_home {
            command.env("RUSTUP_HOME", rustup);
        }
        if let Some(cargo) = cargo_home {
            command.env("CARGO_HOME", cargo);
        }
        command.env("RUSTUP_AUTO_INSTALL", "0");
    }
    if let Some(directory) = scratch {
        command.env("TMPDIR", directory);
    }
    if !session {
        #[cfg(unix)]
        command.process_group(0);
    }
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
/// `deadline`, a reading of `clock` (larry#436): the timeout is decided on the injected
/// timeline, never the wall, so a seeded run on a virtual clock drives it. It checks `cancelled`
/// after each slice: the person's Esc ends a running command, not only the next step
/// (larry#242). A cancel kills the group and takes the status exactly as the deadline does, so
/// exactly one of `interrupted` and `timed_out` is set; a waiter that ends without a status is
/// the fault it always was.
fn wait_for_status(
    clock: &Clock,
    receiver: &mpsc::Receiver<io::Result<ExitStatus>>,
    pgid: i32,
    deadline: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<(io::Result<ExitStatus>, bool, bool), ExecError> {
    loop {
        let left = deadline.saturating_sub(clock.now());
        if left.is_zero() {
            settle_group(clock, pgid)?;
            let status = killed_status(clock, receiver, pgid)?;
            return Ok((status, true, false));
        }
        match receiver.recv_timeout(CANCEL_POLL.min(left)) {
            Ok(status) => return Ok((status, false, false)),
            Err(RecvTimeoutError::Timeout) => {
                if cancelled() {
                    settle_group(clock, pgid)?;
                    let status = killed_status(clock, receiver, pgid)?;
                    return Ok((status, false, true));
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

/// Keeps the first `HEAD_BYTES` and the last `TAIL_BYTES` of a stream, and counts the bytes
/// between them, reading to the end in pieces so memory is bounded. The shipped drain is
/// [`capture_bounded`]; this arm is the seeded sim's reference and takes any `Read`.
#[cfg(test)]
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

/// What a bounded drain read: the captured stream and whether a child still held it open when
/// the drain stopped.
struct Drained {
    /// The head, the tail and the counts of the stream read so far.
    captured: Captured,
    /// Whether the write end was still open when the drain stopped, so the stream is not whole.
    open: bool,
}

/// One output stream as a socket pair: the child writes to the second end and the executor
/// reads the first with a read timeout, so no reader blocks past the command's status
/// (larry#334). A pipe cannot carry a read timeout through the standard library, and a
/// detached reader thread would leak while an escaped child holds the write end.
///
/// The timeout is armed here, before the command runs. Darwin refuses `SO_RCVTIMEO` with
/// `EINVAL` on a socket pair whose writing end the child has already closed, so arming it in
/// the reader thread raced every command that finished first.
fn capture_pair(argv0: &str) -> Result<(UnixStream, UnixStream), ExecError> {
    let (reader, writer) = UnixStream::pair().map_err(|source| spawn_failure(argv0, source))?;
    reader
        .set_read_timeout(Some(CANCEL_POLL))
        .map_err(|source| spawn_failure(argv0, source))?;
    Ok((reader, writer))
}

/// Reads one stream until its end, or until the command's group is settled and its writer is
/// still there. Past `settled`, a stream that keeps yielding bytes or stays silent is given
/// [`DRAIN_GRACE`] from the first settled read and then reported open,
/// so a child that left the group cannot hold the run (larry#334). The read timeout
/// [`capture_pair`] armed makes the flag visible within [`CANCEL_POLL`], so a cancel is
/// honoured just as quickly.
fn capture_bounded(
    mut stream: UnixStream,
    settled: &AtomicBool,
    callback: Option<&StreamCallback>,
) -> io::Result<Drained> {
    let origin = Instant::now();
    drain_with_callback(
        |chunk| stream.read(chunk),
        || origin.elapsed(),
        settled,
        callback,
    )
}

/// [`capture_bounded`]'s decisions over a read and a clock the caller supplies, so the seeded
/// sim drives the grace on a virtual clock rather than the wall (larry#334, SD-4).
#[cfg(test)]
fn drain(
    read: impl FnMut(&mut [u8]) -> io::Result<usize>,
    now: impl Fn() -> Duration,
    settled: &AtomicBool,
) -> io::Result<Drained> {
    drain_with_callback(read, now, settled, None)
}

/// The bounded drain with an optional live-output callback. The ordinary drain remains the
/// seeded reference path; the callback path keeps the report capture and its head+tail bounds
/// unchanged while splitting every observed read into at most [`STREAM_CHUNK`] bytes.
fn drain_with_callback(
    mut read: impl FnMut(&mut [u8]) -> io::Result<usize>,
    now: impl Fn() -> Duration,
    settled: &AtomicBool,
    callback: Option<&StreamCallback>,
) -> io::Result<Drained> {
    let mut captured = Captured::default();
    let mut chunk = [0_u8; CAPTURE_CHUNK];
    let mut grace: Option<Duration> = None;
    loop {
        // Read the flag before the read: a read issued once the group is known gone, which
        // still finds nothing, was issued against a peer that is really still there. A read
        // issued before the flag was seen may merely have raced the group's last exit.
        let settled_now = settled.load(Ordering::Relaxed);
        if settled_now {
            let at = now();
            let until = *grace.get_or_insert_with(|| match at.checked_add(DRAIN_GRACE) {
                Some(until) => until,
                // An unrepresentable grace reads as already passed: the stream is reported
                // open, never awaited without a bound.
                None => at,
            });
            if at >= until {
                return Ok(Drained {
                    captured,
                    open: true,
                });
            }
        }
        match read(&mut chunk) {
            Ok(0) => {
                // The tail is cut to its bounded size here, as the reference capture does:
                // the marker line and the reported counts depend on it.
                if captured.tail.len() > TAIL_BYTES {
                    captured.trim()?;
                }
                return Ok(Drained {
                    captured,
                    open: false,
                });
            }
            Ok(read) => {
                let filled = chunk.get(..read).ok_or_else(|| {
                    io::Error::other("a read reported more bytes than its buffer holds")
                })?;
                if let Some(callback) = callback {
                    for piece in filled.chunks(STREAM_CHUNK) {
                        callback(piece);
                    }
                }
                captured.push(filled)?;
            }
            // A silent read is retried like an interrupted one, settled or not: a sibling command
            // forked between this pair's creation and its own `exec` holds a copy of the writer,
            // and on a loaded host that copy outlives many read timeouts (larry#454). The grace
            // above, on the clock, is what ends a stream a child that left the group still holds.
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error),
        }
    }
}

/// The typed refusal of a command whose stream a child outside its group still holds open.
fn output_held(argv0: &str) -> ExecError {
    spawn_failure(
        argv0,
        io::Error::other("an output stream stayed open after the command ended"),
    )
}

/// Waits for one bounded reader and turns its failure into a typed error.
fn join_reader(
    reader: ScopedJoinHandle<'_, io::Result<Drained>>,
    argv0: &str,
) -> Result<Drained, ExecError> {
    match reader.join() {
        Ok(Ok(drained)) => Ok(drained),
        Ok(Err(source)) => Err(spawn_failure(argv0, source)),
        Err(payload) => {
            let text = format!(
                "output reader panicked: {}",
                crate::panic_text(payload.as_ref())
            );
            Err(spawn_failure(argv0, io::Error::other(text)))
        }
    }
}

/// The status of a leader whose group was just signalled, the signal repeated each
/// [`CANCEL_POLL`] slice until it arrives (larry#442).
fn killed_status(
    clock: &Clock,
    receiver: &mpsc::Receiver<io::Result<ExitStatus>>,
    pgid: i32,
) -> Result<io::Result<ExitStatus>, ExecError> {
    status_after_kill(clock, receiver, pgid, CANCEL_POLL, || {
        settle_group(clock, pgid)
    })
}

/// Builds a spawn failure that names the program.
fn spawn_failure(argv0: &str, source: io::Error) -> ExecError {
    ExecError::Spawn {
        argv0: argv0.to_owned(),
        source,
    }
}

/// Settling a command's process group, bounded by one clock reading (larry#268, larry#427).
#[path = "group.rs"]
mod group;

/// Seeded simulations of the bounded capture (larry#206).
#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;

/// Seeded simulations of the bounded drain's grace, on a virtual clock (larry#334).
#[cfg(test)]
#[path = "sim_drain.rs"]
mod sim_drain;
