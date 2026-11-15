//! Auto-compaction of a terminal session's conversation (larry#235, ADR 0044).
//!
//! A session's conversation grows by one reply and one observation per step.
//! Before every model call the session encodes the request once. Past three
//! quarters of its window it compacts to half, with no model call and no
//! operator. First it clears the output of every observation but the last four:
//! the command and its exit status stay, so the model can run it again. Then it
//! folds the oldest steps into one record, one line per step, until the request
//! fits. The window starts at the route's request bound. A gateway that refuses
//! a request as past the model's context window shrinks it to three quarters of
//! that request, so a model smaller than its route says still finishes.
//!
//! The system turn and the task turn never change, so a provider's prefix cache
//! holds across every compaction, and the record turns change only when a
//! compaction runs. Nothing here reads a clock or a random source: every
//! decision is a byte length, so one seed replays one conversation.

use std::collections::VecDeque;
use std::fmt;

use lgwks_std::json::{Value, json};

use super::run::clip;
use crate::model::{MAX_MESSAGES, Message, ModelClient, Request, Role};

/// Observations kept whole at the tail; older ones keep their status line only.
const KEEP_OBSERVED: usize = 4;
/// Exchanges never folded: the newest stays whole, so the model sees its last step.
const KEEP_EXCHANGES: usize = 1;
/// Most exchanges kept unfolded.
const MAX_EXCHANGES: usize = 120;
/// Exchanges a fold past [`MAX_EXCHANGES`] leaves, so that fold runs once in 24
/// steps rather than on every step.
const LOW_EXCHANGES: usize = 96;
/// Most lines the record keeps; older lines are counted, never kept.
const MAX_RECORD_LINES: usize = 64;
/// Bytes of a command or a refusal one record line keeps.
const RECORD_TEXT_BYTES: usize = 160;
/// Most times one session shrinks its window after a gateway refusal.
pub(super) const MAX_SHRINKS: usize = 8;
/// The JSON two turns add to a request around their text, rounded up:
/// `{"role":"assistant","content":""}`, its user twin and two commas. Only the
/// fold's estimate reads it; every decision re-measures the encoded request.
const TURN_FRAMING: usize = 64;
/// What the user turn after the record says.
const CONTINUE: &str =
    "Continue the task. Every step in that record ran; run a command again to see its output.";
/// What follows a cleared observation's status line.
const CLEARED: &str =
    " (its output was cleared by auto-compaction; run the command again to see it)";
/// What opens the note a refused reply gets.
const REFUSED: &str = "Your reply was refused: ";
/// What closes it.
const REFUSED_ASK: &str =
    ". Reply with exactly one JSON object: {\"command\": \"...\"} or {\"done\": \"...\"}.";

const _: () = assert!(
    MAX_MESSAGES.checked_sub(4 + 2 * MAX_EXCHANGES).is_some(),
    "the head, the record and every unfolded exchange fit the client's message bound"
);
const _: () = assert!(
    matches!(MAX_EXCHANGES.checked_sub(LOW_EXCHANGES), Some(1..)),
    "a fold past the exchange bound leaves fewer than the bound"
);

/// `parts` quarters of `whole`, rounded down.
const fn quarters(whole: usize, parts: usize) -> usize {
    whole.div_euclid(4).saturating_mul(parts)
}

/// The user turn that answered one reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Answer {
    /// A command ran.
    Observed {
        /// The step's number, from 0.
        step: usize,
        /// The command the reply asked for.
        command: String,
        /// The exit status line.
        status: String,
        /// Its streams as the model is shown them, `None` once compaction cleared them.
        output: Option<String>,
    },
    /// The reply was refused, and why, as the parser said it.
    Refused {
        /// The parser's reason.
        fault: String,
    },
}

impl Answer {
    /// The turn's text as the model is shown it now.
    pub(super) fn text(&self) -> String {
        match *self {
            Self::Observed {
                ref status,
                output: Some(ref output),
                ..
            } => format!("{status}\n{output}"),
            Self::Observed {
                ref status,
                output: None,
                ..
            } => format!("{status}{CLEARED}"),
            Self::Refused { ref fault } => format!("{REFUSED}{fault}{REFUSED_ASK}"),
        }
    }

    /// The length of [`Answer::text`], counted without building it.
    fn text_len(&self) -> usize {
        match *self {
            Self::Observed {
                ref status,
                output: Some(ref output),
                ..
            } => status.len().saturating_add(1).saturating_add(output.len()),
            Self::Observed {
                ref status,
                output: None,
                ..
            } => status.len().saturating_add(CLEARED.len()),
            Self::Refused { ref fault } => REFUSED
                .len()
                .saturating_add(fault.len())
                .saturating_add(REFUSED_ASK.len()),
        }
    }

