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
//!
//! A journal opened with [`Journal::open`] is durable (larry#75): each entry
//! the index takes is staged, and [`Journal::sync`] puts every staged entry in
//! its `lgwks_bot::journal::RecordJournal` file in one write and one sync. The
//! runtime syncs before every owner reach and before every tick returns, so no
//! fact an owner or a caller saw is ever missing from the file; the next open
//! replays it through the same acceptance rules. [`Journal::new`] stays
//! memory-only, and a clone is always memory-only: the file has one writer.

use std::collections::BTreeMap;
use std::path::Path;

mod durable;

use larry_contracts::command::{Command, CommandAcceptance, accept};
use larry_contracts::event::{Event, EventSequence, assign_sequence, validate_append};
use larry_contracts::ids::{CommandId, RunId};

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
#[derive(Debug)]
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
    /// The file every entry reaches before the index, when opened durable.
    durable: Option<durable::Backing>,
    /// Set by the first refused [`Journal::sync`] (larry#209): the file's
    /// outcome is unknown, so this handle takes no new entry. A restore over
    /// the file is the only way to write again.
    refused: bool,
    /// Whether the next [`Journal::sync`] appends and then refuses as a lost
    /// acknowledgment (larry#182), for the in-crate admission falsifier.
    #[cfg(test)]
    lose_sync: bool,
}

impl Clone for Journal {
    /// The same history, memory-only: the file keeps its one writer, so a
    /// clone never appends to it.
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            command_index: self.command_index.clone(),
            last_sequence: self.last_sequence.clone(),
            consumer_index: self.consumer_index.clone(),
            capacity: self.capacity,
            durable: None,
            refused: self.refused,
            #[cfg(test)]
            lose_sync: self.lose_sync,
        }
    }
}

impl Journal {
    /// Opens the durable journal at `path`, bounded to `capacity` entries,
    /// creating the file when it is absent and replaying every entry it holds.
    ///
    /// Each replayed entry passes the same rules a live append does: a command
    /// identity is accepted once, events are gap-free per run, and digests are
    /// re-derived from the parts. A torn tail a killed writer left is taken
    /// back by the store before replay.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Durable`] when the store refuses (another writer holds
    /// the file, committed bytes break the chain, a ceiling, the device),
    /// [`RuntimeError::Codec`] for a record that does not decode,
    /// [`RuntimeError::DurableRepeat`] for a command recorded twice,
    /// [`RuntimeError::Sequence`] for an event out of order, and
    /// [`RuntimeError::JournalFull`] when the file holds more than `capacity`.
    pub fn open(path: &Path, capacity: usize) -> Result<Self, RuntimeError> {
        let mut journal = Self::new(capacity)?;
        let (backing, entries) = durable::Backing::open(path)?;
        for (index, entry) in entries.into_iter().enumerate() {
            match entry {
                JournalEntry::Command(command) => {
                    if journal.accept_command(command)? != CommandAcceptance::Initial {
                        return Err(RuntimeError::DurableRepeat { index });
                    }
                }
                JournalEntry::Event(event) => journal.record(event)?,
            }
        }
        journal.durable = Some(backing);
        Ok(journal)
    }

    /// Whether this journal is backed by a file (test inspection).
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_durable(&self) -> bool {
        self.durable.is_some()
    }

