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
use super::observation::redact_observation;
use super::run::{CommandResult, Place, RunHooks, StreamCallback, run_command};
use super::workspace::report_members;
use crate::model::{Completion, ModelClient, Route, Usage};

/// A terminal session's reply protocol: one JSON object per turn, parsed and
/// refused with an exact reason the model can retry from (larry#545).
#[path = "session_reply.rs"]
pub(crate) mod reply;

use reply::{Reply, parse_reply};

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

/// The absolute working directory as an observation redaction when it is long
/// enough to be selective; a shorter name stays out of the redaction list.
fn cwd_redaction(absolute: String) -> Option<String> {
    if absolute.len() >= 8 {
        Some(absolute)
    } else {
        None
    }
}

/// Keeps a bounded, redacted model reply for the next conversation turn.
fn kept_reply(reply: &str, redactions: &[String]) -> String {
    let kept = if reply.len() > MAX_REPLY_BYTES {
        format!(
            "(a reply of {} bytes, over the bound, was dropped)",
            reply.len()
        )
    } else {
        reply.to_owned()
    };
    redact_observation(&kept, redactions)
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

/// The `authored` usage array of a sealed session report: one entry per model
/// completion, so `larry eval` prices a session task like a brief (larry#217).
fn authored_usage(usage: &[Option<Usage>]) -> Vec<Value> {
    usage
        .iter()
        .map(|spent| {
            json!({
                "prompt_tokens": spent.map(|value| value.prompt_tokens),
                "completion_tokens": spent.map(|value| value.completion_tokens),
                "cached_tokens": spent.and_then(|value| value.cached_tokens),
                "cache_write_tokens": spent.and_then(|value| value.cache_write_tokens),
            })
        })
        .collect()
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

/// Which authorization a terminal session runs under: the tenant door or the
/// narrower evaluator route beneath `<state>/eval` (larry#207).
#[derive(Debug, Clone, Copy)]
pub(crate) enum SessionDoor {
    /// A person's door: the tenant allowlist, open until its first root row.
    Tenant,
    /// An evaluator-owned origin: only beneath `<state>/eval`.
    Eval,
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
    door: SessionDoor,
) -> Result<Ended, ExecError> {
    check(instruction, cwd).map_err(ExecError::Session)?;
    // A host that cannot confine refuses here, before the ledger, the model or a copy.
    isolation.check()?;
    let prepared = super::prepared::prepare(settings)?;
    let repository = match door {
        SessionDoor::Tenant => prepared.ledger.authorized_repo(
            prepared.tenant_text.as_str(),
            settings.state_dir(),
            cwd,
            settings.is_local(),
        )?,
        SessionDoor::Eval => Ledger::authorized_eval_repo(settings.state_dir(), cwd)?,
    };
    run_session_prepared(
        instruction,
        &repository,
        settings,
        isolation,
        observer,
        &prepared,
    )
}

/// The system prompt a turn sends: the route's template, when it names one,
/// rides before the standing instruction, so a tenant's session prompts carry
/// its route's words (larry#205).
fn session_system(route: &Route) -> String {
    match route.prompt() {
        Some(template) => format!("{template}\n{SYSTEM}"),
        None => String::from(SYSTEM),
    }
}

/// The seal a session's report carries: the selected tenant's active key, so
/// a remote caller verifies under the tenant that ran it (larry#211).
fn session_seal(prepared: &super::prepared::Prepared) -> super::Seal<'_> {
    super::Seal {
        id: prepared.seal_id,
        key: &prepared.seal_key,
    }
}

/// Runs the session in an already authorized directory: the route, the model,
/// the gated proposal and the sealed report.
fn run_session_prepared(
    instruction: &str,
    cwd: &Path,
    settings: &Settings,
    isolation: Isolation,
    observer: &mut dyn Observer,
    prepared: &super::prepared::Prepared,
) -> Result<Ended, ExecError> {
    let ledger = &prepared.ledger;
    let route = settings.route(ledger, None)?;
    let config = settings.model_config(&route)?;
    let document = json::to_vec(&json!({"session": {"cwd": cwd.to_string_lossy(), "instruction": instruction, "isolation": isolation.name()}})).map_err(|cause| ExecError::Gate(cause.into()))?;
    let task_id = fresh_id()?;
    let task_digest = digest_hex(&document);
    let mut redactions = settings.secrets().to_vec();
    let absolute = cwd.to_string_lossy().into_owned();
    if let Some(redaction) = cwd_redaction(absolute) {
        redactions.push(redaction);
    }
    let mut stop = None;
    let subject = Subject {
        required: isolation.required(),
        path: cwd,
        bytes: &document,
    };
    let seal = session_seal(prepared);
    let report = gate::admit_and_run(
        subject,
        settings,
        &task_id,
        &task_digest,
        prepared.tenant,
        seal,
        || {
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
            let system = session_system(&route);
            let mut turn = Turn {
                client: &client,
                settings,
                ledger,
                task_id: &task_id,
                place: match island {
                    Some(ref isle) => isle.place(),
                    None => Place::Open(cwd),
                },
                // The model sees a relative directory in every isolation mode, so the
                // request carries no host path or account name.
                shown: String::from("."),
                redactions: &redactions,
                system,
                observer: &mut *observer,
            };
            let done = turn.run(instruction)?;
            let authored = authored_usage(&done.usage);
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
                "authored": authored,
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
            Ok(Report::sealed(body, &prepared.seal_key, prepared.seal_id))
        },
    )?;
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

#[cfg(test)]
mod issue207_tests {
    use super::{Approval, Isolation, Observer, run_session};
    use crate::executor::gate;
    use crate::executor::sim_tree::scratch;
    use crate::executor::{Ledger, Settings};

    /// An observer that must never be reached by a refusal before model setup.
    struct QuietObserver;

    impl Observer for QuietObserver {
        fn approve(&mut self, _step: usize, _command: &str) -> Approval {
            Approval::Decline
        }
    }

    /// A remote ACP session must refuse an unallowlisted cwd before route or model resolution.
    #[test]
    fn sim_session_refuses_an_unallowlisted_cwd_before_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = scratch("issue207-session-state")?;
        let work = scratch("issue207-session-work")?;
        let policy = gate::session_scopes();
        let state_text = state.to_string_lossy().into_owned();
        let path = std::env::var("PATH")?;
        let settings = Settings::remote_from_env({
            move |name| match name {
                "LARRY_STATE_DIR" => Some(state_text.clone()),
                "PATH" => Some(path.clone()),
                _ => None,
            }
        })?;
        Ledger::open(&state)?.set_tenant_policy(settings.tenant_text(), &policy)?;
        let mut observer = QuietObserver;
        assert!(matches!(
            run_session(
                "read this repository",
                &work,
                &settings,
                Isolation::Host,
                &mut observer,
                super::super::SessionDoor::Tenant,
            ),
            Err(super::super::ExecError::RepoNotAllowed { .. })
        ));
        Ok(())
    }
}

/// What one session's loop did.
struct Steps {
    /// Why it stopped.
    stop: Stop,
    /// Each command's record, in order.
    records: Vec<Value>,
    /// Usage reported by each model completion, in order.
    usage: Vec<Option<Usage>>,
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
    /// Exact secrets and the absolute workspace path removed from model-visible text.
    redactions: &'a [String],
    /// The system turn: the route's template when it names one, then the standing instruction.
    system: String,
    /// Who hears the session's progress.
    observer: &'a mut dyn Observer,
}

impl Turn<'_> {
    /// Asks, runs and observes until a stop, and returns it with each command's
    /// record, what compaction did and how many commands the person declined
    /// or interrupted.
    fn run(&mut self, instruction: &str) -> Result<Steps, ExecError> {
        let task = session_prompt(instruction, &self.shown);
        let mut conversation = Conversation::new(&self.system, task, self.client.request_bound());
        let mut steps: Vec<Value> = Vec::new();
        let mut usage = Vec::new();
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
            usage.push(completion.usage());
            if self.observer.cancelled() {
                break Stop::Cancelled;
            }
            let reply = completion.content();
            let kept = kept_reply(reply, self.redactions);
            match parse_reply(reply) {
                Ok(Reply::Done(summary)) => {
                    self.observer.message(&summary);
                    break Stop::EndTurn;
                }
                Ok(Reply::Command(command)) => {
                    refused = 0;
                    let step = steps.len();
                    let shown_command = redact_observation(&command, self.redactions);
                    if matches!(
                        self.observer.approve(step, &shown_command),
                        Approval::Decline
                    ) {
                        declined = declined.saturating_add(1);
                        let answer = Answer::Observed {
                            step,
                            command: shown_command,
                            status: String::from(DECLINED),
                            output: Some(String::new()),
                        };
                        conversation.push(kept, answer);
                        continue;
                    }
                    if self.observer.cancelled() {
                        break Stop::Cancelled;
                    }
                    let (answer, was_interrupted, result) = self.run_step(step, command)?;
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
            usage,
            compaction: conversation.json(),
            declined,
            interrupted,
        })
    }

    /// Runs `command` as step `step` and observes it: live output streams while the
    /// command runs (larry#252), and the person's cancel ends its whole group (larry#242).
    fn run_step(
        &mut self,
        step: usize,
        command: String,
    ) -> Result<(Answer, bool, CommandResult), ExecError> {
        let shown_command = redact_observation(&command, self.redactions);
        self.observer.started(step, &shown_command);
        let argv = [String::from("sh"), String::from("-c"), command];
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
        let answer = Answer::Observed {
            step,
            command: shown_command,
            status: status(&result),
            output: Some(redact_observation(&streams(&result), self.redactions)),
        };
        let was_interrupted = result.interrupted();
        let passed = result.exit_code() == Some(0) && !result.timed_out() && !was_interrupted;
        self.observer.finished(step, &answer.text(), passed);
        Ok((answer, was_interrupted, result))
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

/// The user turn that starts every terminal conversation.
fn session_prompt(instruction: &str, shown: &str) -> String {
    format!("Working directory: {shown}\n\nTask:\n{instruction}")
}

#[cfg(test)]
#[path = "sim_session.rs"]
pub(super) mod sim;

/// Seeded simulations of the session's model-egress redactions (larry#203):
/// the route key, the absolute working directory and secret-shaped values never
/// reach the model, and the bounds that guard them stay exact.
#[cfg(test)]
mod residency_tests {
    use lgwks_std::seeded::Seeded;

    use super::{MAX_REPLY_BYTES, cwd_redaction, kept_reply};

    /// The giant-and-empty fool at the reply bound: a reply of exactly the
    /// bound is kept, so only a longer one is dropped. A `>=` for the `>`
    /// would drop it.
    #[test]
    fn a_reply_of_exactly_the_bound_is_kept() {
        let reply = "r".repeat(MAX_REPLY_BYTES);
        assert_eq!(kept_reply(&reply, &[]), reply);
    }

    /// The giant-and-empty fool at the redaction floor: an eight-byte working
    /// directory is still a redaction, a seven-byte one is not. A `>` for the
    /// `>=` would let the eight-byte path reach the model.
    #[test]
    fn an_eight_byte_working_directory_is_still_a_redaction() {
        assert_eq!(
            cwd_redaction(String::from("12345678")),
            Some(String::from("12345678"))
        );
        assert_eq!(cwd_redaction(String::from("1234567")), None);
    }

    /// The saboteur runs after every fool: exact secrets and secret-shaped
    /// values never survive the kept reply at any seeded offset, while safe
    /// text does. The oracle is the invariant, never the code's output.
    #[test]
    fn sim_secret_shaped_observations_are_redacted() -> Result<(), Box<dyn std::error::Error>> {
        for seed in 0..256_u64 {
            let mut rng = Seeded::from_seed(seed);
            let secret = format!("session-secret-{seed:016x}");
            let offset = usize::try_from(rng.below(24)?)?;
            let pad = "p".repeat(offset);
            let reply = format!("{pad}KEY: {secret} Bearer {secret}-tail\nsafe-{seed}\n");
            let shown = kept_reply(&reply, std::slice::from_ref(&secret));
            assert!(!shown.contains(&secret), "seed {seed}: {shown}");
            assert!(
                shown.contains(&format!("safe-{seed}")),
                "seed {seed}: safe text was lost: {shown}"
            );
        }
        Ok(())
    }
}
