//! Drives one shipped `larry acp` process over its stdio transport for the Harbor journeys
//! (larry#93), split out of `acp_harbor_journey.rs` by responsibility (larry#341): this file
//! speaks the protocol, the journey file asserts what a user sees.
//!
//! Source: the Agent Client Protocol v1 (initialize, session/new, session/prompt,
//! session/update, stdio transport: one JSON-RPC message per line).

use std::error::Error;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Stdio};

use lgwks_std::json::{self, Value, json};

use crate::json_at::At;
use crate::larry_child;

/// What every step here returns.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// One `larry acp` process: its protocol pipes and its log.
pub(crate) struct Agent {
    /// The process.
    child: Child,
    /// The protocol stream in; `None` once closed.
    stdin: Option<ChildStdin>,
    /// The protocol stream out.
    out: BufReader<ChildStdout>,
    /// The agent's log.
    log: BufReader<ChildStderr>,
}

impl Agent {
    /// Starts `larry acp` with only PATH and `env` set.
    pub(crate) fn start(env: &[(&str, &OsStr)]) -> TestResult<Self> {
        let (mut child, stdin, out) = larry_child::start("acp", env, Stdio::piped())?;
        let log = BufReader::new(child.stderr.take().ok_or("stderr was not piped")?);
        Ok(Self {
            child,
            stdin: Some(stdin),
            out,
            log,
        })
    }

    /// Writes one message as its line.
    pub(crate) fn send(&mut self, message: &Value) -> TestResult {
        let stdin = self.stdin.as_mut().ok_or("stdin is closed")?;
        let mut line = message.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Reads one message; end of output is an error.
    pub(crate) fn recv(&mut self) -> TestResult<Value> {
        let mut line = String::new();
        if self.out.read_line(&mut line)? == 0 {
            return Err("larry acp closed its output".into());
        }
        Ok(json::from_str(&line)?)
    }

    /// Reads until the response to `id`, returning the notifications before it.
    pub(crate) fn until(&mut self, id: &Value) -> TestResult<(Vec<Value>, Value)> {
        let mut notes = Vec::new();
        loop {
            let message = self.recv()?;
            if message.get("method").is_some() {
                notes.push(message);
            } else if message.get("id") == Some(id) {
                return Ok((notes, message));
            }
        }
    }

    /// Sends a request and reads to its response.
    pub(crate) fn call(&mut self, request: &Value) -> TestResult<(Vec<Value>, Value)> {
        self.send(request)?;
        self.until(request.at("/id")?)
    }

    /// Initializes and opens one session in `cwd`, returning its id.
    pub(crate) fn open(&mut self, cwd: &Path) -> TestResult<String> {
        let (_, init) = self.call(&json!({"jsonrpc":"2.0","id":0,"method":"initialize",
            "params":{"protocolVersion":1,"clientCapabilities":{"terminal":false}}}))?;
        assert_eq!(*init.at("/result/protocolVersion")?, 1, "version 1");
        let (_, new) = self.call(&json!({"jsonrpc":"2.0","id":1,"method":"session/new",
            "params":{"cwd":cwd.to_string_lossy(),"mcpServers":[]}}))?;
        let session = new
            .at("/result/sessionId")?
            .as_str()
            .ok_or("session/new answers a session id")?;
        Ok(session.to_owned())
    }

    /// Reads log lines until one holds `text`.
    pub(crate) fn logged(&mut self, text: &str) -> TestResult {
        let mut line = String::new();
        loop {
            line.clear();
            if self.log.read_line(&mut line)? == 0 {
                return Err(format!("the log ended before {text:?}").into());
            }
            if line.contains(text) {
                return Ok(());
            }
        }
    }

    /// Sends SIGKILL mid-session and reaps the process: the crash the next start recovers.
    pub(crate) fn kill_9(&mut self) -> TestResult {
        self.child.kill()?;
        self.child.wait()?;
        Ok(())
    }

    /// Closes stdin and returns the exit code.
    pub(crate) fn finish(mut self) -> TestResult<Option<i32>> {
        self.stdin = None;
        Ok(self.child.wait()?.code())
    }
}

impl Drop for Agent {
    /// A journey that already ended its agent leaves nothing to kill. A child that cannot be
    /// reaped fails the test, unless it is already failing.
    fn drop(&mut self) {
        let reaped = match self.child.try_wait() {
            Ok(Some(_status)) => Ok(()),
            Ok(None) => self
                .child
                .kill()
                .and_then(|()| self.child.wait().map(|_status| ())),
            Err(cause) => Err(cause),
        };
        assert!(
            reaped.is_ok() || std::thread::panicking(),
            "the larry acp child is killed and reaped: {reaped:?}"
        );
    }
}

/// A `session/prompt` request carrying `text`.
pub(crate) fn prompt(id: u64, session: &str, text: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"session/prompt",
        "params":{"sessionId":session,"prompt":[{"type":"text","text":text}]}})
}

/// The `sessionUpdate` kinds of `notes`, in order.
pub(crate) fn kinds(notes: &[Value]) -> Vec<String> {
    notes
        .iter()
        .filter_map(|note| note.member("/params/update/sessionUpdate")?.as_str())
        .map(str::to_owned)
        .collect()
}

/// The text of the `agent_message_chunk` among `notes`.
pub(crate) fn said(notes: &[Value]) -> Option<&str> {
    notes
        .iter()
        .find(|note| {
            note.member("/params/update/sessionUpdate")
                .and_then(Value::as_str)
                == Some("agent_message_chunk")
        })
        .and_then(|note| note.member("/params/update/content/text")?.as_str())
}

/// The text of message `index` of the model request `body`.
pub(crate) fn message_text(body: &[u8], index: usize) -> TestResult<String> {
    let request = json::from_slice::<Value>(body)?;
    let text = request
        .at(&format!("/messages/{index}/content"))?
        .as_str()
        .ok_or("the request carries that message")?;
    Ok(text.to_owned())
}
