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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use lgwks_std::json::serde::{Deserialize, Serialize};

use rocco_contracts::command::{Command, CommandAcceptance, accept};
use rocco_contracts::event::{Event, EventSequence, assign_sequence, validate_append};
use rocco_contracts::ids::{CommandId, RunId};

use crate::admission::JournalStorage;
use crate::error::RuntimeError;
use crate::payload::CommandAcceptedPayload;

/// One recorded journal entry, in the order it was recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
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
    /// Maximum retained entries.
    capacity: usize,
    /// Host-owned durable compare-and-append boundary, absent for fixtures.
    storage: Option<Arc<dyn JournalStorage>>,
    /// An uncertain append requires reopening from authoritative storage.
    write_failed: bool,
    /// Unpromoted command offsets, ordered by durable admission rather than ID.
    pending: BTreeSet<usize>,
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
            command_index: BTreeMap::new(),
            last_sequence: BTreeMap::new(),
            capacity,
            storage: None,
            write_failed: false,
            pending: BTreeSet::new(),
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
        Ok(Self { capacity, ..self })
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
            let id = command.id();
            let entry = JournalEntry::Command(command);
            self.persist(&entry)?;
            self.command_index.insert(id, index);
            self.pending.insert(index);
            self.entries.push(entry);
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
        let promoted = if event.kind() == rocco_contracts::event::EventKind::CommandAccepted {
            let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
            Some(payload.command)
        } else {
            None
        };
        let sequence = event.sequence();
        let entry = JournalEntry::Event(event);
        self.persist(&entry)?;
        self.last_sequence.insert(run, sequence);
        if let Some(command) = promoted
            && let Some(index) = self.command_index.get(&command)
        {
            self.pending.remove(index);
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Opens host-recovered entries and attaches their durable writer.
    ///
    /// The host must supply the complete committed prefix from this storage,
    /// with exclusive writer ownership. Each append compares its expected
    /// offset; a stale clone cannot append over a different history.
    ///
    /// # Errors
    /// Refuses invalid sequencing, conflicting identities and capacity overflow.
    pub fn with_storage(
        capacity: usize,
        entries: impl IntoIterator<Item = JournalEntry>,
        storage: Arc<dyn JournalStorage>,
    ) -> Result<Self, RuntimeError> {
        let mut journal = Self::new(capacity)?;
        for entry in entries {
            match entry {
                JournalEntry::Command(command) => {
                    journal.accept_command(command)?;
                }
                JournalEntry::Event(event) => journal.record(event)?,
            }
        }
        journal.storage = Some(storage);
        Ok(journal)
    }

    /// Requires a healthy durable owner before returning an admission acknowledgement.
    pub(crate) fn require_storage(&self) -> Result<(), RuntimeError> {
        self.check_writer()?;
        if self.storage.is_none() {
            return Err(RuntimeError::DurabilityUnavailable);
        }
        Ok(())
    }

    /// No execution may continue after persistence became uncertain.
    pub(crate) fn check_writer(&self) -> Result<(), RuntimeError> {
        if self.write_failed {
            return Err(RuntimeError::JournalRecoveryRequired);
        }
        Ok(())
    }

    /// Whether the accepted identity already entered reduction.
    pub(crate) fn is_promoted(&self, command: CommandId) -> bool {
        self.command_index
            .get(&command)
            .is_some_and(|index| !self.pending.contains(index))
    }

    /// Accepted inputs that have not entered reduction, in admission order.
    pub(crate) fn pending(&self) -> impl Iterator<Item = &Command> {
        self.pending.iter().filter_map(|index| {
            let entry = self.entries.get(*index)?;
            match *entry {
                JournalEntry::Command(ref command) => Some(command),
                JournalEntry::Event(_) => None,
            }
        })
    }

    /// Persists before exposing an entry to any reader or owner.
    fn persist(&mut self, entry: &JournalEntry) -> Result<(), RuntimeError> {
        if self.write_failed {
            return Err(RuntimeError::JournalRecoveryRequired);
        }
        if let Some(ref storage) = self.storage
            && let Err(error) = storage.append(self.entries.len(), entry)
        {
            self.write_failed = true;
            return Err(RuntimeError::JournalStorage(error));
        }
        Ok(())
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
