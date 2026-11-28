//! A terminal session's turn loop (larry#93, larry#586, larry#606): one model
//! reply after another, each a command, a read, an edit or done, until the model
//! says it is done. Every step runs only after the person approves it. Every
//! command runs in its own process group under the executor's timeout and is
//! recorded in `results`, which the session's verdict reads; every read and edit
//! is recorded in `changes`. The loop stops at the model's `done`, at 256 steps,
//! at three refused replies in a row, at a model refusal, or at a cancellation.

use std::path::Path;

use lgwks_std::json::{Value, json};

use super::ExecError;
use super::Ledger;
use super::Settings;
use super::compact::{Answer, Conversation};
use super::edit::{Edit, EditError, FileView, apply_edit, read_file};
use super::run::{CommandResult, Place, RunHooks, clip, run_command};
use super::session::reply::{Reply, parse_reply};
use super::session::{Approval, MAX_REPLY_BYTES, OBSERVED_BYTES, Observer, Stop};
use crate::model::{Completion, ModelClient, Usage};

/// Most steps one session takes before it stops with `MaxTurnRequests`. The
/// conversation is bounded by compaction, not by this; it bounds the report, whose
/// records are at most 256 times 12 KiB (a record holds no file and no edit text).
const MAX_STEPS: usize = 256;

/// Most replies in a row the session refuses before it stops.
const MAX_REFUSED: usize = 3;

/// The standing instruction of every session.
pub(super) const SYSTEM: &str = "You are Larry, a terminal agent. You work in the directory named below. Reply with exactly one JSON object and nothing else: {\"command\": \"<one shell command>\"} runs it with sh -c in that directory and shows you its exit status and output; {\"read\": \"<repo-relative path>\"} shows you that file and its digest; {\"edit\": {\"path\": \"<repo-relative path>\", \"old\": \"<text in the file>\", \"new\": \"<its replacement>\", \"expect_digest\": \"<the digest your last read or edit of the file showed>\"}} replaces the one place old matches, and is refused when the digest is stale or the match is missing or ambiguous; {\"done\": \"<one-line summary>\"} ends the task. Read a file before you edit it. Every command, read and edit runs only after the person approves. Send one object per reply.";

/// What the model is shown for a command the person declined.
const DECLINED: &str = "not run: the person declined this command";

/// What the model is shown for a read the person declined.
const DECLINED_READ: &str = "not run: the person declined this read";

/// What the model is shown for an edit the person declined.
const DECLINED_EDIT: &str = "not run: the person declined this edit";

/// What the session's steps did, as the report records it.
#[derive(Default)]
pub(super) struct Tally {
    /// Each command's record, in order.
    pub(super) results: Vec<Value>,
    /// Each read's and edit's record, in order.
    pub(super) changes: Vec<Value>,
    /// Reads that ran, refused ones included.
    pub(super) reads: usize,
    /// Edits that ran, refused ones included.
    pub(super) edits: usize,
    /// Steps the person declined; none of them ran.
    pub(super) declined: usize,
    /// Commands the person interrupted; each had its process group killed (larry#242).
    pub(super) interrupted: usize,
}

impl Tally {
    /// The number of the next step: every step that ran holds one record.
    fn step(&self) -> usize {
        self.results.len().saturating_add(self.changes.len())
    }
}

/// What one session's loop did.
pub(super) struct Steps {
    /// Why it stopped.
    pub(super) stop: Stop,
    /// The records and counts of its steps.
    pub(super) tally: Tally,
    /// Usage reported by each model completion, in order.
    pub(super) usage: Vec<Option<Usage>>,
    /// What compaction did.
    pub(super) compaction: Value,
}

/// What one reply came to: what the model is shown for it, if anything, and the
/// stop that ends the session, if it ended.
type Outcome = (Option<Answer>, Option<Stop>);

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
    /// Where every step happens: the directory itself, or its confined copy.
    pub(super) place: Place<'a>,
    /// The directory the model is told it works in.
    pub(super) shown: String,
    /// The system turn: the route's template when it names one, then the standing instruction.
    pub(super) system: String,
    /// Who hears the session's progress.
    pub(super) observer: &'a mut dyn Observer,
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