    /// The one line the record keeps for this turn.
    fn record_line(&self) -> String {
        match *self {
            Self::Observed {
                step,
                ref command,
                ref status,
                ..
            } => format!("step {step}: {} -> {status}", one_line(command)),
            Self::Refused { ref fault } => format!("a reply was refused: {}", one_line(fault)),
        }
    }
}

/// `text` on one line, clipped to [`RECORD_TEXT_BYTES`] on a character boundary.
fn one_line(text: &str) -> String {
    let mut line = text.replace(['\n', '\r'], " ");
    if clip(&mut line, RECORD_TEXT_BYTES) {
        line.push_str(" ...");
    }
    line
}

/// One model reply and the user turn that answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Exchange {
    /// The reply as the conversation keeps it.
    reply: String,
    /// What answered it.
    answer: Answer,
}

/// What auto-compaction did in one session; sealed in its report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Tally {
    /// Compactions run.
    pub(super) runs: usize,
    /// Observations whose output was cleared.
    pub(super) cleared: usize,
    /// Steps folded into the record.
    pub(super) folded: usize,
    /// Times a gateway refusal shrank the window.
    pub(super) shrinks: usize,
}

/// Why a conversation cannot be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Overflow {
    /// Compacted as far as it goes, the request is still over the window.
    Irreducible {
        /// The request's encoded length, in bytes.
        len: usize,
        /// The window, in bytes.
        window: usize,
    },
    /// The gateway refused the window [`MAX_SHRINKS`] times.
    Shrinks,
}

impl fmt::Display for Overflow {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Irreducible { len, window } => write!(
                out,
                "compacted as far as it goes, the request is {len} bytes, over the model's window of {window}"
            ),
            Self::Shrinks => write!(
                out,
                "the gateway refused the model's window {MAX_SHRINKS} times"
            ),
        }
    }
}

impl std::error::Error for Overflow {}

/// A session's conversation and its auto-compaction.
#[derive(Debug)]
pub(super) struct Conversation {
    /// The system turn and the task turn, never changed.
    head: [Message; 2],
    /// One line per folded step still listed, oldest first; the folds it no
    /// longer lists are `tally.folded` less its length.
    record: VecDeque<String>,
    /// The exchanges not folded, oldest first.
    exchanges: VecDeque<Exchange>,
    /// The most bytes a request may carry.
    window: usize,
    /// What compaction did.
    tally: Tally,
}

impl Conversation {
    /// A conversation of `system` and `task` whose requests stay within `window` bytes.
    pub(super) fn new(system: &str, task: String, window: usize) -> Self {
        Self {
            head: [
                Message::new(Role::System, system),
                Message::new(Role::User, task),
            ],
            record: VecDeque::new(),
            exchanges: VecDeque::new(),
            window,
            tally: Tally::default(),
        }
    }

    /// Appends `reply` and the turn that answered it.
    pub(super) fn push(&mut self, reply: String, answer: Answer) {
        self.exchanges.push_back(Exchange { reply, answer });
    }

    /// The report object for what compaction did.
    pub(super) fn json(&self) -> Value {
        json!({
            "runs": self.tally.runs,
            "cleared": self.tally.cleared,
            "folded": self.tally.folded,
            "shrinks": self.tally.shrinks,
            "window_bytes": self.window,
        })
    }

    /// The next request, encoded once, compacted first when it is past three
    /// quarters of the window or holds too many turns.
    ///
    /// # Errors
    ///
    /// [`Overflow::Irreducible`] when even the compacted request is over the window.
    pub(super) fn prepare(&mut self, client: &ModelClient) -> Result<Request, Overflow> {
        let mut request = client.encode(&self.render());
        if request.bytes() > quarters(self.window, 3) || self.exchanges.len() > MAX_EXCHANGES {
            request = self.compact(client, quarters(self.window, 2));
        }
        if request.bytes() > self.window {
            return Err(Overflow::Irreducible {
                len: request.bytes(),
                window: self.window,
            });
        }
        Ok(request)
    }

    /// The gateway refused a request of `len` bytes as past the model's window:
    /// the window becomes three quarters of it.
    ///
    /// # Errors
    ///
    /// [`Overflow::Shrinks`] once the window has shrunk [`MAX_SHRINKS`] times.
    pub(super) fn shrink(&mut self, len: usize) -> Result<(), Overflow> {
        if self.tally.shrinks >= MAX_SHRINKS {
            return Err(Overflow::Shrinks);
        }
        self.tally.shrinks = self.tally.shrinks.saturating_add(1);
        self.window = self.window.min(quarters(len, 3));
        Ok(())
    }

