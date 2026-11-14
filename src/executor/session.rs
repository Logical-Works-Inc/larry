//! A terminal session (larry#93): the model works in one directory by choosing
//! shell commands, one per reply, until it says it is done. This is what a
//! Terminal-Bench task asks of an agent, and what `larry acp` runs for Harbor.
//!
//! The session is one displayed `WorkProposal` through the executor's admission
//! gate, like every task: it needs every executor scope, `model.command` among
//! them, which no default policy holds, so a session runs only where an operator
//! has granted it, as for a Harbor task container that isolates the commands.
//! Each reply is exactly one JSON object, `{"command": "<sh -c text>"}` or
//! `{"done": "<summary>"}`; each command runs in its own process group under the
//! executor's timeout, tracked in the ledger so a kill -9 is reaped by the next
//! start; the model sees the exit status and the first 8 KiB of each stream. The
//! loop stops at the model's `done`, at 48 steps, at three refused replies in a
//! row, at a model refusal, or at a cancellation checked before every model call
//! and every command. The sealed report records every command as the model saw it.

use std::fmt;
use std::path::Path;

use lgwks_std::json::{self, Value, json};

use super::ExecError;
use super::Ledger;
use super::Report;
use super::Settings;
use super::author::unfenced;
use super::backoff;
use super::digest_hex;
use super::gate::{self, Subject};
use super::identity::fresh_id;
use super::prepare;
use super::run::{CommandResult, run_command};
use crate::model::{Completion, Message, ModelClient, Role};

/// Most commands one session runs before it stops with `MaxTurnRequests`.
pub(crate) const MAX_STEPS: usize = 48;
/// Most replies in a row the session refuses before it stops.
const MAX_REFUSED: usize = 3;
/// Largest instruction a session takes, in bytes.
pub(crate) const MAX_INSTRUCTION_BYTES: usize = 65_536;
/// Largest model reply the session parses or keeps in the conversation, in bytes.
const MAX_REPLY_BYTES: usize = 8192;
/// Largest command or summary one reply may carry, in bytes.
const MAX_COMMAND_BYTES: usize = 4096;
/// Bytes of each output stream the model sees and the report keeps.
pub(crate) const OBSERVED_BYTES: usize = 4096;
/// The standing instruction of every session.
const SYSTEM: &str = "You are Larry, a terminal agent. You work in the directory named below by running shell commands. Reply with exactly one JSON object and nothing else: {\"command\": \"<one shell command>\"} runs it with sh -c in that directory and shows you its exit status and output; {\"done\": \"<one-line summary>\"} ends the task. Run one command per reply.";

/// The request bound the model client refuses past, checked against the most a
/// session can send in raw text: the instruction, then per step a reply and an
/// observation. Output heavy in control bytes grows when JSON-escaped; past the
/// bound the client refuses with `BodyTooLarge`, and the session stops as a refusal.
const _: () = assert!(
    MAX_INSTRUCTION_BYTES + MAX_STEPS * (MAX_REPLY_BYTES + 2 * OBSERVED_BYTES + 512) + 4096
        <= 1_048_576,
    "a session's longest conversation fits the model client's 1 MiB request bound"
);

/// Why a session was refused before anything ran.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionError {
    /// The instruction is empty or blank.
    EmptyInstruction,
    /// The instruction is over 64 KiB.
    InstructionTooLarge {
        /// Its length in bytes.
        len: usize,
    },
    /// The instruction holds a NUL byte.
    NulInInstruction,
    /// The working directory is not a directory Larry can work in.
    NotADirectory,
}

impl fmt::Display for SessionError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyInstruction => out.write_str("the instruction is empty"),
            Self::InstructionTooLarge { len } => write!(
                out,
                "the instruction is {len} bytes, over {MAX_INSTRUCTION_BYTES}"
            ),
            Self::NulInInstruction => out.write_str("the instruction holds a NUL byte"),
            Self::NotADirectory => out.write_str("the working directory is not a directory"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Why a session ended; the names are the Agent Client Protocol's stop reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Stop {
    /// The model said it was done.
    EndTurn,
    /// The session ran [`MAX_STEPS`] commands.
    MaxTurnRequests,
    /// The gate, the model gateway or the replies refused; the reason says which.
    Refusal(String),
    /// The client cancelled the session.
    Cancelled,
}

