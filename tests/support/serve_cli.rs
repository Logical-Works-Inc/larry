//! The shipped `larry serve` as a child process, its `serve key` issuer, its
//! `serve revoke` runner, a bulk key-store filler, and one HTTP/1.1 exchange
//! against it. Shared by `tests/serve_journey.rs` and `tests/sim_serve.rs`
//! (larry#88, larry#89, larry#210), so the host is started, keyed, revoked, read
//! and stopped one way. Its standard output is read by one thread with a
//! deadline, so a host that never prints a line fails the test instead of
//! hanging it (larry#269).

use std::error::Error;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use lgwks_deps::storefront::rusqlite;
use lgwks_std::json::{self, Value};

#[path = "temp_dir.rs"]
mod temp_dir;
pub(crate) use temp_dir::TempDir;

/// How long one exchange may take before the host is called broken: a bound on
/// a hung host, never the pace of a working one. A serving line and every later
/// line share it.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(20);

/// Lines the reader may hold before it waits for the test: the host prints a
/// serving line and one acknowledgement per SIGHUP, so this bound is never met
/// by a working host.
const LINE_BACKLOG: usize = 64;

/// A fresh, empty state directory under the system temp dir, removed when its
/// guard drops (larry#267).
pub(crate) fn fresh_state_dir() -> std::io::Result<TempDir> {
    TempDir::new()
}

/// `larry serve key <tenant>` against `state`: the key it printed once.
pub(crate) fn issue_key(state: &Path, tenant: &str) -> Result<String, Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_larry"))
        .args(["serve", "key", tenant])
        .env_clear()
        .env("LARRY_STATE_DIR", state)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "serve key {tenant:?}: {}, {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let printed: Value = json::from_slice(&output.stdout)?;
    Ok(String::from(
        printed
            .get("key")
            .and_then(Value::as_str)
            .ok_or("serve key prints its key")?,
    ))
}

/// `larry serve revoke <tenant>` against `state`: the child's whole output, so
/// a test reads its status and its refusal text.
pub(crate) fn revoke(state: &Path, tenant: &str) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_larry"))
        .args(["serve", "revoke", tenant])
        .env_clear()
        .env("LARRY_STATE_DIR", state)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .output()?)
}

