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

use std::io;
use std::path::Path;
use std::sync::Arc;

use lgwks_macros::Error;
use lgwks_std::json::{Value, json};
#[cfg(test)]
use lgwks_std::seeded::Seeded;

use super::ExecError;
use super::Ledger;
use super::Report;
use super::Settings;
use super::compact::{Answer, Conversation};
use super::run::{CommandResult, Place, RunHooks, StreamCallback, run_command};
use super::session_gate::{
    CheckProof, Fingerprint, FingerprintFault, VerificationFault, checklist, fingerprint_tree,
    first_change, has_fresh_check, invalidate_finish, kept_reply, reached_refusal_limit,
};
use super::session_reply::{Reply, parse_reply};
use crate::model::{Completion, ModelClient, Usage};

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
    /// A nonblocking callback for live command output, when the frontend can paint it.
    fn stream_callback(&mut self) -> Option<StreamCallback> {
        None
    }
    /// Reports callback chunks dropped at the bounded event channel.
    fn stream_dropped(&mut self) {}
    /// What became of that change: `line` says so, and `applied` whether it is in the folder.
    /// An observer that records nothing leaves this alone.
    fn handed(&mut self, _line: &str, _applied: bool) {}
    /// The model asks to run `command` as step `step`; nothing runs unless this
    /// answers [`Approval::Run`]. A frontend asks the person; `larry acp` runs
    /// inside the operator's isolated container under the same gate and runs it.
    /// No default: every observer says aloud whether a command may run (larry#239).
    fn approve(&mut self, step: usize, command: &str) -> Approval;
    /// Step `step` is about to run `command`. An observer that records nothing
    /// leaves this alone.
    fn started(&mut self, _step: usize, _command: &str) {}
    /// Step `step` ended; `observation` is what the model is shown, `passed` its exit 0.
    /// An observer that records nothing leaves this alone.
    fn finished(&mut self, _step: usize, _observation: &str, _passed: bool) {}
    /// A message for the person: the model's summary or the reason the session stopped.
    /// An observer with nobody to tell leaves this alone.
    fn message(&mut self, _text: &str) {}
    /// Whether the client has cancelled the session. An observer nobody can stop
    /// leaves this alone, which never stops the session early.
    fn cancelled(&self) -> bool {
        false
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

/// What one session's loop did.
pub(super) struct Steps {
    /// Why it stopped.
    pub(super) stop: Stop,
    /// Each command's record, in order.
    pub(super) records: Vec<Value>,
    /// Usage reported by each model completion, in order.
    pub(super) usage: Vec<Option<Usage>>,
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
pub(super) struct Turn<'a> {
    /// The model gateway.
    pub(super) client: &'a ModelClient,
    /// The executor's settings: the PATH and the command timeout.
    pub(super) settings: &'a Settings,
    /// The ledger every command group is recorded in.
    pub(super) ledger: &'a Ledger,
    /// The run's identity.
    pub(super) task_id: &'a str,
    /// Where every command runs: the directory itself, or its confined copy.
    pub(super) place: Place<'a>,
    /// The directory the model is told it works in.
    pub(super) shown: String,
    /// The real tree used to resolve the typed finish's artifact paths.
    pub(super) artifact_root: &'a Path,
    /// What the last command proved about the session tree: clean, mutated at a
    /// path, or refused to fingerprint. `None` before the first command runs.
    pub(super) last_proof: Option<CheckProof>,
    /// Who hears the session's progress.
    pub(super) observer: &'a mut dyn Observer,
}

/// What one `command` reply asked of the session loop.
#[derive(Debug)]
enum CommandDecision {
    /// The person declined: nothing ran, and the model is told the answer.
    Declined(Answer),
    /// The command ran: its observed step and the proof it left about the tree.
    Ran(CommandStep, CheckProof),
    /// The client cancelled before or while it ran: the loop ends.
    Cancelled,
}

/// The proof one command left about the session tree: its before fingerprint
/// against the after fingerprint taken once it ended.
fn prove_command(before: Result<Fingerprint, FingerprintFault>, root: &Path) -> CheckProof {
    match (before, fingerprint_tree(root)) {
        (Ok(before), Ok(after)) => match first_change(&before, &after) {
            Some(path) => CheckProof::Mutated { path },
            None => CheckProof::Clean,
        },
        (Err(fault), _) | (_, Err(fault)) => CheckProof::Fault(fault),
    }
}

/// One command's observed result and the facts the verification gate needs.
#[derive(Debug)]
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
    pub(super) fn run(&mut self, instruction: &str) -> Result<Steps, ExecError> {
        let task = format!("Working directory: {}\n\nTask:\n{instruction}", self.shown);
        let mut conversation = Conversation::new(SYSTEM, task, self.client.request_bound());
        let mut steps: Vec<Value> = Vec::new();
        let mut usage = Vec::new();
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
            usage.push(completion.usage());
            if self.observer.cancelled() {
                break Stop::Cancelled;
            }
            let reply = completion.content();
            let kept = kept_reply(reply, MAX_REPLY_BYTES);
            match parse_reply(reply) {
                Ok(Reply::Finish(request)) => {
                    let proof = self.last_proof.clone();
                    match self.finish_reply(
                        &request,
                        last_command.as_deref(),
                        last_passed == Some(true),
                        proof.as_ref(),
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
                            if let Some(stop) =
                                self.refuse_finish(&mut conversation, kept, &mut refused, &reason)
                            {
                                break stop;
                            }
                        }
                    }
                }
                Ok(Reply::Command(command)) => {
                    refused = 0;
                    let step = steps.len();
                    match self.run_one_command(step, &command)? {
                        CommandDecision::Declined(answer) => {
                            declined = declined.saturating_add(1);
                            conversation.push(kept, answer);
                        }
                        CommandDecision::Ran(result, proof) => {
                            self.last_proof = Some(proof);
                            invalidate_finish(&mut first_finish);
                            last_command = Some(command.clone());
                            last_passed = Some(result.passed);
                            let halted = result.interrupted;
                            conversation.push(kept, result.answer);
                            steps.push(result.record);
                            if halted {
                                interrupted = interrupted.saturating_add(1);
                                break Stop::Cancelled;
                            }
                        }
                        CommandDecision::Cancelled => break Stop::Cancelled,
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
            usage,
            compaction: conversation.json(),
            declined,
            interrupted,
            verification,
        })
    }

    /// Checks the last fresh-shell result, that the last command changed nothing,
    /// and every named artifact before completion.
    fn verify_finish(
        &self,
        request: &FinishRequest,
        last_command: Option<&str>,
        last_passed: bool,
        last_proof: Option<&CheckProof>,
    ) -> Result<Value, VerificationFault> {
        if !has_fresh_check(last_command, last_passed, &request.check) {
            return Err(VerificationFault::NoFreshCheck);
        }
        match last_proof.cloned() {
            Some(CheckProof::Clean) => {}
            Some(CheckProof::Mutated { path }) => {
                return Err(VerificationFault::CheckMutated { path });
            }
            Some(CheckProof::Fault(FingerprintFault::TooLarge { what, limit })) => {
                return Err(VerificationFault::FingerprintTooLarge { what, limit });
            }
            Some(CheckProof::Fault(FingerprintFault::Unreadable { path, source })) => {
                let source = match Arc::try_unwrap(source) {
                    Ok(source) => source,
                    Err(shared) => io::Error::new(shared.kind(), shared.to_string()),
                };
                return Err(VerificationFault::FingerprintIo { path, source });
            }
            None => return Err(VerificationFault::NoFreshCheck),
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
        last_proof: Option<&CheckProof>,
        first_finish: &mut Option<FinishRequest>,
    ) -> Result<Option<Value>, VerificationFault> {
        let receipt = self.verify_finish(request, last_command, last_passed, last_proof)?;
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

    /// Counts one refused finish toward the budget: the session ends once three
    /// in a row are refused, else the model is told why and may try again.
    fn refuse_finish(
        &mut self,
        conversation: &mut Conversation,
        kept: String,
        refused: &mut usize,
        reason: &VerificationFault,
    ) -> Option<Stop> {
        *refused = refused.saturating_add(1);
        if reached_refusal_limit(*refused) {
            return Some(self.refuse(format!("finish: {reason}")));
        }
        conversation.push(
            kept,
            Answer::Refused {
                fault: format!("finish: {reason}"),
            },
        );
        None
    }

    /// Asks the person about one model command and runs it when approved: the
    /// decision the loop records. A declined command runs nothing; a cancelled
    /// client ends the loop.
    fn run_one_command(
        &mut self,
        step: usize,
        command: &str,
    ) -> Result<CommandDecision, ExecError> {
        if self.observer.approve(step, command) == Approval::Decline {
            let answer = Answer::Observed {
                step,
                command: command.to_owned(),
                status: String::from(DECLINED),
                output: Some(String::new()),
            };
            return Ok(CommandDecision::Declined(answer));
        }
        if self.observer.cancelled() {
            return Ok(CommandDecision::Cancelled);
        }
        let before = fingerprint_tree(self.artifact_root);
        let outcome = self.command_step(step, command)?;
        Ok(CommandDecision::Ran(
            outcome,
            prove_command(before, self.artifact_root),
        ))
    }

    /// Runs one model command in its fresh process group and records its receipt.
    fn command_step(&mut self, step: usize, command: &str) -> Result<CommandStep, ExecError> {
        self.observer.started(step, command);
        let argv = [String::from("sh"), String::from("-c"), command.to_owned()];
        // The person's cancel is polled while the command runs, so Esc ends it and its whole
        // group, not only the next step (larry#242); live output streams while it runs (larry#252).
        let stream = self.observer.stream_callback();
        let result = run_command(
            &argv,
            self.place,
            &self.settings.path,
            self.settings.timeout,
            self.ledger,
            self.task_id,
            RunHooks::new(&|| self.observer.cancelled(), stream.as_ref()),
        );
        self.observer.stream_dropped();
        let result = result?.clipped(OBSERVED_BYTES);
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

/// One lowercase word drawn from the seed; the seeded session simulations share
/// it so every request field stays bounded text.
#[cfg(test)]
pub(super) fn word(rng: &mut Seeded) -> Result<String, crate::sim_error::SimError> {
    let len = rng.index(12)?.saturating_add(1);
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        out.push(char::from(
            b'a'.saturating_add(u8::try_from(rng.index(26)?)?),
        ));
    }
    Ok(out)
}

/// The typed finish request the seeded simulations answer with: the check
/// `true`, one artifact, clean state and no servers. One constructor, so the
/// two verification sims never repeat its vectors (larry#271).
#[cfg(test)]
pub(super) fn sim_finish_request(summary: &str, artifact: &str) -> FinishRequest {
    FinishRequest {
        summary: summary.to_owned(),
        check: String::from("true"),
        artifacts: vec![artifact.to_owned()],
        state: vec![String::from("clean")],
        servers: vec![String::from("none")],
    }
}

/// Keeps a bounded model reply in the compacted conversation.
#[cfg(test)]
#[path = "session_verify.rs"]
mod verification_tests;

/// Seeded turn-loop simulations against a scripted person.
#[cfg(test)]
#[path = "sim_person.rs"]
mod person_sim;

#[cfg(test)]
#[path = "sim_session.rs"]
pub(super) mod sim;
