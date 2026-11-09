//! The shipped `larry serve` as a child process, and one HTTP/1.1 exchange
//! against it. Shared by `tests/serve_journey.rs` and `tests/sim_serve.rs`
//! (larry#88), so the host is started, read and stopped one way.

use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::time::Duration;

use lgwks_std::json::{self, Value};

/// How long one exchange may take before the host is called broken: a bound on
/// a hung host, never the pace of a working one.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(20);

/// A running `larry serve` and the address it printed. Dropping it kills the
/// child, so a failed test leaves no host behind.
pub struct Serving {
    /// The child; `None` once it was waited for.
    child: Option<Child>,
    /// The address from the serving line.
    addr: SocketAddr,
}

/// The child's stderr, read whole after it exited.
fn stderr_text(stderr: Option<ChildStderr>) -> String {
    let mut text = String::new();
    if let Some(mut stream) = stderr {
        // A closed or unreadable stderr leaves the text empty; the exit status is the evidence.
        stream.read_to_string(&mut text).ok();
    }
    text
}

impl Serving {
    /// Start `larry serve` with only PATH and `env` set, from the system temp
    /// directory, and read its serving line.
    pub fn start(env: &[(&str, &str)]) -> Result<Self, Box<dyn Error>> {
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
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line)?;
        if line.is_empty() {
            let status = child.wait()?;
            let stderr = stderr_text(child.stderr.take());
            return Err(format!("larry serve printed no serving line: {status}, {stderr}").into());
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
            child: Some(child),
            addr,
        })
    }

    /// The address the host is serving on.
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Send `signal` (`TERM` or `KILL`) and wait for the exit; its status and stderr.
    pub fn signal_and_wait(
        &mut self,
        signal: &str,
    ) -> Result<(ExitStatus, String), Box<dyn Error>> {
        let mut child = self.child.take().ok_or("the host was already waited for")?;
        let sent = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(child.id().to_string())
            .status()?;
        if !sent.success() {
            return Err(format!("kill -{signal} failed: {sent}").into());
        }
        let status = child.wait()?;
        Ok((status, stderr_text(child.stderr.take())))
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // The child may already be gone; either way it is reaped here.
            child.kill().ok();
            child.wait().ok();
        }
    }
}

/// One request on a fresh connection: its status and body. The connection
/// closes after the answer, so the read ends with it.
pub fn exchange(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, String), Box<dyn Error>> {
    let head = format!(
        "{method} {path} HTTP/1.1\r\nhost: {addr}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(EXCHANGE_TIMEOUT))?;
    stream.set_write_timeout(Some(EXCHANGE_TIMEOUT))?;
    stream.write_all(head.as_bytes())?;
    // A refused body (413) may close before it is all written; the answer is still read.
    stream.write_all(body).ok();
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

/// `GET path` with no body.
pub fn get(addr: SocketAddr, path: &str) -> Result<(u16, String), Box<dyn Error>> {
    exchange(addr, "GET", path, b"")
}

/// The probe body the shipped binary answers, built independently of it.
pub fn probe_body() -> String {
    format!(
        "{{\"status\":\"ok\",\"service\":\"larry\",\"version\":\"{}\"}}",
        env!("CARGO_PKG_VERSION")
    )
}
