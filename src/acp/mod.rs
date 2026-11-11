//! `larry acp`: Larry as an Agent Client Protocol agent on stdio (larry#93), the
//! surface Harbor's generic `acp` agent runs Terminal-Bench 2.0 tasks through,
//! beside Claude Code, Codex CLI and OpenHands, with no Python anywhere.
//!
//! Written from the Agent Client Protocol, protocol version 1: `initialize`
//! (version negotiation and capabilities), `session/new` (an absolute `cwd`, the
//! `mcpServers` list, a unique session id), `session/prompt` (content blocks in,
//! `session/update` notifications out, one `stopReason` back) and the
//! `session/cancel` notification. A prompt runs one terminal session
//! (`executor::run_session`): every command is a `tool_call` of kind `execute`
//! and its `tool_call_update`, the model's summary is an `agent_message_chunk`,
//! and the result's `_meta.larry.report` is the sealed report `larry verify` checks.
//!
//! Larry connects to none of the `mcpServers` a client names and advertises no
//! authentication method, no `loadSession`, and no image or audio input. Framing
//! and envelope checks are `larry mcp`'s own (`mcp::wire`). A reader thread holds
//! the input so a `session/cancel` lands while a prompt runs: it flags the session,
//! and logs one line on standard error, which ACP leaves to the agent. The inbox
//! between the two holds at most 64 frames, so a client that floods while a prompt
//! runs waits on its own pipe; a process holds at most 16 sessions.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Mutex, MutexGuard};

use lgwks_std::json::{Map, Value, json};

use crate::executor::{ExecError, Observer, Settings, run_session};
use crate::mcp::wire::{self, Envelope, Frame};

/// The one protocol version Larry speaks; any other request is answered with it.
const PROTOCOL_VERSION: u64 = 1;
/// Most sessions one process opens.
const MAX_SESSIONS: usize = 16;
/// Most frames read ahead while a prompt runs.
const INBOX: usize = 64;
/// Longest tool-call title, in characters: the command's first line, cut.
const MAX_TITLE_CHARS: usize = 256;

/// One session this process opened.
#[derive(Debug)]
struct Entry {
    /// The directory every command runs in.
    cwd: PathBuf,
    /// Whether the client cancelled the prompt that is running.
    cancelled: bool,
}

/// The sessions, shared by the reader thread (which flags cancels) and the agent.
type Book = Mutex<BTreeMap<String, Entry>>;

/// Lock, recovering the map if a holder panicked: a flag is a plain bool.
fn locked(book: &Book) -> MutexGuard<'_, BTreeMap<String, Entry>> {
    match book.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Serves ACP until the reader reaches end of input. The executor's settings are
/// read from the environment once, as the process starts; a prompt on a process
/// whose settings were refused ends as a refusal naming why.
///
/// # Errors
///
/// The reader or the writer failed; a malformed or oversized frame is answered, never an error.
pub fn serve<R: BufRead + Send, W: Write>(reader: R, writer: W) -> io::Result<()> {
    let settings =
        Settings::from_env(|name| std::env::var(name).ok()).map_err(|error| error.to_string());
    serve_bounded(
        reader,
        writer,
        wire::MAX_FRAME_BYTES,
        &settings,
        io::stderr(),
    )
}

/// `serve` with an explicit frame bound, settings and log, so a unit test can use its own.
fn serve_bounded<R, W, L>(
    reader: R,
    mut writer: W,
    max: usize,
    settings: &Result<Settings, String>,
    log: L,
) -> io::Result<()>
where
    R: BufRead + Send,
    W: Write,
    L: Write + Send,
{
    let book: Book = Mutex::new(BTreeMap::new());
    let (sender, inbox) = sync_channel::<io::Result<Frame>>(INBOX);
    std::thread::scope(|scope| {
        let book_ref = &book;
        scope.spawn(move || read_loop(reader, max, book_ref, &sender, log));
        let mut agent = Agent {
            settings,
            book: &book,
            initialized: false,
            opened: 0,
        };
        let mut failed: Option<io::Error> = None;
        // The loop ends when the reader ends; a write fault stops replies but keeps
        // draining, so the reader is never left blocked on a full inbox.
        for received in &inbox {
            let frame = match received {
                Ok(frame) => frame,
                Err(cause) => {
                    failed.get_or_insert(cause);
                    continue;
                }
            };
            if failed.is_some() {
                continue;
            }
            let written = match frame {
                Frame::End => continue,
                Frame::Oversized => send(
                    &mut writer,
                    &wire::error(
                        &Value::Null,
                        wire::INVALID_REQUEST,
                        "frame exceeds the size limit and was discarded",
                    ),
                ),
                Frame::Line(bytes) => agent.handle(&bytes, &mut writer),
            };
            if let Err(cause) = written {
                failed = Some(cause);
            }
        }
        match failed {
            Some(cause) => Err(cause),
            None => Ok(()),
        }
    })
}