    /// How many entries the index holds that the file has not synced yet;
    /// always zero for a memory-only journal (test inspection).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn unsynced(&self) -> usize {
        match self.durable.as_ref() {
            Some(backing) => backing.unsynced(),
            None => 0,
        }
    }

    /// Puts every entry taken since the last sync in the file, in one write
    /// and one sync. A memory-only journal, or one with nothing staged, is a
    /// no-op that touches no device.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Durable`] when the store refuses or cannot say whether
    /// the bytes landed, and [`RuntimeError::DurabilityShort`] when its promise
    /// is under a crash. Either one latches the journal: every later
    /// [`Journal::accept_command`] and [`Journal::record`] in this process is
    /// refused with [`RuntimeError::Durable`] before the index moves, and only a
    /// restore over the file writes again (larry#209). A journal built by
    /// [`Journal::lost_acknowledgement`] appends its staged records and then
    /// reports the lost acknowledgment a real store produces (larry#182).
    pub(crate) fn sync(&mut self) -> Result<(), RuntimeError> {
        match self.durable.as_mut() {
            Some(backing) => {
                if let Err(error) = backing.sync() {
                    self.refused = true;
                    return Err(error);
                }
                #[cfg(test)]
                if self.lose_sync {
                    self.lose_sync = false;
                    self.refused = true;
                    return Err(RuntimeError::Durable(
                        lgwks_bot::journal::JournalError::OutcomeUnknown {
                            cause: std::io::Error::other(
                                "injected: the append's acknowledgment was lost",
                            ),
                        },
                    ));
                }
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// The refusal every write takes once an earlier sync was refused (larry#209).
    ///
    /// The file's outcome is unknown after a refused sync, and the store latches
    /// its own handle for the same reason, so no later append of this process
    /// may move the index or the file. The next open replays the truth.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Durable`] with [`lgwks_bot::journal::JournalError::OutcomeUnknown`]
    /// while the journal is latched.
    fn refuse_if_refused(&self) -> Result<(), RuntimeError> {
        if self.refused {
            return Err(RuntimeError::Durable(
                lgwks_bot::journal::JournalError::OutcomeUnknown {
                    cause: std::io::Error::other(
                        "an earlier sync was refused; reopen the journal to replay the file",
                    ),
                },
            ));
        }
        Ok(())
    }

    /// A journal over the same file that reports the next sync as a lost
    /// acknowledgment, after appending what it staged (larry#182).
    ///
    /// The refusal a real store produces when bytes reach the disk and the
    /// confirmation does not reach the caller. Test-only: it exists so the
    /// admission ordering can be falsified against the exact error a store
    /// gives, never a made-up one.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn lost_acknowledgement(mut self) -> Self {
        self.lose_sync = true;
        self
    }

    /// Whether opening took back an append a killed writer never finished
    /// (test inspection).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn torn_tail_repaired(&self) -> bool {
        self.durable
            .as_ref()
            .is_some_and(durable::Backing::torn_tail_repaired)
    }

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
            durable: None,
            refused: false,
            #[cfg(test)]
            lose_sync: false,
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
    /// retry or a cross-tenant retry, [`RuntimeError::JournalFull`] when
    /// the append would exceed the ceiling, and the refusal of
    /// [`Journal::sync`] while an earlier sync is refused (larry#209). A
    /// refused call moves nothing, not even for a retry of a journaled command.
    pub fn accept_command(&mut self, command: Command) -> Result<CommandAcceptance, RuntimeError> {
        self.refuse_if_refused()?;
        let acceptance = accept(self.command(command.id()), &command)?;
        if acceptance == CommandAcceptance::Initial {
            self.reserve_one()?;
            let id = command.id();
            let index = self.entries.len();
            self.append_indexed(JournalEntry::Command(command))?;
            self.command_index.insert(id, index);
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
        self.refuse_if_refused()?;
        let run = event.run();
        validate_append(self.last_sequence(run), event.sequence())?;
        self.reserve_one()?;
        let sequence = event.sequence();
        self.append_indexed(JournalEntry::Event(event))?;
        self.last_sequence.insert(run, sequence);
        Ok(())
    }

    /// Stages the accepted fact for the file first, when there is one, then
    /// publishes the derived descriptor and the fact together.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Codec`] when the entry does not encode; the index has
    /// not taken the entry.
    fn append_indexed(&mut self, entry: JournalEntry) -> Result<(), RuntimeError> {
        if let Some(backing) = self.durable.as_mut() {
            backing.stage(&entry)?;
        }
        self.consumer_index.record(&entry, self.entries.len());
        self.entries.push(entry);
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
