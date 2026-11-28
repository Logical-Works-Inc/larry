//! Drives one shipped `larry acp` process over its stdio transport for the Harbor journeys
//! (larry#93), split out of `acp_harbor_journey.rs` by responsibility (larry#341): this file
//! speaks the protocol, the journey file asserts what a user sees.
//!
//! Source: the Agent Client Protocol v1 (initialize, session/new, session/prompt,
//! session/update, stdio transport: one JSON-RPC message per line).

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use lgwks_std::json::{self, Value, json};

use crate::json_at::At;
use crate::larry_child;
use crate::larry_guard::Larry;

use crate::coverage::forward;

/// What every step here returns.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// One `larry acp` process: its protocol pipes and its log.
pub(crate) struct Agent {
    /// The process, killed and reaped when the agent drops.
    child: Larry,
    /// The state ledger the ACP process reads.
    state: PathBuf,
    /// The policy to persist before opening each work root; empty means deny-all.
    policy: OsString,
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
        let state = env
            .iter()
            .find_map(|&(name, value)| (name == "LARRY_STATE_DIR").then(|| PathBuf::from(value)))
            .ok_or("ACP test agent needs LARRY_STATE_DIR")?;
        let policy = match env
            .iter()
            .find_map(|&(name, value)| (name == "LARRY_POLICY").then(|| value.to_os_string()))
        {
            Some(policy) => policy,
            None => OsString::from(""),
        };
        let (mut child, stdin, out) = larry_child::start("acp", env, Stdio::piped())?;
        let log = BufReader::new(child.stderr.take().ok_or("stderr was not piped")?);
        Ok(Self {
            child,
            state,
            policy,
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
        let mut provision = Command::new(env!("CARGO_BIN_EXE_larry"));
        provision
            .args(["serve", "provision"])
            .arg(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH")?)
            .env("LARRY_STATE_DIR", &self.state)
            .env("LARRY_POLICY", &self.policy)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        forward(&mut provision);
        let provision = provision.output()?;
        if !provision.status.success() {
            return Err(format!(
                "serve provision refused {}: {}",
                cwd.display(),
                String::from_utf8_lossy(&provision.stderr)
            )
            .into());
        }
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