/// Reads frames into the inbox until end of input, flagging each `session/cancel`
/// on the way so it lands while a prompt runs.
fn read_loop<R: BufRead, L: Write>(
    mut reader: R,
    max: usize,
    book: &Book,
    inbox: &SyncSender<io::Result<Frame>>,
    mut log: L,
) {
    loop {
        let frame = wire::read_frame(&mut reader, max);
        let end = !matches!(frame, Ok(Frame::Line(_) | Frame::Oversized));
        if let Ok(Frame::Line(ref bytes)) = frame {
            mark(bytes, book, &mut log);
        }
        if inbox.send(frame).is_err() || end {
            return;
        }
    }
}

/// Applies a frame's effect on a session's cancel flag in stream order: a
/// `session/prompt` clears it and a `session/cancel` sets it, so a cancel sent
/// after a prompt always reaches that prompt, however far behind the agent runs.
fn mark<L: Write>(bytes: &[u8], book: &Book, log: &mut L) {
    let Ok(frame) = wire::object(bytes) else {
        return;
    };
    let (cancel, params) = match wire::envelope(&frame) {
        Ok(Envelope::Notification {
            method: "session/cancel",
            params: Some(params),
        }) => (true, params),
        Ok(Envelope::Request {
            method: "session/prompt",
            params: Some(params),
            ..
        }) => (false, params),
        _ => return,
    };
    let Some(session) = params.get("sessionId").and_then(Value::as_str) else {
        return;
    };
    if let Some(entry) = locked(book).get_mut(session) {
        entry.cancelled = cancel;
    }
    if cancel {
        // The log line is the observable acknowledgement that the cancel was
        // read; a failed log write changes nothing about the cancel itself.
        drop(writeln!(
            log,
            "larry acp: session {session} cancel requested"
        ));
    }
}

/// Writes one message as its line and flushes.
fn send<W: Write>(out: &mut W, value: &Value) -> io::Result<()> {
    out.write_all(&wire::encode(value))?;
    out.flush()
}

/// The agent's side of one connection.
struct Agent<'run> {
    /// The executor's settings, or why they were refused.
    settings: &'run Result<Settings, String>,
    /// The sessions this process opened.
    book: &'run Book,
    /// Whether `initialize` has been answered.
    initialized: bool,
    /// Sessions opened so far; the next session id's counter.
    opened: u64,
}