    /// Compacts toward `target` bytes and returns the request it leaves: clears
    /// old output, folds the oldest exchanges, clears what output is left, then
    /// drops the oldest record lines, each stage only while still over.
    fn compact(&mut self, client: &ModelClient, target: usize) -> Request {
        self.tally.runs = self.tally.runs.saturating_add(1);
        self.clear(KEEP_OBSERVED);
        let mut request = client.encode(&self.render());
        // A pass re-measures what the last one left: the first fold also adds
        // the record turn, which its estimate cannot see. A pass that shortens
        // nothing ends the stage, and each stage has at most one pass per item.
        for _ in 0..self.exchanges.len() {
            let before = self.exchanges.len();
            self.fold(request.bytes().saturating_sub(target));
            if self.exchanges.len() == before {
                break;
            }
            request = client.encode(&self.render());
        }
        if request.bytes() > target {
            self.clear(0);
            request = client.encode(&self.render());
        }
        for _ in 0..self.record.len() {
            let before = self.record.len();
            self.forget(request.bytes().saturating_sub(target));
            if self.record.len() == before {
                break;
            }
            request = client.encode(&self.render());
        }
        request
    }

    /// Clears the output of every observation but the newest `keep`.
    fn clear(&mut self, keep: usize) {
        let mut kept = 0_usize;
        for exchange in self.exchanges.iter_mut().rev() {
            if let Answer::Observed {
                output: ref mut output @ Some(_),
                ..
            } = exchange.answer
            {
                if kept < keep {
                    kept = kept.saturating_add(1);
                } else {
                    *output = None;
                    self.tally.cleared = self.tally.cleared.saturating_add(1);
                }
            }
        }
    }

    /// Folds the oldest exchanges into the record until their bytes cover
    /// `excess` and, past [`MAX_EXCHANGES`], until [`LOW_EXCHANGES`] remain.
    fn fold(&mut self, excess: usize) {
        let low = if self.exchanges.len() > MAX_EXCHANGES {
            LOW_EXCHANGES
        } else {
            MAX_EXCHANGES
        };
        let mut saved = 0_usize;
        while self.exchanges.len() > KEEP_EXCHANGES
            && (saved < excess || self.exchanges.len() > low)
        {
            let Some(oldest) = self.exchanges.pop_front() else {
                break;
            };
            let line = oldest.answer.record_line();
            let spent = oldest
                .reply
                .len()
                .saturating_add(oldest.answer.text_len())
                .saturating_add(TURN_FRAMING);
            saved = saved.saturating_add(spent.saturating_sub(line.len()));
            self.record.push_back(line);
            if self.record.len() > MAX_RECORD_LINES {
                self.record.pop_front();
            }
            self.tally.folded = self.tally.folded.saturating_add(1);
        }
    }

    /// Drops the oldest record lines until their bytes cover `excess`; each stays
    /// counted in `tally.folded`.
    fn forget(&mut self, excess: usize) {
        let mut saved = 0_usize;
        while saved < excess {
            let Some(line) = self.record.pop_front() else {
                break;
            };
            saved = saved.saturating_add(line.len());
        }
    }

    /// The record turn's text.
    fn record_text(&self) -> String {
        let mut lines = vec![String::from(
            "Auto-compaction record of my earlier steps, oldest first:",
        )];
        let unlisted = self.tally.folded.saturating_sub(self.record.len());
        if unlisted > 0 {
            lines.push(format!("({unlisted} earlier steps are not listed)"));
        }
        lines.extend(self.record.iter().cloned());
        lines.join("\n")
    }

    /// The conversation as the next request carries it.
    fn render(&self) -> Vec<Message> {
        let mut messages = Vec::with_capacity(
            self.exchanges
                .len()
                .saturating_mul(2)
                .saturating_add(self.head.len())
                .saturating_add(2),
        );
        messages.extend(self.head.iter().cloned());
        if self.tally.folded > 0 {
            messages.push(Message::new(Role::Assistant, self.record_text()));
            messages.push(Message::new(Role::User, CONTINUE));
        }
        for exchange in &self.exchanges {
            messages.push(Message::new(Role::Assistant, exchange.reply.clone()));
            messages.push(Message::new(Role::User, exchange.answer.text()));
        }
        messages
    }
}

#[cfg(test)]
#[path = "sim_compact.rs"]
mod sim;
