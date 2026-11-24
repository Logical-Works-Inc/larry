//! The thread that runs one task's terminal session for the frame (larry#239).
//!
//! `executor::run_session` is synchronous and owns its model calls and its
//! commands; the frame must keep painting while it runs. One thread per task
//! runs it with a [`Relay`] observer. Every event crosses a bounded channel of
//! [`EVENTS`]: a full channel blocks the session, never grows. Before each
//! command the relay asks the person and waits on a channel of one for the
//! answer; a frame that is gone answers by dropping it, and a dropped answer is
//! a decline, so nothing runs unless a person said yes (OWASP ASVS 4.0.3 §4.1.3,
//! deny by default). The thread is owned: [`Worker`] cancels and joins it on
//! drop, so no session outlives the frame that started it. A thread that panics
//! sends no `Ended`, so the read that finds its channel closed joins it and
//! ends the task as [`Ended::Failed`] naming the panic (larry#412).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use lgwks_std::json::Value;

use crate::executor::{
    Approval, ExecError, Handoff, Isolation, Observer, Report, Settings, Stop, run_session,
};

/// Events one session may have in flight to the frame.
pub(crate) const EVENTS: usize = 64;
/// Hex characters of the report's digest the receipt shows.
const DIGEST_CHARS: usize = 12;

/// The person's answer to one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Run it.
    Run,
    /// Run it and every later command of this Larry run without asking.
    Always,
    /// Do not run it; the model is told.
    Decline,
}

/// How a task ended, as the frame shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The model said it was done.
    Done,
    /// The session ran its 256 commands.
    StepBound,
    /// The gate, the gateway or the replies refused; the last message says why.
    Refused,
    /// The person interrupted it.
    Interrupted,
    /// Larry could not run the session: the state directory, the ledger, a route.
    Failed(String),
}

/// What the sealed report says, as the artifact pane shows it (larry#244).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Receipt {
    /// The first [`DIGEST_CHARS`] hex characters of the BLAKE3 digest of the report's bytes.
    pub(crate) digest: String,
    /// Whether the receipt's verdict is accepted.
    pub(crate) accepted: bool,
    /// Commands that ran.
    pub(crate) commands: usize,
    /// Commands the person declined.
    pub(crate) declined: usize,
    /// Commands the person interrupted.
    pub(crate) interrupted: usize,
    /// What became of the change: `applied`, `discarded`, `failed` or `none`.
    pub(crate) handoff: String,
}

/// The receipt of a sealed report body: its verdict, its bytes' digest and the session's counts.
/// A report the gate denied holds no `session`, so nothing ran: its counts are 0 and its
/// hand-off is `none`.
pub(crate) fn receipt_from(body: &Value, accepted: bool, bytes: &[u8]) -> Receipt {
    let session = body.get("session");
    let count = |name: &str| {
        let Some(reported) = session
            .and_then(|report| report.get(name))
            .and_then(Value::as_u64)
        else {
            return 0;
        };
        // A count past `usize` saturates at its ceiling; reporting 0 would read as no
        // commands at all (larry#343).
        match usize::try_from(reported) {
            Ok(number) => number,
            Err(_) => usize::MAX,
        }
    };
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(bytes);
    let handoff = match session
        .and_then(|report| report.get("handoff"))
        .and_then(Value::as_str)
    {
        Some(name) => name.to_owned(),
        None => String::from("none"),
    };
    Receipt {
        digest: hasher
            .finalize()
            .to_hex()
            .chars()
            .take(DIGEST_CHARS)
            .collect(),
        accepted,
        commands: count("commands"),
        declined: count("declined"),
        interrupted: count("interrupted"),
        handoff,
    }
}

/// What the session tells the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    /// The model asks to run `command`; the frame answers with [`Worker::decide`].
    Ask(String),
    /// `command` started.
    Started(String),
    /// The running command ended: what the model was shown, and whether it exited 0.
    Finished {
        /// The observation.
        observation: String,
        /// Exit 0, no timeout.
        passed: bool,
    },
    /// A message for the person: the summary, or why the session stopped.
    Said(String),
    /// The session ended with this change in its copy of the folder; the frame answers
    /// with [`Worker::decide`]: [`Decision::Run`] applies it, anything else discards it.
    Review(Vec<String>),
    /// What became of that change, and whether it is now in the person's folder.
    Handed {
        /// The one line that says so.
        line: String,
        /// Whether the folder changed.
        applied: bool,
    },
    /// The change as a unified diff (at most 1 MiB), sent once before its [`Event::Review`]
    /// so the artifact pane shows it while the person decides (larry#244).
    Diff(String),
    /// The sealed report's receipt, sent once after the session's last command, before
    /// [`Event::Ended`] (larry#244).
    Receipt(Receipt),
    /// The session ended; nothing follows.
    Ended(Ended),
}

