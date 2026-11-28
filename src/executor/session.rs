//! A terminal session (larry#93): the model works in one directory by choosing
//! one step per reply, until it says it is done. This is what a
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
//! Each reply is exactly one JSON object: `{"command": "<sh -c text>"}`,
//! `{"read": "<path>"}`, `{"edit": {"path", "old", "new", "expect_digest"}}` or
//! `{"done": "<summary>"}`. Each command runs in its own process group under the
//! executor's timeout, tracked in the ledger so a kill -9 is reaped by the next
//! start; the model sees the exit status and the first 4 KiB of each stream. A
//! read shows the file's BLAKE3 digest and its first 4 KiB (larry#586). An edit
//! applies only to the file that still has that digest (larry#606).
//! Auto-compaction (`super::compact`, larry#235) keeps every request inside the
//! model's window, so the length of the conversation never stops a session. The
//! loop (`super::session_turn`) stops at the model's `done`, at 256 steps, at
//! three refused replies in a row, at a model refusal, or at a cancellation
//! checked before every model call and every step and polled while a command
//! runs: the person's cancel kills the running command's process group and the
//! report counts it interrupted (larry#242). The sealed report records every
//! command in `results`, every read and edit in `changes`, and what compaction did.

use std::path::Path;

use lgwks_macros::Error;
use lgwks_std::json::{self, Value, json};

use super::ExecError;
use super::Ledger;
use super::Report;
use super::Settings;
use super::backoff;
use super::digest_hex;
use super::gate::{self, Subject};
use super::identity::fresh_id;
use super::island::{Island, Isolation, Outcome};
use super::run::{Place, StreamCallback};
use super::session_turn::{SYSTEM, Turn};
use super::workspace::report_members;
use crate::model::{ModelClient, Route, Usage};

/// A terminal session's reply protocol: one JSON object per turn, parsed and
/// refused with an exact reason the model can retry from (larry#545).
#[path = "session_reply.rs"]
pub(crate) mod reply;
/// Largest instruction a session takes, in bytes.
pub(crate) const MAX_INSTRUCTION_BYTES: usize = 65_536;
/// Largest model reply the session parses or keeps in the conversation, in bytes.
pub(super) const MAX_REPLY_BYTES: usize = 8192;
/// Largest command or summary one reply may carry, in bytes.
const MAX_COMMAND_BYTES: usize = 4096;
/// Bytes of each output stream the model sees and the report keeps.
pub(crate) const OBSERVED_BYTES: usize = 4096;

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
    /// The session took 256 steps.
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
    /// The model asks to take step `step`, which `command` describes: a shell
    /// command, `read <path>` or an edit with its texts. Nothing happens unless this
    /// answers [`Approval::Run`]. A frontend asks the person; `larry acp` runs
    /// inside the operator's isolated container under the same gate and runs it.
    /// No default: every observer says aloud whether a step may run (larry#239).
    fn approve(&mut self, step: usize, command: &str) -> Approval;
    /// Step `step` is about to run `command`. An observer that records nothing
    /// leaves this alone.
    fn started(&mut self, _step: usize, _command: &str) {}
    /// Step `step` ended; `observation` is what the model is shown, `passed` its
    /// exit 0, or that the read or change was not refused. Called once per step.
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

/// The confined copy an isolated session works in; `None` on the host.
fn open_island(
    isolation: Isolation,
    settings: &Settings,
    task_id: &str,
    cwd: &Path,
) -> Result<Option<Island>, ExecError> {
    match isolation {
        Isolation::Host => Ok(None),
        Isolation::Workspace(platform) => Island::open(platform, settings, task_id, cwd).map(Some),
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
            let island = open_island(isolation, settings, &task_id, cwd)?;
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
                // The model is told its directory is `.` in a copy, so the request carries
                // no run id and no path of the person's.
                shown: match island {
                    Some(_) => String::from("."),
                    None => cwd.display().to_string(),
                },
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
            let commands = done.tally.results.len();
            let mut body = json!({
                "task_id": task_id,
                "task_digest": task_digest,
                "diff": null,
                "results": done.tally.results,
                "changes": done.tally.changes,
                "authored": authored,
                "denied": [],
                "session": {
                    "stop": done.stop.name(),
                    "commands": commands,
                    "reads": done.tally.reads,
                    "edits": done.tally.edits,
                    "declined": done.tally.declined,
                    "interrupted": done.tally.interrupted,
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

#[cfg(test)]
#[path = "sim_session.rs"]
pub(super) mod sim;

#[cfg(test)]
#[path = "sim_run_session.rs"]
mod sim_run;
