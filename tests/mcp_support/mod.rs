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
pub type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

/// splitmix64: the one source of randomness, seeded per run.
pub struct Rng(pub u64);

impl Rng {
    /// The next 64 random bits.
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ mixed.wrapping_shr(30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ mixed.wrapping_shr(27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ mixed.wrapping_shr(31)
    }

    /// A value below `bound`, by multiply-shift: no modulo, no division.
    pub fn below(&mut self, bound: u32) -> u32 {
        let top = self.next().to_le_bytes();
        let high = u32::from_le_bytes([top[4], top[5], top[6], top[7]]);
        let scaled = u64::from(high)
            .wrapping_mul(u64::from(bound))
            .wrapping_shr(32)
            .to_le_bytes();
        u32::from_le_bytes([scaled[0], scaled[1], scaled[2], scaled[3]])
    }

    /// `below`, as a length or index.
    pub fn index(&mut self, bound: u32) -> TestResult<usize> {
        Ok(usize::try_from(self.below(bound))?)
    }
}

/// One running `larry mcp` process and its two pipes.
pub struct Session {
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
    pub fn start() -> TestResult<Self> {
        Self::start_with(&[])
    }

    /// `start` with `env` set as well: the operator's executor settings.
    pub fn start_with(env: &[(&str, &OsStr)]) -> TestResult<Self> {
        let (child, stdin, stdout) = larry_child::start("mcp", env, Stdio::null())?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout,
        })
    }

    /// Writes bytes exactly as given, with no terminator.
    pub fn send_raw(&mut self, bytes: &[u8]) -> TestResult<()> {
        let stdin = self.stdin.as_mut().ok_or("stdin already closed")?;
        stdin.write_all(bytes)?;
        stdin.flush()?;
        Ok(())
    }

    /// Writes one frame: the bytes plus a newline.
    pub fn send(&mut self, frame: &[u8]) -> TestResult<()> {
        self.send_raw(frame)?;
        self.send_raw(b"\n")
    }

    /// Reads one reply line, newline included; fails if the server closed stdout.
    pub fn recv_line(&mut self) -> TestResult<Vec<u8>> {
        let mut line = Vec::new();
        self.stdout.read_until(b'\n', &mut line)?;
        if line.last() != Some(&b'\n') {
            return Err("server closed stdout before a full reply line".into());
        }
        Ok(line)
    }

    /// Reads one reply and parses it.
    pub fn recv(&mut self) -> TestResult<Value> {
        Ok(json::from_slice(&self.recv_line()?)?)
    }

    /// Sends one request value and returns its reply.
    pub fn request(&mut self, frame: &Value) -> TestResult<Value> {
        self.send(&json::to_vec(frame)?)?;
        self.recv()
    }

    /// Runs the lifecycle handshake: initialize, then the initialized notification.
    pub fn initialize(&mut self) -> TestResult<Value> {
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
    pub fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    /// Sends SIGKILL and returns the status; a killed process has no exit code.
    pub fn kill(mut self) -> TestResult<ExitStatus> {
        self.child.kill()?;
        Ok(self.child.wait()?)
    }

    /// Closes stdin (end of input) and waits for the server's exit status.
    pub fn finish(mut self) -> TestResult<ExitStatus> {
        self.close_stdin();
        Ok(self.child.wait()?)
    }
}

impl Drop for Session {
    /// A session abandoned by a failing test never leaves a running server behind.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) && self.child.kill().is_ok() {
            drop(self.child.wait());
        }
    }
}
