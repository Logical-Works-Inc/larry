//! Drives the shipped `larry mcp` binary over stdio for the MCP suites.
//!
//! Source: the MCP specification 2025-06-18, "Transports: stdio" (one
//! JSON-RPC message per line on the child's stdin and stdout). Every suite
//! speaks to the real binary through its public interface and nothing else.

#[path = "../support/larry_child.rs"]
mod larry_child;

use lgwks_std::json::{self, Value};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, ExitStatus, Stdio};

/// A fallible test step; failures propagate instead of panicking.
pub(crate) type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

#[path = "../support/rng.rs"]
mod rng;
pub(crate) use rng::Rng;

/// One running `larry mcp` process and its two pipes.
pub(crate) struct Session {
    /// The server process.
    child: Child,
    /// Write end of the server's stdin; `None` once closed.
    stdin: Option<ChildStdin>,
    /// Buffered read end of the server's stdout.
    stdout: BufReader<ChildStdout>,
}

impl Session {
    /// Starts `larry mcp` with no executor behind it: piped stdio, discarded
    /// stderr, and an environment holding only PATH, so no developer variable
    /// (`LARRY_STATE_DIR`, `LARRY_MODEL_KEY`) can reach the server.
    pub(crate) fn start() -> TestResult<Self> {
        Self::start_with(&[])
    }

    /// `start` with `env` set as well: the operator's executor settings.
    pub(crate) fn start_with(env: &[(&str, &OsStr)]) -> TestResult<Self> {
        let (child, stdin, stdout) = larry_child::start("mcp", env, Stdio::null())?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout,
        })
    }

    /// Writes bytes exactly as given, with no terminator.
    pub(crate) fn send_raw(&mut self, bytes: &[u8]) -> TestResult<()> {
        let stdin = self.stdin.as_mut().ok_or("stdin already closed")?;
        stdin.write_all(bytes)?;
        stdin.flush()?;
        Ok(())
    }

    /// Writes one frame: the bytes plus a newline.
    pub(crate) fn send(&mut self, frame: &[u8]) -> TestResult<()> {
        self.send_raw(frame)?;
        self.send_raw(b"\n")
    }

    /// Reads one reply line, newline included; fails if the server closed stdout.
    pub(crate) fn recv_line(&mut self) -> TestResult<Vec<u8>> {
        let mut line = Vec::new();
        self.stdout.read_until(b'\n', &mut line)?;
        if line.last() != Some(&b'\n') {
            return Err("server closed stdout before a full reply line".into());
        }
        Ok(line)
    }

    /// Reads one reply and parses it.
    pub(crate) fn recv(&mut self) -> TestResult<Value> {
        Ok(json::from_slice(&self.recv_line()?)?)
    }

    /// Sends one request value and returns its reply.
    pub(crate) fn request(&mut self, frame: &Value) -> TestResult<Value> {
        self.send(&json::to_vec(frame)?)?;
        self.recv()
    }

    /// Runs the lifecycle handshake: initialize, then the initialized notification.
    pub(crate) fn initialize(&mut self) -> TestResult<Value> {
        let reply = self.request(&json::json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "mcp-support", "version": "0" }
            }
        }))?;
        self.send(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)?;
        Ok(reply)
    }

    /// Closes stdin: the server sees end of input.
    pub(crate) fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Sends SIGKILL and returns the status; a killed process has no exit code.
    pub(crate) fn kill(mut self) -> TestResult<ExitStatus> {
        self.child.kill()?;
        Ok(self.child.wait()?)
    }

    /// Closes stdin (end of input) and waits for the server's exit status.
    pub(crate) fn finish(mut self) -> TestResult<ExitStatus> {
        self.close_stdin();
        Ok(self.child.wait()?)
    }
}

impl Drop for Session {
    /// A session abandoned by a failing test never leaves a running server behind.
    /// A server that would not stop fails the test that owned it, unless that test is already
    /// failing.
    fn drop(&mut self) {
        let stopped = match self.child.try_wait() {
            Ok(Some(_status)) => Ok(()),
            Ok(None) => self
                .child
                .kill()
                .and_then(|()| self.child.wait().map(|_status| ())),
            Err(cause) => Err(cause),
        };
        assert!(
            std::thread::panicking() || stopped.is_ok(),
            "the abandoned server would not stop: {stopped:?}"
        );
    }
}