/// The observer the session reports to, on the worker thread.
pub(super) struct Relay {
    /// Where events go.
    events: SyncSender<Event>,
    /// Where the person's answers come from.
    decisions: Receiver<Decision>,
    /// Set by the frame to stop the session.
    cancel: Arc<AtomicBool>,
    /// Whether the person said to stop asking.
    always: bool,
}

impl Relay {
    /// Sends `event`; a frame that is gone cancels the session.
    fn send(&self, event: Event) {
        if self.events.send(event).is_err() {
            self.cancel.store(true, Ordering::Release);
        }
    }
}

impl Observer for Relay {
    fn approve(&mut self, _step: usize, command: &str) -> Approval {
        if self.cancelled() {
            return Approval::Decline;
        }
        if self.always {
            return Approval::Run;
        }
        self.send(Event::Ask(command.to_owned()));
        match self.decisions.recv() {
            Ok(Decision::Run) => Approval::Run,
            Ok(Decision::Always) => {
                self.always = true;
                Approval::Run
            }
            Ok(Decision::Decline) | Err(_) => Approval::Decline,
        }
    }

    fn started(&mut self, _step: usize, command: &str) {
        self.send(Event::Started(command.to_owned()));
    }

    fn finished(&mut self, _step: usize, observation: &str, passed: bool) {
        self.send(Event::Finished {
            observation: observation.to_owned(),
            passed,
        });
    }

    fn message(&mut self, text: &str) {
        self.send(Event::Said(text.to_owned()));
    }

    /// Always asks, even after "always": a change to the person's own folder is never
    /// standing consent. Only [`Decision::Run`] applies; a cancel, "no", a closed frame
    /// and every other answer discard.
    fn review(&mut self, summary: &[String]) -> Handoff {
        if self.cancelled() {
            return Handoff::Discard;
        }
        self.send(Event::Review(summary.to_vec()));
        match self.decisions.recv() {
            Ok(Decision::Run) => Handoff::Apply,
            Ok(Decision::Always | Decision::Decline) | Err(_) => Handoff::Discard,
        }
    }

    fn handed(&mut self, line: &str, applied: bool) {
        self.send(Event::Handed {
            line: line.to_owned(),
            applied,
        });
    }