impl Stop {
    /// The stop reason's wire name.
    pub(crate) const fn name(&self) -> &'static str {
        match *self {
            Self::EndTurn => "end_turn",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal(_) => "refusal",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What a session reports while it runs, and how it learns it was cancelled.
pub(crate) trait Observer {
    /// Step `step` is about to run `command`.
    fn started(&mut self, step: usize, command: &str);
    /// Step `step` ended; `observation` is what the model is shown, `passed` its exit 0.
    fn finished(&mut self, step: usize, observation: &str, passed: bool);
    /// A message for the person: the model's summary or the reason the session stopped.
    fn message(&mut self, text: &str);
    /// Whether the client has cancelled the session.
    fn cancelled(&self) -> bool;
}

/// One reply, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Run this command with `sh -c`.
    Command(String),
    /// The task is done; the summary for the person.
    Done(String),
}

/// Why a reply was refused; the model is told and may try again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyFault {
    /// The reply is over [`MAX_REPLY_BYTES`].
    TooLong,
    /// The reply is not JSON; the parser's reason, so the model sees where it broke.
    NotJson(String),
    /// The reply is JSON but not an object.
    NotAnObject,
    /// The object does not hold exactly one key, `command` or `done`.
    Keys,
    /// The key's value is not a string.
    NotAString,
    /// The command is empty or blank.
    EmptyCommand,
    /// The command or summary is over 4 KiB.
    FieldTooLong,
    /// The command holds a NUL byte, which no shell argument can carry.
    Nul,
}

impl fmt::Display for ReplyFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooLong => write!(out, "the reply is over {MAX_REPLY_BYTES} bytes"),
            Self::NotJson(ref cause) => write!(out, "the reply is not JSON ({cause})"),
            Self::NotAnObject => out.write_str("the reply is not a JSON object"),
            Self::Keys => out.write_str("the object must hold exactly one key, command or done"),
            Self::NotAString => out.write_str("the value must be a string"),
            Self::EmptyCommand => out.write_str("the command is empty"),
            Self::FieldTooLong => write!(
                out,
                "the command or summary is over {MAX_COMMAND_BYTES} bytes"
            ),
            Self::Nul => out.write_str("the command holds a NUL byte"),
        }
    }
}

/// Parses one model reply: one surrounding code fence and outer whitespace are allowed.
pub(crate) fn parse_reply(text: &str) -> Result<Reply, ReplyFault> {
    if text.len() > MAX_REPLY_BYTES {
        return Err(ReplyFault::TooLong);
    }
    let value: Value = json::from_str(unfenced(text).trim())
        .map_err(|cause| ReplyFault::NotJson(cause.to_string()))?;
    let object = value.as_object().ok_or(ReplyFault::NotAnObject)?;
    let mut entries = object.iter();
    let (Some((key, field)), None) = (entries.next(), entries.next()) else {
        return Err(ReplyFault::Keys);
    };
    let text = field.as_str().ok_or(ReplyFault::NotAString)?;
    if text.len() > MAX_COMMAND_BYTES {
        return Err(ReplyFault::FieldTooLong);
    }
    let name: &str = key;
    match name {
        "command" if text.trim().is_empty() => Err(ReplyFault::EmptyCommand),
        "command" if text.contains('\0') => Err(ReplyFault::Nul),
        "command" => Ok(Reply::Command(text.to_owned())),
        "done" => Ok(Reply::Done(text.to_owned())),
        _ => Err(ReplyFault::Keys),
    }
}

/// The text the model is shown for one clipped command result.
pub(crate) fn observation(result: &CommandResult) -> String {
    let status = match (result.timed_out(), result.exit_code()) {
        (true, _) => String::from("timed out; its process group was killed"),
        (false, Some(code)) => format!("exit status {code}"),
        (false, None) => String::from("ended by a signal"),
    };
    let cut = if result.truncated() {
        format!(" (output past {OBSERVED_BYTES} bytes of a stream was cut)")
    } else {
        String::new()
    };
    format!(
        "{status}{cut}\nstdout:\n{}\nstderr:\n{}",
        result.stdout(),
        result.stderr()
    )
}

/// Checks a session's instruction and directory before anything runs.
pub(crate) fn check(instruction: &str, cwd: &Path) -> Result<(), SessionError> {
    if instruction.trim().is_empty() {
        return Err(SessionError::EmptyInstruction);
    }
    if instruction.len() > MAX_INSTRUCTION_BYTES {
        return Err(SessionError::InstructionTooLarge {
            len: instruction.len(),
        });
    }
    if instruction.contains('\0') {
        return Err(SessionError::NulInInstruction);
    }
    match std::fs::metadata(cwd) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) | Err(_) => Err(SessionError::NotADirectory),
    }
}

/// How a session ended, with its sealed report.
#[derive(Debug)]
pub(crate) struct Ended {
    /// Why it ended.
    pub(crate) stop: Stop,
    /// The report sealed under the state directory's key.
    pub(crate) report: Report,
}

/// The one denial line of a report the gate sealed without running anything.
fn denial(report: &Report) -> String {
    match report.json()["denied"][0].as_str() {
        Some(line) => line.to_owned(),
        None => String::from("gate: the session was denied"),
    }
}

