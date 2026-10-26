//! The bounded, append-only journal: the authoritative record (D04).
//!
//! "Event history is authoritative; graph and context are projections." The
//! journal is that history. Every other structure the runtime holds —
//! projection, effect table, ready queue — is derivable from it and
//! discardable at any moment.
//!
//! Two things are recorded, in one order:
//!
//! - an accepted [`Command`], once per identity, so duplicate suppression is
//!   a fold over the journal rather than a separate side table; and
//! - a recorded [`Event`], gap-free per run and carrying its own payload
//!   digest.
//!
//! The journal never grows without bound: it has a fixed entry ceiling and
//! refuses the append that would exceed it. Refusal is backpressure, not a
//! silent drop (Cell 05).

use std::collections::BTreeMap;

use rocco_contracts::command::{Command, CommandAcceptance, accept};
use rocco_contracts::event::{Event, EventSequence, assign_sequence, validate_append};
use rocco_contracts::ids::{CommandId, RunId};

use crate::error::RuntimeError;

/// One recorded journal entry, in the order it was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JournalEntry {
    /// A command identity was accepted for the first time.
    Command(Command),
    /// A fact was recorded.
    Event(Event),
}

/// The authoritative, bounded, append-only record.
///
/// Iteration order is always insertion order; the duplicate index is a
/// [`BTreeMap`], never a `HashMap`, so no operation on the journal depends on
/// hash iteration order. That is a determinism requirement, not a style
/// preference.
#[derive(Debug, Clone)]
pub struct Journal {
    /// Recorded facts in append order.
    entries: Vec<JournalEntry>,
    /// Accepted command identities mapped to entry offsets.
    command_index: BTreeMap<CommandId, usize>,
    /// Last accepted event sequence for each run.
    last_sequence: BTreeMap<RunId, EventSequence>,
    /// Disposable tenant cursor lookup, rebuilt from entries on reopen.
    pub(crate) consumer_index: crate::consumer::ConsumerIndex,
    /// Maximum retained entries.
    capacity: usize,
}

impl Journal {
    /// A journal bounded to `capacity` entries.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::JournalFull`] when `capacity` is zero: a
    /// zero-capacity journal can never record anything, which is a
    /// configuration defect rather than a runtime condition.
    pub fn new(capacity: usize) -> Result<Self, RuntimeError> {
        if capacity == 0 {
            return Err(RuntimeError::JournalFull { capacity });
        }
        Ok(Self {
            entries: Vec::new(),
            consumer_index: crate::consumer::ConsumerIndex::default(),
            command_index: BTreeMap::new(),
            last_sequence: BTreeMap::new(),
            capacity,
        })
    }

    /// Consumes the journal and returns the same history under a different
    /// entry ceiling.
    ///
    /// Recovery needs this: a journal that filled up is exactly the state a
    /// crash leaves behind, and restoring it into the same ceiling would make
    /// the runtime unable to record the reconciliation it is there to perform.
    /// No entry is copied and no ordering is disturbed.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::JournalFull`] when `capacity` is zero, or when
    /// `capacity` is smaller than the history already recorded — a ceiling
    /// below the current length is a contradiction, not a truncation.
    pub fn reopen(self, capacity: usize) -> Result<Self, RuntimeError> {
        if capacity == 0 || capacity < self.entries.len() {
            return Err(RuntimeError::JournalFull { capacity });
        }
        let consumer_index = crate::consumer::ConsumerIndex::rebuild(&self.entries);
        Ok(Self {
            capacity,
            consumer_index,
            ..self
        })
    }

    /// The number of recorded entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry ceiling.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether one more entry would exceed the ceiling.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.capacity
    }

    /// Every recorded entry, in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// The most recent entry, if any.
    #[must_use]
    pub fn last(&self) -> Option<&JournalEntry> {
        self.entries.last()
    }

    /// The number of distinct runs the journal mentions.
    #[must_use]
    pub fn run_count(&self) -> usize {
        self.last_sequence.len()
    }

    /// The command already accepted under `id`, if any.
    ///
    /// This is the value [`accept`] compares an incoming command against.
    #[must_use]
    pub fn command(&self, id: CommandId) -> Option<&Command> {
        let index = *self.command_index.get(&id)?;
        match *self.entries.get(index)? {
            JournalEntry::Command(ref command) => Some(command),
            JournalEntry::Event(_) => None,
        }
    }

    /// The last recorded sequence for `run`, if it has any events.
    #[must_use]
    pub fn last_sequence(&self, run: RunId) -> Option<EventSequence> {
        self.last_sequence.get(&run).copied()
    }

    /// The next gap-free sequence number for `run`.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Sequence`] when the sequence space is spent.
    pub fn next_sequence(&self, run: RunId) -> Result<EventSequence, RuntimeError> {
        Ok(assign_sequence(self.last_sequence(run))?)
    }

    /// Applies the duplicate-suppression check and records the command if it
    /// is an initial sighting.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Command`] for a same-identity/different-content
    /// retry or a cross-tenant retry, and [`RuntimeError::JournalFull`] when
    /// the append would exceed the ceiling.
    pub fn accept_command(&mut self, command: Command) -> Result<CommandAcceptance, RuntimeError> {
        let acceptance = accept(self.command(command.id()), &command)?;
        if acceptance == CommandAcceptance::Initial {
            self.reserve_one()?;
            let index = self.entries.len();
            self.command_index.insert(command.id(), index);
            self.append_indexed(JournalEntry::Command(command));
        }
        Ok(acceptance)
    }

    /// Records an event after validating gap-free sequencing for its run.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Sequence`] when the candidate would introduce
    /// a gap or reuse a number, and [`RuntimeError::JournalFull`] when the
    /// append would exceed the ceiling.
    pub fn record(&mut self, event: Event) -> Result<(), RuntimeError> {
        let run = event.run();
        validate_append(self.last_sequence(run), event.sequence())?;
        self.reserve_one()?;
        self.last_sequence.insert(run, event.sequence());
        self.append_indexed(JournalEntry::Event(event));
        Ok(())
    }

    /// Publishes the derived descriptor and the accepted fact together.
    fn append_indexed(&mut self, entry: JournalEntry) {
        self.consumer_index.record(&entry, self.entries.len());
        self.entries.push(entry);
    }

    /// Refuses an append once the configured entry bound is reached.
    fn reserve_one(&self) -> Result<(), RuntimeError> {
        if self.is_full() {
            return Err(RuntimeError::JournalFull {
                capacity: self.capacity,
            });
        }
        Ok(())
    }
}