impl Agent<'_> {
    /// Answers one frame: a request gets one reply, written after any notifications
    /// its prompt sent; a notification or a response gets none.
    fn handle<W: Write>(&mut self, bytes: &[u8], out: &mut W) -> io::Result<()> {
        let frame = match wire::object(bytes) {
            Ok(frame) => frame,
            Err(reply) => return send(out, &reply),
        };
        match wire::envelope(&frame) {
            Err(Some(reply)) => send(out, &reply),
            Err(None) | Ok(Envelope::Notification { .. } | Envelope::Response) => Ok(()),
            Ok(Envelope::Request { id, method, params }) => {
                let reply = self.request(id, method, params, out)?;
                send(out, &reply)
            }
        }
    }

    /// The reply to one request.
    fn request<W: Write>(
        &mut self,
        id: &Value,
        method: &str,
        params: Option<&Map>,
        out: &mut W,
    ) -> io::Result<Value> {
        if method == "initialize" {
            return Ok(self.initialize(id, params));
        }
        if !self.initialized {
            return Ok(wire::error(
                id,
                wire::INVALID_REQUEST,
                "initialize must come first",
            ));
        }
        Ok(match method {
            "authenticate" => wire::error(
                id,
                wire::INVALID_PARAMS,
                "Larry advertises no authentication method",
            ),
            "session/new" => self.new_session(id, params),
            "session/prompt" => return self.prompt(id, params, out),
            _ => wire::error(id, wire::METHOD_NOT_FOUND, "method not found"),
        })
    }

    /// `initialize`: version 1 whatever was asked, and what Larry offers.
    fn initialize(&mut self, id: &Value, params: Option<&Map>) -> Value {
        if self.initialized {
            return wire::error(id, wire::INVALID_REQUEST, "initialize was already answered");
        }
        if params
            .and_then(|map| map.get("protocolVersion"))
            .and_then(Value::as_u64)
            .is_none()
        {
            return wire::error(
                id,
                wire::INVALID_PARAMS,
                "protocolVersion must be a non-negative integer",
            );
        }
        self.initialized = true;
        wire::result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": true },
                    "mcpCapabilities": { "http": false, "sse": false }
                },
                "authMethods": [],
                "agentInfo": { "name": "larry", "title": "Larry", "version": env!("CARGO_PKG_VERSION") }
            }),
        )
    }

    /// `session/new`: an absolute directory and the server list, under the session bound.
    fn new_session(&mut self, id: &Value, params: Option<&Map>) -> Value {
        let Some(cwd) = params
            .and_then(|map| map.get("cwd"))
            .and_then(Value::as_str)
        else {
            return wire::error(id, wire::INVALID_PARAMS, "cwd must be a string");
        };
        let cwd = Path::new(cwd);
        if !cwd.is_absolute() {
            return wire::error(id, wire::INVALID_PARAMS, "cwd must be an absolute path");
        }
        if !cwd.is_dir() {
            return wire::error(id, wire::INVALID_PARAMS, "cwd is not a directory");
        }
        if !params
            .and_then(|map| map.get("mcpServers"))
            .is_some_and(|servers| servers.as_array().is_some())
        {
            return wire::error(id, wire::INVALID_PARAMS, "mcpServers must be an array");
        }
        let mut book = locked(self.book);
        if book.len() >= MAX_SESSIONS {
            return wire::error(
                id,
                wire::INVALID_REQUEST,
                "this process already holds 16 sessions, its bound",
            );
        }
        let session = format!("larry-{}-{}", std::process::id(), self.opened);
        self.opened = self.opened.saturating_add(1);
        book.insert(
            session.clone(),
            Entry {
                cwd: cwd.to_path_buf(),
                cancelled: false,
            },
        );
        wire::result(id, json!({ "sessionId": session }))
    }

    /// `session/prompt`: runs one terminal session and answers with its stop reason.
    fn prompt<W: Write>(
        &mut self,
        id: &Value,
        params: Option<&Map>,
        out: &mut W,
    ) -> io::Result<Value> {
        let Some(session) = params
            .and_then(|map| map.get("sessionId"))
            .and_then(Value::as_str)
        else {
            return Ok(wire::error(
                id,
                wire::INVALID_PARAMS,
                "sessionId must be a string",
            ));
        };
        // The reader cleared this session's cancel flag as it read this prompt.
        let cwd = match locked(self.book).get(session) {
            Some(entry) => entry.cwd.clone(),
            None => return Ok(wire::error(id, wire::INVALID_PARAMS, "unknown session")),
        };
        let instruction = match instruction(params) {
            Ok(text) => text,
            Err(fault) => return Ok(wire::error(id, wire::INVALID_PARAMS, &fault.to_string())),
        };
        let mut updates = Updates {
            out,
            session,
            book: self.book,
            failed: None,
        };
        let settings = match *self.settings {
            Ok(ref settings) => settings,
            Err(ref reason) => {
                updates.message(reason);
                return updates.close(stopped(id, "refusal", None));
            }
        };
        match run_session(&instruction, &cwd, settings, &mut updates) {
            Ok(ended) => {
                let reply = stopped(id, ended.stop.name(), Some(ended.report.json()));
                updates.close(reply)
            }
            Err(ExecError::Session(cause)) => {
                updates.close(wire::error(id, wire::INVALID_PARAMS, &cause.to_string()))
            }
            Err(cause @ ExecError::ModelUnconfigured) => {
                updates.message(&cause.to_string());
                updates.close(stopped(id, "refusal", None))
            }
            Err(cause) => updates.close(wire::error(id, wire::INTERNAL_ERROR, &cause.to_string())),
        }
    }
}

