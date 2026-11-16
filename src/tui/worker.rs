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
//! drop, so no session outlives the frame that started it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use crate::executor::{Approval, ExecError, Observer, Settings, Stop, run_session};

/// Events one session may have in flight to the frame.
pub(crate) const EVENTS: usize = 64;

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
    /// The session ended; nothing follows.
    Ended(Ended),
}

/// The observer the session reports to, on the worker thread.
struct Relay {
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

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
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
    /// Starts the session on `instruction` in `cwd`; `always` carries the
    /// person's "don't ask again" from an earlier task of this run.
    pub(crate) fn spawn(
        instruction: String,
        cwd: PathBuf,
        settings: Settings,
        always: bool,
    ) -> std::io::Result<Self> {
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
                let outcome =
                    run_session(&instruction, &cwd, &settings, &mut relay).map(|done| done.stop);
                relay.send(Event::Ended(ended(outcome)));
            })?;
        Ok(Self {
            events: Some(events),
            decisions: Some(decisions),
            cancel,
            thread: Some(thread),
        })
    }

    /// The next event, if one is waiting. A session that is gone sent
    /// [`Event::Ended`] last, so an empty and a closed channel read the same.
    pub(crate) fn next(&self) -> Option<Event> {
        self.events.as_ref()?.try_iter().next()
    }

    /// The next event within `patience`; the worker sims wait on it instead of sleeping.
    #[cfg(test)]
    pub(crate) fn wait(&self, patience: std::time::Duration) -> Option<Event> {
        self.events.as_ref()?.recv_timeout(patience).ok()
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
        // Cancel, then drop both channels so a session blocked on a send or on
        // an answer wakes to a decline and ends; then reap the thread.
        self.cancel.store(true, Ordering::Release);
        self.decisions = None;
        self.events = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            self.cancel.store(true, Ordering::Release);
        }
    }
}
