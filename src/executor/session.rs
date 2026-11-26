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
//! Each reply is exactly one JSON object, `{"command": "<sh -c text>"}` or a
//! typed `{"finish": ...}` request; each command runs in its own process group under the
//! executor's timeout, tracked in the ledger so a kill -9 is reaped by the next
//! start; the model sees the exit status and the first 4 KiB of each stream.
//! Auto-compaction (`super::compact`, larry#235) keeps every request inside the
//! model's window, so the length of the conversation never stops a session. The
//! loop stops at the second accepted `finish`, at 256 steps, at three refused replies in a
//! row, at a model refusal, or at a cancellation checked before every model call
//! and every command and polled while one runs: the person's cancel kills the
//! running command's process group and the report counts it interrupted
//! (larry#242). The sealed report records every command as the model saw it,
//! and what compaction did.

use std::path::Path;

use lgwks_macros::Error;
use lgwks_std::json::{self, Value, json};

use super::ExecError;
use super::Ledger;
use super::Report;
use super::Settings;
use super::backoff;
use super::compact::{Answer, Conversation};
use super::digest_hex;
use super::gate::{self, Subject};
use super::identity::fresh_id;
use super::island::{Island, Isolation, Outcome};
use super::prepare;
use super::run::{CommandResult, Place, run_command};
use super::session_gate::{
    SessionIdentity, VerificationFault, checklist, has_fresh_check, invalidate_finish, kept_reply,
    reached_refusal_limit,
};
use super::session_reply::{Reply, parse_reply};
use crate::model::{Completion, ModelClient};

pub(super) use super::session_gate::FinishRequest;

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
/// Bytes of each output stream the model sees and the report keeps.
pub(crate) const OBSERVED_BYTES: usize = 4096;
/// The standing instruction of every session.
const SYSTEM: &str = "You are Larry, a terminal agent. You work in the directory named below by running shell commands. Reply with exactly one JSON object and nothing else: {\"command\": \"<one shell command>\"} runs it with sh -c in that directory and shows you its exit status and output; {\"finish\":{\"summary\":\"<one-line summary>\",\"check\":\"<the exact passing check command>\",\"artifacts\":[\"<relative artifact path>\"],\"state\":[\"<state left behind>\"],\"servers\":[\"<server that must stay up>\"]}} requests the verification checklist. A first accepted finish returns the checklist; repeat the same finish to end the task. Run one command per reply.";

/// Why a session was refused before anything ran.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The instruction is empty or blank.
    #[error("the instruction is empty")]
    EmptyInstruction,
    /// The instruction is over 64 KiB.
    #[error("the instruction is {len} bytes, over {}", MAX_INSTRUCTION_BYTES)]
    InstructionTooLarge {
        /// Its length in bytes.
        len: usize,
    },
    /// The instruction holds a NUL byte.
    #[error("the instruction holds a NUL byte")]
    NulInInstruction,
    /// The working directory is not a directory Larry can work in.
    #[error("the working directory is not a directory")]
    NotADirectory,
}