/// Runs a terminal session on `instruction` in `cwd`, reporting through `observer`.
/// A refusal by the gate seals a report that ran nothing; a fault outside the
/// session's doing (the ledger, a process group) is a typed error.
pub(crate) fn run_session(
    instruction: &str,
    cwd: &Path,
    settings: &Settings,
    observer: &mut dyn Observer,
) -> Result<Ended, ExecError> {
    check(instruction, cwd).map_err(ExecError::Session)?;
    let config = settings.model.clone().ok_or(ExecError::ModelUnconfigured)?;
    let (ledger, key) = prepare(settings)?;
    let document = json::to_vec(&json!({
        "session": { "cwd": cwd.to_string_lossy(), "instruction": instruction }
    }))
    .map_err(|cause| ExecError::Gate(cause.into()))?;
    let task_id = fresh_id()?;
    let task_digest = digest_hex(&document);
    let mut stop = None;
    let subject = Subject {
        required: gate::session_scopes(),
        path: cwd,
        bytes: &document,
    };
    let report = gate::admit_and_run(subject, settings, &task_id, &task_digest, &key, || {
        let client = ModelClient::new(config, backoff);
        let mut turn = Turn {
            client: &client,
            settings,
            ledger: &ledger,
            task_id: &task_id,
            cwd,
            observer: &mut *observer,
        };
        let (ended, steps) = turn.run(instruction)?;
        let body = json!({
            "task_id": task_id,
            "task_digest": task_digest,
            "diff": null,
            "results": steps,
            "denied": [],
            "session": { "stop": ended.name(), "commands": steps.len() },
        });
        stop = Some(ended);
        Ok(Report::sealed(body, &key))
    })?;
    let stop = match stop {
        Some(stop) => stop,
        None => {
            let line = denial(&report);
            observer.message(&line);
            Stop::Refusal(line)
        }
    };
    Ok(Ended { stop, report })
}

/// One session's loop state.
struct Turn<'a> {
    /// The model gateway.
    client: &'a ModelClient,
    /// The executor's settings: the PATH and the command timeout.
    settings: &'a Settings,
    /// The ledger every command group is recorded in.
    ledger: &'a Ledger,
    /// The run's identity.
    task_id: &'a str,
    /// The directory every command runs in.
    cwd: &'a Path,
    /// Who hears the session's progress.
    observer: &'a mut dyn Observer,
}

impl Turn<'_> {
    /// Asks, runs and observes until a stop, and returns it with each command's record.
    fn run(&mut self, instruction: &str) -> Result<(Stop, Vec<Value>), ExecError> {
        let mut messages = vec![
            Message::new(Role::System, SYSTEM),
            Message::new(
                Role::User,
                format!(
                    "Working directory: {}\n\nTask:\n{instruction}",
                    self.cwd.display()
                ),
            ),
        ];
        let mut steps: Vec<Value> = Vec::new();
        let mut refused = 0_usize;
        while steps.len() < MAX_STEPS {
            if self.observer.cancelled() {
                return Ok((Stop::Cancelled, steps));
            }
            let completion: Completion = match self.client.complete(&messages) {
                Ok(completion) => completion,
                Err(cause) => {
                    let line = format!("model: {cause}");
                    self.observer.message(&line);
                    return Ok((Stop::Refusal(line), steps));
                }
            };
            if self.observer.cancelled() {
                return Ok((Stop::Cancelled, steps));
            }
            let reply = completion.content();
            // A reply past the bound is refused below; only its length enters the
            // conversation, so it cannot grow the next request.
            let kept = if reply.len() > MAX_REPLY_BYTES {
                format!(
                    "(a reply of {} bytes, over the bound, was dropped)",
                    reply.len()
                )
            } else {
                reply.to_owned()
            };
            messages.push(Message::new(Role::Assistant, kept));
            match parse_reply(reply) {
                Ok(Reply::Done(summary)) => {
                    self.observer.message(&summary);
                    return Ok((Stop::EndTurn, steps));
                }
                Ok(Reply::Command(command)) => {
                    refused = 0;
                    let step = steps.len();
                    self.observer.started(step, &command);
                    let argv = [String::from("sh"), String::from("-c"), command];
                    let result = run_command(
                        &argv,
                        self.cwd,
                        &self.settings.path,
                        self.settings.timeout,
                        self.ledger,
                        self.task_id,
                    )?
                    .clipped(OBSERVED_BYTES);
                    let text = observation(&result);
                    let passed = result.exit_code() == Some(0) && !result.timed_out();
                    self.observer.finished(step, &text, passed);
                    messages.push(Message::new(Role::User, text));
                    steps.push(result.into_json());
                }
                Err(fault) => {
                    refused = refused.saturating_add(1);
                    if refused >= MAX_REFUSED {
                        let line =
                            format!("model: {MAX_REFUSED} replies in a row refused: {fault}");
                        self.observer.message(&line);
                        return Ok((Stop::Refusal(line), steps));
                    }
                    messages.push(Message::new(
                        Role::User,
                        format!("Your reply was refused: {fault}. Reply with exactly one JSON object: {{\"command\": \"...\"}} or {{\"done\": \"...\"}}."),
                    ));
                }
            }
        }
        Ok((Stop::MaxTurnRequests, steps))
    }
}

#[cfg(test)]
#[path = "sim_session.rs"]
mod sim;