    fn diff(&mut self, text: &str) {
        self.send(Event::Diff(text.to_owned()));
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

/// The receipt of `report`, read through its public accessors.
fn receipt_of(report: &Report) -> Receipt {
    receipt_from(report.json(), report.accepted(), report.bytes())
}

/// What a finished session becomes for the frame.
fn ended(outcome: Result<Stop, ExecError>) -> Ended {
    match outcome {
        Ok(Stop::EndTurn) => Ended::Done,
        Ok(Stop::MaxTurnRequests) => Ended::StepBound,
        Ok(Stop::Refusal(_)) => Ended::Refused,
        Ok(Stop::Cancelled) => Ended::Interrupted,
        Err(cause) => Ended::Failed(cause.to_string()),
    }
}

/// One running task: its events, its answer channel, its cancel flag and its thread.
#[derive(Debug)]
pub(crate) struct Worker {
    /// Events from the session; `None` once dropped.
    events: Option<Receiver<Event>>,
    /// Answers to the session; `None` once dropped.
    decisions: Option<SyncSender<Decision>>,
    /// Stops the session before its next model call or command.
    cancel: Arc<AtomicBool>,
    /// The thread, joined on drop.
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts the session on `instruction` in `cwd` under `isolation`; `always` carries
    /// the person's "don't ask again" from an earlier task of this run.
    pub(crate) fn spawn(
        instruction: String,
        cwd: PathBuf,
        settings: Settings,
        isolation: Isolation,
        always: bool,
    ) -> std::io::Result<Self> {
        Self::start(always, move |relay| {
            match run_session(&instruction, &cwd, &settings, isolation, relay) {
                Ok(done) => {
                    relay.send(Event::Receipt(receipt_of(&done.report)));
                    Ok(done.stop)
                }
                Err(cause) => Err(cause),
            }
        })
    }

    /// Runs `job` on the session thread with its relay; the job's outcome is the last event.
    /// `spawn` is its one production caller; the worker sims give it a job that breaks.
    pub(super) fn start<F>(always: bool, job: F) -> std::io::Result<Self>
    where
        F: FnOnce(&mut Relay) -> Result<Stop, ExecError> + Send + 'static,
    {
        let (event_tx, events) = sync_channel(EVENTS);
        let (decisions, decision_rx) = sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let relay = Relay {
            events: event_tx,
            decisions: decision_rx,
            cancel: Arc::clone(&cancel),
            always,
        };
        let thread = std::thread::Builder::new()
            .name(String::from("larry-session"))
            .spawn(move || {
                let mut relay = relay;
                let outcome = job(&mut relay);
                relay.send(Event::Ended(ended(outcome)));
            })?;
        Ok(Self {
            events: Some(events),
            decisions: Some(decisions),
            cancel,
            thread: Some(thread),
        })
    }

    /// The next event, if one is waiting; the frame reads with it between paints.
    pub(crate) fn next(&mut self) -> Option<Event> {
        self.wait(Duration::ZERO)
    }

    /// The next event within `patience`. A session that ended sent [`Event::Ended`] last, so
    /// a closed channel after it reads as nothing. A channel that closed with no `Ended` is a
    /// thread that panicked: the read joins it and ends the task as [`Ended::Failed`] naming
    /// the panic, once (larry#412). The worker sims wait on it instead of sleeping.
    pub(crate) fn wait(&mut self, patience: Duration) -> Option<Event> {
        let got = self.events.as_ref()?.recv_timeout(patience);
        match got {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.events = None;
                self.join().map(|why| Event::Ended(Ended::Failed(why)))
            }
        }
    }

    /// Stops and reaps the session at quit: the panic it stopped on, if it panicked, so the
    /// caller reports it once the screen is restored (larry#412).
    pub(crate) fn finish(mut self) -> Option<String> {
        self.halt();
        self.join()
    }

    /// Cancels, then drops both channels so a session blocked on a send or on an answer
    /// wakes to a decline and ends.
    fn halt(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.decisions = None;
        self.events = None;
    }

    /// Joins the thread, once: the text its panic carried, if it panicked.
    fn join(&mut self) -> Option<String> {
        let payload = self.thread.take()?.join().err()?;
        Some(format!(
            "the session stopped on a panic: {}",
            crate::panic_text(&*payload)
        ))
    }

    /// Answers the command the session asked about; a session that already
    /// ended has nobody to answer, which changes nothing.
    pub(crate) fn decide(&self, decision: Decision) {
        if let Some(decisions) = self.decisions.as_ref()
            && decisions.try_send(decision).is_err()
        {
            self.cancel.store(true, Ordering::Release);
        }
    }

    /// Stops the session before its next model call or command.
    pub(crate) fn interrupt(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.halt();
        // `tui::run` finishes its worker on every return, so a thread still held here is the
        // frame's own unwind: that panic is the failure the process reports, and std's panic
        // hook named this session's panic as it happened. The join reaps the thread.
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            self.cancel.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::{Receiver, sync_channel};

    use lgwks_std::json::json;

    use super::{Decision, Event, Handoff, Observer, Receipt, Relay, receipt_from};

    /// A relay whose person has `answer` waiting (`None`: the frame is gone), and its events.
    fn relay(answer: Option<Decision>, always: bool, cancelled: bool) -> (Relay, Receiver<Event>) {
        let (event_tx, events) = sync_channel(4);
        let (decision_tx, decisions) = sync_channel(1);
        if let Some(decision) = answer {
            assert!(decision_tx.try_send(decision).is_ok());
        }
        let relay = Relay {
            events: event_tx,
            decisions,
            cancel: Arc::new(AtomicBool::new(cancelled)),
            always,
        };
        (relay, events)
    }

    /// Only an explicit yes applies a change to the person's folder: "always", "no", a gone
    /// frame and a cancel each discard, and a standing "always" never skips the question.
    #[test]
    fn sim_tui_worker_review_applies_only_on_run() {
        let cases = [
            (Some(Decision::Run), false, false, Handoff::Apply, true),
            (Some(Decision::Run), true, false, Handoff::Apply, true),
            (Some(Decision::Always), false, false, Handoff::Discard, true),
            (
                Some(Decision::Decline),
                false,
                false,
                Handoff::Discard,
                true,
            ),
            (None, false, false, Handoff::Discard, true),
            (Some(Decision::Run), false, true, Handoff::Discard, false),
        ];
        for (answer, always, cancelled, want, asked) in cases {
            let (mut relay, events) = relay(answer, always, cancelled);
            let summary = vec![String::from("1 file changed, +1 -0")];
            assert_eq!(
                relay.review(&summary),
                want,
                "{answer:?} always {always} cancelled {cancelled}"
            );
            let sent = events.try_recv();
            assert_eq!(
                sent.is_ok(),
                asked,
                "{answer:?}: the question was or was not sent"
            );
            if let Ok(event) = sent {
                assert_eq!(event, Event::Review(summary));
            }
        }
    }

    /// The receipt of a sealed body: each count and the hand-off name are read from where the
    /// session writes them, a body with no `session` (a denied run) reads as nothing ran, a
    /// member of the wrong type reads as 0, and the digest is the BLAKE3 prefix of the bytes.
    #[test]
    fn receipt_of_reads_each_member_and_names_a_missing_one() {
        let body = json!({
            "session": {
                "commands": 3, "declined": 1, "interrupted": 2, "handoff": "applied"
            }
        });
        let receipt = receipt_from(&body, true, b"sealed bytes");
        let mut reference = lgwks_std::hash::Hasher::new();
        reference.update(b"sealed bytes");
        let full = reference.finalize().to_hex();
        assert_eq!(receipt.digest.len(), 12);
        assert!(
            full.starts_with(&receipt.digest),
            "{full} {}",
            receipt.digest
        );
        assert_eq!(
            (
                receipt.accepted,
                receipt.commands,
                receipt.declined,
                receipt.interrupted,
                receipt.handoff.as_str()
            ),
            (true, 3, 1, 2, "applied")
        );
        let other = receipt_from(&body, true, b"sealed bytez");
        assert_ne!(other.digest, receipt.digest, "one byte changes the digest");
        let denied = receipt_from(&json!({"denied": ["gate: no"]}), false, b"");
        assert_eq!(
            denied,
            Receipt {
                digest: denied.digest.clone(),
                accepted: false,
                commands: 0,
                declined: 0,
                interrupted: 0,
                handoff: String::from("none"),
            }
        );
        for wrong in [json!("7"), json!(-1), json!(1.5), json!(null), json!([1])] {
            let body = json!({"session": {"commands": wrong, "handoff": 4}});
            let read = receipt_from(&body, true, b"x");
            assert_eq!((read.commands, read.handoff.as_str()), (0, "none"));
        }
    }

    /// A count past `usize` saturates at its ceiling; it never reads as zero, which would
    /// say no command ran (larry#343).
    #[test]
    fn a_count_past_usize_saturates() {
        let body = json!({"session": {"commands": u64::MAX, "declined": 2}});
        let receipt = receipt_from(&body, true, b"");
        assert_eq!(receipt.commands, usize::MAX, "the count saturates");
        assert_eq!(receipt.declined, 2, "a count within usize is its own");
    }

    /// Both artifacts cross the observer contract whole: the diff the island offers arrives as
    /// one `Event::Diff` carrying every byte, empty and past a megabyte alike, and nothing else.
    #[test]
    fn sim_tui_worker_receipt_and_diff_cross_the_observer_contract() {
        for len in [0_usize, 1, 4096, 1_048_576] {
            let text = "+x\n".repeat(len.checked_div(3).into_iter().sum());
            let (mut relay, events) = relay(None, false, false);
            relay.diff(&text);
            assert_eq!(events.try_recv().ok(), Some(Event::Diff(text)), "{len}");
            assert!(events.try_recv().is_err(), "{len}: exactly one event");
        }
    }

    /// The hand-off line reaches the frame whole, with whether the folder changed.
    #[test]
    fn sim_tui_worker_handed_reaches_the_frame() {
        let (mut relay, events) = relay(None, false, false);
        relay.handed("applied to your folder", true);
        assert_eq!(
            events.try_recv().ok(),
            Some(Event::Handed {
                line: String::from("applied to your folder"),
                applied: true
            })
        );
    }
}
