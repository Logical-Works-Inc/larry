//! A terminal session (larry#93): the model works in one directory by choosing
//! shell commands, one per reply, until it says it is done. This is what a
//! Terminal-Bench task asks of an agent, and what `larry acp` runs for Harbor.
//!
//! The session is one displayed `WorkProposal` through the executor's admission
//! gate, like every task. Run on the host (`larry acp`) it needs every executor
//! scope, `model.command` among them, which no default policy holds, so it runs
//! only where an operator has granted it, as for a Harbor task container that
//! isolates the commands. Run isolated (the terminal frontend, larry#243) every
//! command runs in a confined copy of the folder (`super::island`), the model
//! is told its directory is `.`, and the folder changes only when the person
//! approves the diff the session ends with; the default policy suffices.
//! Each reply is exactly one JSON object, `{"command": "<sh -c text>"}` or
//! `{"done": "<summary>"}`; each command runs in its own process group under the
//! executor's timeout, tracked in the ledger so a kill -9 is reaped by the next
//! start; the model sees the exit status and the first 4 KiB of each stream.
//! Auto-compaction (`super::compact`, larry#235) keeps every request inside the
//! model's window, so the length of the conversation never stops a session. The
//! loop stops at the model's `done`, at 256 steps, at three refused replies in a
//! row, at a model refusal, or at a cancellation checked before every model call
//! and every command and polled while one runs: the person's cancel kills the
//! running command's process group and the report counts it interrupted
//! (larry#242). The sealed report records every command as the model saw it,
//! and what compaction did.

use std::fmt;
use std::path::Path;

use lgwks_std::json::{self, Value, json};

use super::ExecError;
use super::Ledger;
use super::Report;
use super::Settings;
use super::author::unfenced;
use super::backoff;
use super::compact::{Answer, Conversation};
use super::digest_hex;
use super::gate::{self, Subject};
use super::identity::fresh_id;
use super::island::{Island, Isolation, Outcome};
use super::prepare;
use super::run::{CommandResult, Place, run_command};
use super::workspace::report_members;
use crate::model::{Completion, ModelClient};

/// Most commands one session runs before it stops with `MaxTurnRequests`. The
/// conversation is bounded by compaction, not by this; it bounds the report, whose
/// records are at most 256 times 12 KiB.
pub(crate) const MAX_STEPS: usize = 256;
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

/// The person's answer to one command, before it runs (larry#239).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Approval {
    /// Run it.
    Run,
    /// Do not run it; the model is told and may choose another.
    Decline,
}

/// What the model is shown for a command the person declined.
const DECLINED: &str = "not run: the person declined this command";

/// The person's answer to the change a session made in its copy of their folder (larry#243).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handoff {
    /// Apply exactly the diff shown to the folder.
    Apply,
    /// Leave the folder as it is.
    Discard,
}