/// Why a session ended; the names are the Agent Client Protocol's stop reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Stop {
    /// The typed verification gate accepted the second finish.
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
        // The retry budget's timeline, built before the copy so a host whose clock refuses
        // copies nothing (larry#436).
        let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
        // Opened inside the gate: a denied session copies nothing.
        let island = match isolation {
            Isolation::Host => None,
            Isolation::Workspace(platform) => {
                Some(Island::open(platform, settings, &task_id, cwd)?)
            }
        };
        let client = ModelClient::new(config, clock, backoff);
        let mut turn = Turn {
            client: &client,
            settings,
            ledger: &ledger,
            task_id: &task_id,
            place: match island {
                Some(ref isle) => isle.place(),
                None => Place::Open(cwd),
            },
            artifact_root: match island {
                Some(ref isle) => isle.path(),
                None => cwd,
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
        let done_stop = done.stop.clone();
        let mut outcome = Outcome::None;
        let mut diff = None;
        if let Some(ref isle) = island
            && done.stop == Stop::EndTurn
        {
            diff = isle.diff()?;
            outcome = isle.hand_off(cwd, diff.as_ref(), &mut *observer);
        }
        let body = super::session_report::body(
            &SessionIdentity {
                task_id: &task_id,
                task_digest: &task_digest,
            },
            done,
            &route.seal(),
            isolation,
            island.as_ref(),
            outcome,
            diff,
        );
        stop = Some(done_stop);
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

/// The two stable identities a session report carries.
/// Builds the sealed session body after the typed finish gate has decided the stop.
/// What one session's loop did.
pub(super) struct Steps {
    /// Why it stopped.
    pub(super) stop: Stop,
    /// Each command's record, in order.
    pub(super) records: Vec<Value>,
    /// What compaction did.
    pub(super) compaction: Value,
    /// Commands the person declined; none of them ran.
    pub(super) declined: usize,
    /// Commands the person interrupted; each had its process group killed (larry#242).
    pub(super) interrupted: usize,
    /// The check receipt and checklist metadata that authorized completion.
    pub(super) verification: Option<Value>,
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
    /// The real tree used to resolve the typed finish's artifact paths.
    artifact_root: &'a Path,
    /// Who hears the session's progress.
    observer: &'a mut dyn Observer,
}

/// One command's observed result and the facts the verification gate needs.
struct CommandStep {
    /// The bounded answer appended to the model conversation.
    answer: Answer,
    /// The full result sealed in the report.
    record: Value,
    /// Whether the command passed without timeout or interruption.
    passed: bool,
    /// Whether cancellation interrupted its process group.
    interrupted: bool,
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
        let mut last_command: Option<String> = None;
        let mut last_passed: Option<bool> = None;
        let mut first_finish: Option<FinishRequest> = None;
        let mut verification = None;
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
            let kept = kept_reply(reply, MAX_REPLY_BYTES);
            match parse_reply(reply) {
                Ok(Reply::Finish(request)) => {
                    match self.finish_reply(
                        &request,
                        last_command.as_deref(),
                        last_passed == Some(true),
                        &mut first_finish,
                    ) {
                        Ok(Some(receipt)) => {
                            verification = Some(receipt);
                            break Stop::EndTurn;
                        }
                        Ok(None) => {
                            self.show_checklist(&mut conversation, kept, &request, &mut refused)
                        }
                        Err(reason) => {
                            refused = refused.saturating_add(1);
                            if reached_refusal_limit(refused) {
                                break self.refuse(format!("finish: {reason}"));
                            }
                            conversation.push(
                                kept,
                                Answer::Refused {
                                    fault: format!("finish: {reason}"),
                                },
                            );
                        }
                    }
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
                    let result = self.command_step(step, &command)?;
                    invalidate_finish(&mut first_finish);
                    last_command = Some(command.clone());
                    conversation.push(kept, result.answer);
                    steps.push(result.record);
                    last_passed = Some(result.passed);
                    if result.interrupted {
                        interrupted = interrupted.saturating_add(1);
                        break Stop::Cancelled;
                    }
                }
                Err(fault) => {
                    refused = refused.saturating_add(1);
                    if reached_refusal_limit(refused) {
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
            verification,
        })
    }

    /// Checks the last fresh-shell result and every named artifact before completion.
    fn verify_finish(
        &self,
        request: &FinishRequest,
        last_command: Option<&str>,
        last_passed: bool,
    ) -> Result<Value, VerificationFault> {
        if !has_fresh_check(last_command, last_passed, &request.check) {
            return Err(VerificationFault::NoFreshCheck);
        }
        let mut artifacts = Vec::with_capacity(request.artifacts.len());
        for path in &request.artifacts {
            if !super::author::is_writable_path(path) {
                return Err(VerificationFault::BadArtifactPath { path: path.clone() });
            }
            let target = self.artifact_root.join(path);
            let metadata = std::fs::symlink_metadata(&target).map_err(|source| {
                VerificationFault::MissingArtifact {
                    path: path.clone(),
                    source,
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err(VerificationFault::SymlinkArtifact { path: path.clone() });
            }
            artifacts.push(path.clone());
        }
        Ok(verification_receipt(
            &request.check,
            &artifacts,
            &request.state,
            &request.servers,
        ))
    }
}

/// The receipt is explicit about the shell boundary that produced the check.
fn verification_receipt(
    check: &str,
    artifacts: &[String],
    state: &[String],
    servers: &[String],
) -> Value {
    json!({
        "check": check,
        "fresh_shell": true,
        "artifacts": artifacts,
        "state": state,
        "servers": servers,
    })
}

/// Whether one finish repeats the exact request that produced the checklist.
fn same_finish(first_finish: Option<&FinishRequest>, request: &FinishRequest) -> bool {
    first_finish == Some(request)
}

/// A command passes only with exit zero and no timeout or interruption.
fn command_passed(exit_code: Option<i32>, timed_out: bool, interrupted: bool) -> bool {
    exit_code == Some(0) && !timed_out && !interrupted
}

impl Turn<'_> {
    /// Processes one typed finish request and returns a receipt only on its second acceptance.
    fn finish_reply(
        &mut self,
        request: &FinishRequest,
        last_command: Option<&str>,
        last_passed: bool,
        first_finish: &mut Option<FinishRequest>,
    ) -> Result<Option<Value>, VerificationFault> {
        let receipt = self.verify_finish(request, last_command, last_passed)?;
        if same_finish(first_finish.as_ref(), request) {
            self.observer.message(&request.summary);
            return Ok(Some(receipt));
        }
        *first_finish = Some(request.clone());
        self.observer.message(&checklist(request));
        Ok(None)
    }

    /// Sends the checklist and clears the consecutive refusal count.
    fn show_checklist(
        &self,
        conversation: &mut Conversation,
        kept: String,
        request: &FinishRequest,
        refused: &mut usize,
    ) {
        conversation.push(
            kept,
            Answer::Refused {
                fault: checklist(request),
            },
        );
        *refused = 0;
    }

    /// Runs one model command in its fresh process group and records its receipt.
    fn command_step(&mut self, step: usize, command: &str) -> Result<CommandStep, ExecError> {
        self.observer.started(step, command);
        let argv = [String::from("sh"), String::from("-c"), command.to_owned()];
        // The person's cancel is polled while the command runs, so Esc ends it and its whole
        // group, not only the next step (larry#242).
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
        let interrupted = result.interrupted();
        let passed = command_passed(result.exit_code(), result.timed_out(), interrupted);
        let answer = Answer::Observed {
            step,
            command: command.to_owned(),
            status: status(&result),
            output: Some(streams(&result)),
        };
        self.observer.finished(step, &answer.text(), passed);
        Ok(CommandStep {
            answer,
            record: result.into_json(),
            passed,
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

/// Keeps a bounded model reply in the compacted conversation.
#[cfg(test)]
mod verification_tests {
    use super::super::session_reply::{ReplyFault, parse_reply};
    use super::{
        FinishRequest, command_passed, has_fresh_check, same_finish, verification_receipt,
    };

    #[test]
    fn finish_refuses_without_fresh_passing_check() {
        assert_eq!(
            parse_reply(r#"{"done":"the task is complete"}"#),
            Err(ReplyFault::Keys),
            "the legacy model-written done token must not bypass verification"
        );
        assert!(
            !has_fresh_check(Some("cargo test"), false, "cargo test"),
            "a failed check has no receipt"
        );
        assert!(
            !has_fresh_check(None, true, "cargo test"),
            "a finish without a receipt is refused"
        );
        assert!(
            !has_fresh_check(Some("cargo fmt"), true, "cargo test"),
            "a different passing command is not the named check"
        );
    }

    fn finish_request(summary: &str) -> FinishRequest {
        FinishRequest {
            summary: summary.to_owned(),
            check: String::from("true"),
            artifacts: vec![String::from("out")],
            state: vec![String::from("clean")],
            servers: vec![String::from("none")],
        }
    }

    #[test]
    fn repeated_finish_requires_the_identical_request() {
        let first = finish_request("ready");
        assert!(same_finish(Some(&first), &first));
        assert!(!same_finish(None, &first));
        assert!(!same_finish(Some(&finish_request("different")), &first));
    }

    #[test]
    fn receipt_marks_the_check_as_fresh_shell_evidence() {
        let receipt = verification_receipt(
            "true",
            &[String::from("out")],
            &[String::from("clean")],
            &[String::from("none")],
        );
        assert_eq!(
            receipt
                .get("fresh_shell")
                .and_then(lgwks_std::json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn command_passes_only_for_a_clean_zero_exit() {
        assert!(command_passed(Some(0), false, false));
        assert!(!command_passed(Some(1), false, false));
        assert!(!command_passed(Some(0), true, false));
        assert!(!command_passed(Some(0), false, true));
    }
}

#[cfg(test)]
#[path = "sim_session.rs"]
pub(super) mod sim;