/// The prompt result: the stop reason, and the sealed report when one was made.
fn stopped(id: &Value, reason: &str, report: Option<&Value>) -> Value {
    match report {
        Some(report) => wire::result(
            id,
            json!({ "stopReason": reason, "_meta": { "larry": { "report": report } } }),
        ),
        None => wire::result(id, json!({ "stopReason": reason })),
    }
}

/// Why a prompt's content blocks carry no instruction; its text is the -32602 message.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptFault {
    /// `prompt` is missing or not an array.
    NotAnArray,
    /// `prompt` is an empty array.
    Empty,
    /// A block of a type Larry does not declare (image, audio), named.
    Unsupported(String),
    /// A block lacks its text, or a resource link its name or URI.
    MissingText,
}

impl fmt::Display for PromptFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotAnArray => out.write_str("prompt must be an array of content blocks"),
            Self::Empty => out.write_str("prompt must not be empty"),
            Self::Unsupported(ref kind) => write!(out, "unsupported content block: {kind}"),
            Self::MissingText => out.write_str("a content block is missing its text"),
        }
    }
}

/// The instruction a prompt's content blocks carry: text, an embedded resource's
/// text, and a resource link's name and URI, in order, a blank line between them.
fn instruction(params: Option<&Map>) -> Result<String, PromptFault> {
    let Some(blocks) = params
        .and_then(|map| map.get("prompt"))
        .and_then(Value::as_array)
    else {
        return Err(PromptFault::NotAnArray);
    };
    if blocks.is_empty() {
        return Err(PromptFault::Empty);
    }
    let mut parts: Vec<String> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let part = match block["type"].as_str() {
            Some("text") => block["text"].as_str().map(str::to_owned),
            Some("resource") => block["resource"]["text"].as_str().map(str::to_owned),
            Some("resource_link") => match (block["name"].as_str(), block["uri"].as_str()) {
                (Some(name), Some(uri)) => Some(format!("{name}: {uri}")),
                _ => None,
            },
            Some(other) => return Err(PromptFault::Unsupported(other.to_owned())),
            None => None,
        };
        match part {
            Some(text) => parts.push(text),
            None => return Err(PromptFault::MissingText),
        }
    }
    Ok(parts.join("\n\n"))
}

/// A running prompt's `session/update` stream.
struct Updates<'a, W: Write> {
    /// The protocol stream.
    out: &'a mut W,
    /// The session the updates belong to.
    session: &'a str,
    /// Where the reader thread flags a cancel.
    book: &'a Book,
    /// The first write fault; nothing more is written after it.
    failed: Option<io::Error>,
}

impl<W: Write> Updates<'_, W> {
    /// Sends one `session/update` unless an earlier write failed.
    fn update(&mut self, update: Value) {
        if self.failed.is_some() {
            return;
        }
        let frame = wire::notification(
            "session/update",
            json!({ "sessionId": self.session, "update": update }),
        );
        if let Err(cause) = send(&mut *self.out, &frame) {
            self.failed = Some(cause);
        }
    }

    /// The prompt's reply, or the write fault that ended its stream.
    fn close(self, reply: Value) -> io::Result<Value> {
        match self.failed {
            Some(cause) => Err(cause),
            None => Ok(reply),
        }
    }
}

impl<W: Write> Observer for Updates<'_, W> {
    fn started(&mut self, step: usize, command: &str) {
        let title: String = command
            .lines()
            .next()
            .into_iter()
            .flat_map(str::chars)
            .take(MAX_TITLE_CHARS)
            .collect();
        self.update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": format!("larry-{step}"),
            "title": title,
            "kind": "execute",
            "status": "in_progress",
            "rawInput": { "command": command }
        }));
    }

    fn finished(&mut self, step: usize, observation: &str, passed: bool) {
        let status = if passed { "completed" } else { "failed" };
        self.update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": format!("larry-{step}"),
            "status": status,
            "content": [{ "type": "content", "content": { "type": "text", "text": observation } }]
        }));
    }

    fn message(&mut self, text: &str) {
        self.update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text }
        }));
    }

    fn cancelled(&self) -> bool {
        locked(self.book)
            .get(self.session)
            .is_some_and(|entry| entry.cancelled)
    }
}

#[cfg(test)]
#[path = "sim_acp.rs"]
mod sim;
#[cfg(test)]
mod tests;