/// What a session reports while it runs, and how it learns it was cancelled.
pub(crate) trait Observer {
    /// The session ended with `summary` as the change its commands made in the copy of
    /// the person's folder; the folder changes only when this answers [`Handoff::Apply`].
    /// The answer for an observer nobody can ask is to discard.
    fn review(&mut self, _summary: &[String]) -> Handoff {
        Handoff::Discard
    }
    /// The change in full, as a unified diff of at most 1 MiB, offered once just before
    /// [`Observer::review`] so a frontend can show it while the person decides (larry#244).
    fn diff(&mut self, _text: &str) {}
    /// What became of that change: `line` says so, and `applied` whether it is in the folder.
    fn handed(&mut self, line: &str, applied: bool);
    /// The model asks to run `command` as step `step`; nothing runs unless this
    /// answers [`Approval::Run`]. A frontend asks the person; `larry acp` runs
    /// inside the operator's isolated container under the same gate and runs it.
    fn approve(&mut self, step: usize, command: &str) -> Approval;
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

/// The status line the model is shown for one clipped command result.
fn status(result: &CommandResult) -> String {
    let ended = match (result.timed_out(), result.interrupted(), result.exit_code()) {
        (false, false, Some(code)) => format!("exit status {code}"),
        (false, false, None) => String::from("ended by a signal"),
        (true, false, _) => String::from("timed out; its process group was killed"),
        (_, true, _) => String::from("interrupted by the person; its process group was killed"),
    };
    if result.truncated() {
        format!("{ended} (output past {OBSERVED_BYTES} bytes of a stream was cut)")
    } else {
        ended
    }
}

/// The streams the model is shown for one clipped command result, under its status.
fn streams(result: &CommandResult) -> String {
    format!("stdout:\n{}\nstderr:\n{}", result.stdout(), result.stderr())
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
    let Ok(meta) = std::fs::metadata(cwd) else {
        return Err(SessionError::NotADirectory);
    };
    if meta.is_dir() {
        Ok(())
    } else {
        Err(SessionError::NotADirectory)
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
    let first = report
        .json()
        .get("denied")
        .and_then(lgwks_std::json::Value::as_array)
        .and_then(|lines| lines.first())
        .and_then(lgwks_std::json::Value::as_str);
    match first {
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
    isolation: Isolation,
    observer: &mut dyn Observer,
) -> Result<Ended, ExecError> {
    check(instruction, cwd).map_err(ExecError::Session)?;
    // A host that cannot confine refuses here, before the ledger, the model or a copy.
    isolation.check()?;
    let (ledger, key) = prepare(settings)?;
    let route = settings.route(&ledger, None)?;
    let config = settings.model_config(&route)?;
    let document = json::to_vec(&json!({
        "session": {
            "cwd": cwd.to_string_lossy(),
            "instruction": instruction,
            "isolation": isolation.name(),
        }
    }))
    .map_err(|cause| ExecError::Gate(cause.into()))?;
    let task_id = fresh_id()?;
    let task_digest = digest_hex(&document);
    let mut stop = None;
    let subject = Subject {
        required: isolation.required(),
        path: cwd,
        bytes: &document,
    };
    let report = gate::admit_and_run(subject, settings, &task_id, &task_digest, &key, || {
        // Opened inside the gate: a denied session copies nothing.
        let island = match isolation {
            Isolation::Host => None,
            Isolation::Workspace(platform) => {
                Some(Island::open(platform, settings, &task_id, cwd)?)
            }
        };
        let client = ModelClient::new(config, backoff);
        let mut turn = Turn {
            client: &client,
            settings,
            ledger: &ledger,
            task_id: &task_id,
            place: match island {
                Some(ref isle) => isle.place(),
                None => Place::Open(cwd),
            },
            // The model is told its directory is `.` in a copy, so the request carries
            // no run id and no path of the person's.
            shown: match island {
                Some(_) => String::from("."),
                None => cwd.display().to_string(),
            },
            observer: &mut *observer,
        };
        let done = turn.run(instruction)?;
        let mut outcome = Outcome::None;
        let mut diff = None;
        if let Some(ref isle) = island
            && done.stop == Stop::EndTurn
        {
            diff = isle.diff()?;
            outcome = isle.hand_off(cwd, diff.as_ref(), &mut *observer);
        }
        let mut body = json!({
            "task_id": task_id,
            "task_digest": task_digest,
            "diff": null,
            "results": done.records,
            "denied": [],
            "session": {
                "stop": done.stop.name(),
                "commands": done.records.len(),
                "declined": done.declined,
                "interrupted": done.interrupted,
                "route": route.seal(),
                "compaction": done.compaction,
                "isolation": match island {
                    Some(ref isle) => isle.describe(),
                    None => json!({ "mode": isolation.name() }),
                },
                "handoff": outcome.name(),
            },
        });
        // An isolated session records the change it made, whole or cut (larry#206).
        if island.is_some()
            && let Some(object) = body.as_object_mut()
        {
            for (name, member) in report_members(diff) {
                object.insert(name.to_owned(), member);
            }
        }
        stop = Some(done.stop);
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

/// What one session's loop did.
struct Steps {
    /// Why it stopped.
    stop: Stop,
    /// Each command's record, in order.
    records: Vec<Value>,
    /// What compaction did.
    compaction: Value,
    /// Commands the person declined; none of them ran.
    declined: usize,
    /// Commands the person interrupted; each had its process group killed (larry#242).
    interrupted: usize,
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
    /// Where every command runs: the directory itself, or its confined copy.
    place: Place<'a>,
    /// The directory the model is told it works in.
    shown: String,
    /// Who hears the session's progress.
    observer: &'a mut dyn Observer,
}

impl Turn<'_> {
    /// Asks, runs and observes until a stop, and returns it with each command's
    /// record, what compaction did and how many commands the person declined
    /// or interrupted.
    fn run(&mut self, instruction: &str) -> Result<Steps, ExecError> {
        let task = format!("Working directory: {}\n\nTask:\n{instruction}", self.shown);
        let mut conversation = Conversation::new(SYSTEM, task, self.client.request_bound());
        let mut steps: Vec<Value> = Vec::new();
        let mut refused = 0_usize;
        let mut declined = 0_usize;
        let mut interrupted = 0_usize;
        let stop = loop {
            if steps.len().saturating_add(declined) >= MAX_STEPS {
                break Stop::MaxTurnRequests;
            }
            let completion = match self.ask(&mut conversation) {
                Ok(completion) => completion,
                Err(stop) => break stop,
            };
            if self.observer.cancelled() {
                break Stop::Cancelled;
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
            match parse_reply(reply) {
                Ok(Reply::Done(summary)) => {
                    self.observer.message(&summary);
                    break Stop::EndTurn;
                }
                Ok(Reply::Command(command)) => {
                    refused = 0;
                    let step = steps.len();
                    if self.observer.approve(step, &command) == Approval::Decline {
                        declined = declined.saturating_add(1);
                        let answer = Answer::Observed {
                            step,
                            command,
                            status: String::from(DECLINED),
                            output: Some(String::new()),
                        };
                        conversation.push(kept, answer);
                        continue;
                    }
                    if self.observer.cancelled() {
                        break Stop::Cancelled;
                    }
                    self.observer.started(step, &command);
                    let argv = [String::from("sh"), String::from("-c"), command.clone()];
                    // The person's cancel is polled while the command runs, so Esc
                    // ends it and its whole group, not only the next step (larry#242).
                    let result = run_command(
                        &argv,
                        self.place,
                        &self.settings.path,
                        self.settings.timeout,
                        self.ledger,
                        self.task_id,
                        &|| self.observer.cancelled(),
                    )?
                    .clipped(OBSERVED_BYTES);
                    let answer = Answer::Observed {
                        step,
                        command,
                        status: status(&result),
                        output: Some(streams(&result)),
                    };
                    let was_interrupted = result.interrupted();
                    let passed =
                        result.exit_code() == Some(0) && !result.timed_out() && !was_interrupted;
                    self.observer.finished(step, &answer.text(), passed);
                    conversation.push(kept, answer);
                    steps.push(result.into_json());
                    if was_interrupted {
                        interrupted = interrupted.saturating_add(1);
                        break Stop::Cancelled;
                    }
                }
                Err(fault) => {
                    refused = refused.saturating_add(1);
                    if refused >= MAX_REFUSED {
                        break self.refuse(format!(
                            "model: {MAX_REFUSED} replies in a row refused: {fault}"
                        ));
                    }
                    let fault = fault.to_string();
                    conversation.push(kept, Answer::Refused { fault });
                }
            }
        };
        Ok(Steps {
            stop,
            records: steps,
            compaction: conversation.json(),
            declined,
            interrupted,
        })
    }

    /// One completion over `conversation`, compacted first when it needs it and
    /// again each time the gateway answers that the request is past the model's
    /// window; the stop, said to the person, when there is none.
    fn ask(&mut self, conversation: &mut Conversation) -> Result<Completion, Stop> {
        loop {
            if self.observer.cancelled() {
                return Err(Stop::Cancelled);
            }
            let request = conversation
                .prepare(self.client)
                .map_err(|overflow| self.refuse(format!("model: {overflow}")))?;
            match self.client.send(&request) {
                Ok(completion) => return Ok(completion),
                Err(cause) if cause.is_context_overflow() => conversation
                    .shrink(request.bytes(), cause.window())
                    .map_err(|overflow| self.refuse(format!("model: {cause}; {overflow}")))?,
                Err(cause) => return Err(self.refuse(format!("model: {cause}"))),
            }
        }
    }

    /// Says `line` to the person and returns it as the session's refusal.
    fn refuse(&mut self, line: String) -> Stop {
        self.observer.message(&line);
        Stop::Refusal(line)
    }
}

#[cfg(test)]
#[path = "sim_session.rs"]
pub(super) mod sim;