/// The model reply as the conversation keeps it: the reply itself, or only its
/// length when it is past the bound, so an oversized reply cannot grow the
/// next request.
fn kept_reply(reply: &str) -> String {
    if reply.len() > MAX_REPLY_BYTES {
        format!(
            "(a reply of {} bytes, over the bound, was dropped)",
            reply.len()
        )
    } else {
        reply.to_owned()
    }
}

/// What the person approves for an edit: the path first, so a frontend's title
/// names it, then the digest and the two texts the yes covers.
fn proposal(edit: &Edit) -> String {
    format!(
        "edit {}\nat digest {}\n--- old\n{}\n+++ new\n{}",
        edit.path, edit.expect_digest, edit.old, edit.new
    )
}

/// What the model is shown for a file it read: the digest of the whole file,
/// then the file, cut like a command's output when it is long.
fn render_view(view: FileView) -> String {
    let head = format!("digest {} ({} bytes)", view.digest, view.text.len());
    let mut text = view.text;
    if clip(&mut text, OBSERVED_BYTES) {
        format!("{head}, only the first {OBSERVED_BYTES} bytes shown\n{text}")
    } else {
        format!("{head}\n{text}")
    }
}

impl Turn<'_> {
    /// Asks, runs and observes until a stop, and returns it with each step's
    /// record, what compaction did and what each completion cost.
    pub(super) fn run(&mut self, instruction: &str) -> Result<Steps, ExecError> {
        let task = format!("Working directory: {}\n\nTask:\n{instruction}", self.shown);
        let mut conversation = Conversation::new(&self.system, task, self.client.request_bound());
        let mut tally = Tally::default();
        let mut usage = Vec::new();
        let mut refused = 0_usize;
        let stop = loop {
            if tally.step().saturating_add(tally.declined) >= MAX_STEPS {
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
            // A reply past the bound is refused below; only its length enters the
            // conversation, so it cannot grow the next request.
            let kept = kept_reply(reply);
            match parse_reply(reply) {
                Ok(parsed) => {
                    refused = 0;
                    let (answer, stop) = self.take_step(&mut tally, parsed)?;
                    if let Some(answer) = answer {
                        conversation.push(kept, answer);
                    }
                    if let Some(stop) = stop {
                        break stop;
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
            tally,
            usage,
            compaction: conversation.json(),
        })
    }

    /// Takes one parsed reply: done ends the session; everything else is put to
    /// the person and runs only on a yes.
    fn take_step(&mut self, tally: &mut Tally, reply: Reply) -> Result<Outcome, ExecError> {
        let step = tally.step();
        match reply {
            Reply::Done(summary) => {
                self.observer.message(&summary);
                Ok((None, Some(Stop::EndTurn)))
            }
            Reply::Command(command) => {
                if let Some(withheld) = self.consent(tally, step, &command, DECLINED) {
                    return Ok(withheld);
                }
                let (answer, result) = self.run_step(step, command)?;
                let interrupted = result.interrupted();
                tally.results.push(result.into_json());
                if interrupted {
                    tally.interrupted = tally.interrupted.saturating_add(1);
                }
                Ok((Some(answer), interrupted.then_some(Stop::Cancelled)))
            }
            Reply::Read { path } => {
                let summary = format!("read {path}");
                if let Some(withheld) = self.consent(tally, step, &summary, DECLINED_READ) {
                    return Ok(withheld);
                }
                Ok((Some(self.run_read(tally, step, summary, &path)), None))
            }
            Reply::Edit(edit) => {
                if let Some(withheld) = self.consent(tally, step, &proposal(&edit), DECLINED_EDIT) {
                    return Ok(withheld);
                }
                Ok((Some(self.run_edit(tally, step, &edit)), None))
            }
        }
    }

    /// Puts `summary` to the person as step `step`. `None` means run it;
    /// otherwise the outcome of not running it: a decline the model is told of
    /// with `status`, or a cancellation that arrived while the person decided.
    fn consent(
        &mut self,
        tally: &mut Tally,
        step: usize,
        summary: &str,
        status: &str,
    ) -> Option<Outcome> {
        if self.observer.approve(step, summary) == Approval::Decline {
            tally.declined = tally.declined.saturating_add(1);
            let answer = Answer::Observed {
                step,
                command: summary.to_owned(),
                status: status.to_owned(),
                output: Some(String::new()),
            };
            return Some((Some(answer), None));
        }
        self.observer
            .cancelled()
            .then_some((None, Some(Stop::Cancelled)))
    }

    /// Runs `command` as step `step` and observes it: live output streams while the
    /// command runs (larry#252), and the person's cancel ends its whole group (larry#242).
    fn run_step(
        &mut self,
        step: usize,
        command: String,
    ) -> Result<(Answer, CommandResult), ExecError> {
        self.observer.started(step, &command);
        let argv = [String::from("sh"), String::from("-c"), command.clone()];
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
            command,
            status: status(&result),
            output: Some(streams(&result)),
        };
        let passed = result.exit_code() == Some(0) && !result.timed_out() && !result.interrupted();
        self.observer.finished(step, &answer.text(), passed);
        Ok((answer, result))
    }

    /// The directory reads and edits happen in: the folder itself, or its confined copy.
    fn tree(&self) -> &Path {
        match self.place {
            Place::Open(path) => path,
            Place::Confined(confine) => confine.tree(),
        }
    }

    /// Reads one approved file and shows the model its digest and text, or the
    /// typed refusal. The report keeps the digest and length, never the file.
    fn run_read(&mut self, tally: &mut Tally, step: usize, command: String, path: &str) -> Answer {
        self.observer.started(step, &command);
        tally.reads = tally.reads.saturating_add(1);
        let outcome = read_file(self.tree(), path).map(|view| {
            let record = json!({
                "step": step,
                "kind": "read",
                "path": path,
                "digest": view.digest.as_str(),
                "bytes": view.text.len(),
            });
            (record, "read", render_view(view))
        });
        let refused = json!({"step": step, "kind": "read", "path": path});
        self.close(tally, step, command, outcome, refused)
    }

    /// Applies one approved edit to the session tree and shows the model the
    /// digest it left, or the typed refusal, which wrote nothing. The report
    /// keeps both digests, never the texts.
    fn run_edit(&mut self, tally: &mut Tally, step: usize, edit: &Edit) -> Answer {
        let path = edit.path.as_str();
        let command = format!("edit {path}");
        self.observer.started(step, &command);
        tally.edits = tally.edits.saturating_add(1);
        let outcome = apply_edit(self.tree(), edit).map(|receipt| {
            let record = json!({
                "step": step,
                "kind": "edit",
                "path": path,
                "before_digest": receipt.before_digest.as_str(),
                "after_digest": receipt.after_digest.as_str(),
            });
            let shown = format!(
                "digest {} -> {}",
                receipt.before_digest, receipt.after_digest
            );
            (record, "edited", shown)
        });
        let refused = json!({"step": step, "kind": "edit", "path": path});
        self.close(tally, step, command, outcome, refused)
    }

    /// Ends a read's or an edit's step: its record is kept, `refused` with the
    /// reason when it was refused, and the observer hears the step end once.
    fn close(
        &mut self,
        tally: &mut Tally,
        step: usize,
        command: String,
        outcome: Result<(Value, &str, String), EditError>,
        mut refused: Value,
    ) -> Answer {
        let passed = outcome.is_ok();
        let (record, status, output) = match outcome {
            Ok(done) => done,
            Err(cause) => {
                let refusal = cause.to_string();
                if let Some(record) = refused.as_object_mut() {
                    record.insert(String::from("refusal"), json!(refusal.as_str()));
                }
                (refused, "refused", refusal)
            }
        };
        tally.changes.push(record);
        let answer = Answer::Observed {
            step,
            command,
            status: status.to_owned(),
            output: Some(output),
        };
        self.observer.finished(step, &answer.text(), passed);
        answer
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
#[path = "sim_turn.rs"]
mod sim;