/// Inserts `count` tenant key rows straight into the store `issue_key` created in
/// `state`, named `tenant-<from>` onward, each with a 64 hex digest. One
/// transaction, so filling the cap stays fast.
pub(crate) fn fill_key_store(
    state: &Path,
    from: usize,
    count: usize,
) -> Result<(), Box<dyn Error>> {
    let mut conn = rusqlite::Connection::open(state.join("executor.db"))?;
    let transaction = conn.transaction()?;
    for offset in 0..count {
        let index = from.saturating_add(offset);
        transaction.execute(
            "INSERT INTO tenant_keys (tenant, digest) VALUES (?1, ?2)",
            rusqlite::params![format!("tenant-{index}"), format!("{index:064x}")],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// A running `larry serve` and the address it printed. Dropping it kills the
/// child, so a failed test leaves no host behind.
pub(crate) struct Serving {
    /// The child; `None` once it was waited for.
    child: Option<Child>,
    /// The child's process id, for sending it a signal.
    pid: u32,
    /// The child's standard output, read one line at a time by `reader`.
    lines: Option<Receiver<io::Result<String>>>,
    /// The reader thread, joined with the host.
    reader: Option<Reader>,
    /// The address from the serving line.
    addr: SocketAddr,
}

/// Reads `stdout` one line at a time on a thread the [`Serving`] owns, so the
/// test waits on the channel with a deadline instead of blocking in
/// `read_line` (larry#269). Killing the child closes the pipe and ends the
/// thread; an empty line means the output ended, and a read error is its only
/// error.
fn forward_lines(
    mut stdout: BufReader<ChildStdout>,
) -> io::Result<(Receiver<io::Result<String>>, Reader)> {
    let (sender, lines): (SyncSender<io::Result<String>>, _) = mpsc::sync_channel(LINE_BACKLOG);
    // The thread ends on the end of output, a read error or a closed channel, and hands back
    // the one message the closed channel no longer took, so no send result is lost.
    let reader = std::thread::Builder::new()
        .name(String::from("serve-lines"))
        .spawn(move || {
            loop {
                let mut line = String::new();
                let read = stdout.read_line(&mut line);
                let last = !matches!(read, Ok(count) if count > 0);
                if let Err(unsent) = sender.send(read.map(|_count| line)) {
                    return Some(unsent.0);
                }
                if last {
                    return None;
                }
            }
        })?;
    Ok((lines, reader))
}

/// The reader thread of [`forward_lines`]: it returns the message it could not deliver.
type Reader = JoinHandle<Option<io::Result<String>>>;

/// Kills and reaps `child`, closes `lines` and joins `reader`, in that order. A line the closed
/// channel no longer took is expected after the kill; a read error or a panic is not.
fn shut_down(
    child: Option<Child>,
    lines: Option<Receiver<io::Result<String>>>,
    reader: Option<Reader>,
) -> Result<(), Box<dyn Error>> {
    if let Some(mut child) = child {
        child.kill()?;
        child.wait()?;
    }
    {
        // Closing the receiver before the join returns a reader parked on a full channel.
        let _closed = lines;
    }
    match reader.map(JoinHandle::join) {
        None | Some(Ok(None | Some(Ok(_)))) => Ok(()),
        Some(Ok(Some(Err(cause)))) => {
            Err(format!("the host's stdout failed after the test stopped: {cause}").into())
        }
        Some(Err(panic)) => Err(format!("the serve-lines reader panicked: {panic:?}").into()),
    }
}

/// The next line `lines` carries within `within`, or a typed failure naming the
/// bound: a host that never prints fails instead of hanging the test (larry#269).
fn await_line(
    lines: &Receiver<io::Result<String>>,
    within: Duration,
) -> Result<String, Box<dyn Error>> {
    match lines.recv_timeout(within) {
        Ok(Ok(line)) => Ok(line),
        Ok(Err(cause)) => Err(format!("the host's stdout could not be read: {cause}").into()),
        Err(RecvTimeoutError::Timeout) => {
            Err(format!("the host printed no line within {within:?}").into())
        }
        Err(RecvTimeoutError::Disconnected) => {
            Err("the host's stdout closed without a line".into())
        }
    }
}

/// The child's stderr, read whole after it exited.
fn stderr_text(stderr: Option<ChildStderr>) -> String {
    let mut text = String::new();
    if let Some(mut stream) = stderr {
        match stream.read_to_string(&mut text) {
            Ok(_) => {}
            // The child closed its stderr before the read ended: the text says so, and the exit
            // status stays the evidence.
            Err(cause) => {
                text.push_str("<stderr unreadable: ");
                text.push_str(&cause.to_string());
                text.push('>');
            }
        }
    }
    text
}

impl Serving {
    /// Start `larry serve` with only PATH and `env` set, from the system temp
    /// directory, and read its serving line within [`EXCHANGE_TIMEOUT`]: a host
    /// that never prints fails typed instead of hanging the test (larry#269).
    pub(crate) fn start(env: &[(&str, &str)]) -> Result<Self, Box<dyn Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_larry"))
            .arg("serve")
            .env_clear()
            .env("PATH", std::env::var("PATH")?)
            .envs(env.iter().copied())
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or("the child's stdout was not piped")?;
        let (lines, reader) = forward_lines(BufReader::new(stdout))?;
        let line = match await_line(&lines, EXCHANGE_TIMEOUT) {
            Ok(line) => line,
            Err(cause) => {
                // The host never printed: kill and reap it, then return the refusal, with the
                // stop's own failure when there is one.
                return Err(match shut_down(Some(child), Some(lines), Some(reader)) {
                    Ok(()) => cause,
                    Err(stop) => format!("{cause}; the host would not stop: {stop}").into(),
                });
            }
        };
        if line.is_empty() {
            let status = child.wait()?;
            let stderr = stderr_text(child.stderr.take());
            let stopped = shut_down(None, Some(lines), Some(reader));
            return Err(format!(
                "larry serve printed no serving line: {status}, {stderr}; reader: {stopped:?}"
            )
            .into());
        }
        let printed: Value = json::from_str(&line)?;
        let serving = printed
            .get("serving")
            .and_then(Value::as_str)
            .ok_or("the serving line names its address")?;
        let addr = serving
            .strip_prefix("http://")
            .ok_or("the serving address is http")?
            .parse()?;
        Ok(Self {
            pid: child.id(),
            child: Some(child),
            lines: Some(lines),
            reader: Some(reader),
            addr,
        })
    }

    /// The address the host is serving on.
    pub(crate) const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The next line the host prints on standard output, without its newline,
    /// within [`EXCHANGE_TIMEOUT`].
    pub(crate) fn next_line(&mut self) -> Result<String, Box<dyn Error>> {
        let lines = self.lines.as_ref().ok_or("the host's output is closed")?;
        let line = await_line(lines, EXCHANGE_TIMEOUT)?;
        if line.is_empty() {
            return Err("the host printed no further line".into());
        }
        Ok(String::from(line.trim_end()))
    }

    /// Send `signal` (`TERM`, `HUP`, `KILL` and the like) without waiting.
    pub(crate) fn signal(&self, signal: &str) -> Result<(), Box<dyn Error>> {
        let sent = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(self.pid.to_string())
            .status()?;
        if !sent.success() {
            return Err(format!("kill -{signal} failed: {sent}").into());
        }
        Ok(())
    }

    /// Send `signal` (`TERM` or `KILL`) and wait for the exit; its status and stderr.
    pub(crate) fn signal_and_wait(
        &mut self,
        signal: &str,
    ) -> Result<(ExitStatus, String), Box<dyn Error>> {
        self.signal(signal)?;
        let mut child = self.child.take().ok_or("the host was already waited for")?;
        let status = child.wait()?;
        Ok((status, stderr_text(child.stderr.take())))
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        // A child already gone still kills and reaps cleanly; a host that would not stop fails
        // the test that owned it, unless that test is already failing.
        let stopped = shut_down(self.child.take(), self.lines.take(), self.reader.take());
        assert!(
            std::thread::panicking() || stopped.is_ok(),
            "the serve host would not stop: {stopped:?}"
        );
    }
}

/// A host that never prints its serving line fails the start with a typed
/// message inside the bound, never a hang (larry#269): `sh` blocked on a pipe
/// that stays open is that host, with no wall-clock sleep anywhere.
#[test]
fn a_host_that_never_prints_its_line_fails_within_the_bound() -> Result<(), Box<dyn Error>> {
    let mut silent = Command::new("sh")
        .args(["-c", "read line"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = silent
        .stdout
        .take()
        .ok_or("the silent host's stdout was not piped")?;
    let (lines, reader) = forward_lines(BufReader::new(stdout))?;
    let bound = Duration::from_millis(150);
    let message = match await_line(&lines, bound) {
        Ok(line) => return Err(format!("the silent host printed {line:?}").into()),
        Err(cause) => cause.to_string(),
    };
    shut_down(Some(silent), Some(lines), Some(reader))?;
    assert!(
        message.contains("printed no line within"),
        "the failure names the bound: {message}"
    );
    Ok(())
}

/// One request on a fresh connection, with `key` as its bearer token when
/// given: its status and body. The connection closes after the answer, so the
/// read ends with it.
pub(crate) fn exchange(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
    key: Option<&str>,
) -> Result<(u16, String), Box<dyn Error>> {
    let auth = match key {
        Some(token) => format!("authorization: Bearer {token}\r\n"),
        None => String::new(),
    };
    let head = format!(
        "{method} {path} HTTP/1.1\r\nhost: {addr}\r\n{auth}content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(EXCHANGE_TIMEOUT))?;
    stream.set_write_timeout(Some(EXCHANGE_TIMEOUT))?;
    stream.write_all(head.as_bytes())?;
    // A refused body (413) may close before it is all written; the answer is still read below.
    match stream.write_all(body) {
        Ok(()) => {}
        // The host answered and closed its side mid-body: its answer is what this exchange reads.
        Err(cause)
            if cause.kind() == std::io::ErrorKind::BrokenPipe
                || cause.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(cause) => return Err(cause.into()),
    }
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer)?;
    let text = String::from_utf8(answer)?;
    let (status_line, rest) = text.split_once("\r\n").ok_or("no status line")?;
    let status = status_line
        .strip_prefix("HTTP/1.1 ")
        .and_then(|line| line.get(..3))
        .ok_or("not an HTTP/1.1 status line")?
        .parse()?;
    let (_, answered) = rest.split_once("\r\n\r\n").ok_or("no end of head")?;
    Ok((status, String::from(answered)))
}

/// `GET path` with no body, as `key` when given.
pub(crate) fn get(
    addr: SocketAddr,
    path: &str,
    key: Option<&str>,
) -> Result<(u16, String), Box<dyn Error>> {
    exchange(addr, "GET", path, b"", key)
}

/// The probe body the shipped binary answers, built independently of it.
pub(crate) fn probe_body() -> String {
    format!(
        "{{\"status\":\"ok\",\"service\":\"larry\",\"version\":\"{}\"}}",
        env!("CARGO_PKG_VERSION")
    )
}
