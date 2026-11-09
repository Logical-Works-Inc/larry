//! The journal's file (larry#75): every accepted command and recorded event is
//! staged as the in-memory index takes it, and the staged records reach an
//! `lgwks_bot::journal::RecordJournal` in one write and one sync (group commit)
//! before the runtime reaches an owner and before a tick returns. A kill -9
//! therefore loses no fact an owner or a caller ever saw, and the next open
//! replays every one.
//!
//! Source: lgwks_bot 8.1 `journal::records` (logicalworks-crates#875):
//! compare-and-append, a `ProcessCrash` promise earned per append, a torn tail
//! taken back on open, and committed bytes that do not re-derive the chain
//! refused. A record holds an entry's parts and never its digests: replay
//! rebuilds each entry through `Command::new` and `Event::new`, so a digest
//! written into the file is re-derived, never trusted (E55).

use std::path::Path;

use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::event::{CausalRef, Event, EventKind, EventSequence};
use larry_contracts::ids::{CommandId, RunId, TenantId};
use lgwks_bot::journal::{DurabilityPromise, RecordJournal};
use lgwks_std::json::serde::{Deserialize, Serialize};

use super::JournalEntry;
use crate::error::RuntimeError;

/// An entry's parts as the file holds them, borrowed from the entry so an
/// append copies nothing it does not encode.
#[derive(Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
enum Written<'a> {
    /// An accepted command, without its envelope digest.
    Command {
        /// The caller-retained identity.
        id: CommandId,
        /// The envelope's tenant.
        tenant: TenantId,
        /// The canonical causal citations.
        parents: &'a [CausalRef],
        /// The command's action.
        kind: CommandKind,
        /// The command's payload bytes.
        payload: &'a [u8],
    },
    /// A recorded event, without its payload digest.
    Event {
        /// The tenant owning the fact.
        tenant: TenantId,
        /// The run whose log holds it.
        run: RunId,
        /// Its gap-free position in the run.
        sequence: EventSequence,
        /// The canonical causal citations.
        parents: &'a [CausalRef],
        /// The closed vocabulary discriminator.
        kind: EventKind,
        /// The payload bytes the kind interprets.
        payload: &'a [u8],
    },
}

/// The same parts read back, owned, before the contract constructors validate
/// them again.
#[derive(Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
enum Stored {
    /// An accepted command.
    Command {
        /// The caller-retained identity.
        id: CommandId,
        /// The envelope's tenant.
        tenant: TenantId,
        /// The causal citations, canonicalized again on rebuild.
        parents: Vec<CausalRef>,
        /// The command's action.
        kind: CommandKind,
        /// The command's payload bytes.
        payload: Vec<u8>,
    },
    /// A recorded event.
    Event {
        /// The tenant owning the fact.
        tenant: TenantId,
        /// The run whose log holds it.
        run: RunId,
        /// Its position, validated against the run's log on replay.
        sequence: EventSequence,
        /// The causal citations, canonicalized again on rebuild.
        parents: Vec<CausalRef>,
        /// The closed vocabulary discriminator.
        kind: EventKind,
        /// The payload bytes the kind interprets.
        payload: Vec<u8>,
    },
}

/// The encoded record for `entry`.
fn encode(entry: &JournalEntry) -> Result<Vec<u8>, RuntimeError> {
    let written = match *entry {
        JournalEntry::Command(ref command) => Written::Command {
            id: command.id(),
            tenant: command.tenant(),
            parents: command.causal_parents(),
            kind: command.content().kind(),
            payload: command.content().payload(),
        },
        JournalEntry::Event(ref event) => Written::Event {
            tenant: event.tenant(),
            run: event.run(),
            sequence: event.sequence(),
            parents: event.causal_parents(),
            kind: event.kind(),
            payload: event.payload(),
        },
    };
    Ok(lgwks_std::json::to_vec(&written)?)
}

/// The entry `bytes` hold, rebuilt through the contract constructors.
fn decode(bytes: &[u8]) -> Result<JournalEntry, RuntimeError> {
    let stored: Stored = lgwks_std::json::from_slice(bytes)?;
    Ok(match stored {
        Stored::Command {
            id,
            tenant,
            parents,
            kind,
            payload,
        } => JournalEntry::Command(Command::new(
            id,
            tenant,
            parents,
            CommandContent::new(kind, payload.into_boxed_slice()),
        )?),
        Stored::Event {
            tenant,
            run,
            sequence,
            parents,
            kind,
            payload,
        } => JournalEntry::Event(Event::new(
            tenant,
            run,
            sequence,
            parents,
            kind,
            payload.into_boxed_slice(),
        )?),
    })
}

/// The open journal file behind a [`super::Journal`].
#[derive(Debug)]
pub(super) struct Backing {
    /// The file, locked to this handle for its whole life.
    store: RecordJournal<Vec<u8>>,
    /// Records the index took that the file has not synced yet, in order.
    staged: Vec<Vec<u8>>,
}

impl Backing {
    /// Opens the file at `path` and returns it with every entry it holds, in
    /// commit order, each rebuilt through its contract constructor.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Durable`] when the store refuses (held by another
    /// writer, committed bytes that break the chain, a ceiling, the device),
    /// and [`RuntimeError::Codec`] or [`RuntimeError::Ordering`] for a record
    /// that does not rebuild.
    pub(super) fn open(path: &Path) -> Result<(Self, Vec<JournalEntry>), RuntimeError> {
        let store = RecordJournal::<Vec<u8>>::open(path)?;
        let mut entries = Vec::with_capacity(store.committed());
        for item in store.replay()? {
            entries.push(decode(item?.record())?);
        }
        let staged = Vec::new();
        Ok((Self { store, staged }, entries))
    }

    /// Encodes `entry` and stages it behind every record staged before it.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Codec`] when the entry does not encode; nothing is
    /// staged and the index has not taken the entry.
    pub(super) fn stage(&mut self, entry: &JournalEntry) -> Result<(), RuntimeError> {
        self.staged.push(encode(entry)?);
        Ok(())
    }

    /// How many staged records the file has not synced.
    #[cfg(test)]
    pub(super) const fn unsynced(&self) -> usize {
        self.staged.len()
    }

    /// Appends every staged record after the tail this handle holds, in one
    /// write and one sync, and returns once the store has synced them all.
    /// Nothing staged is a no-op that touches no device.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Durable`] when the store refuses or cannot say whether
    /// the bytes landed, and [`RuntimeError::DurabilityShort`] when an
    /// acknowledgment promises less than a process crash. The records stay
    /// staged on a store refusal, so every later sync refuses as well: a fact
    /// the file could not take is never reported synced.
    pub(super) fn sync(&mut self) -> Result<(), RuntimeError> {
        if self.staged.is_empty() {
            return Ok(());
        }
        let tail = self.store.tail();
        let acks = self.store.compare_and_append_all(tail, &self.staged)?;
        // The store took them: staging them again would write them twice.
        self.staged.clear();
        if let Some(short) = acks
            .iter()
            .find(|ack| !ack.promise().meets(DurabilityPromise::ProcessCrash))
        {
            return Err(RuntimeError::DurabilityShort {
                promised: short.promise(),
            });
        }
        Ok(())
    }

    /// Whether opening took back an append a killed writer never finished.
    #[cfg(test)]
    pub(super) fn torn_tail_repaired(&self) -> bool {
        self.store.torn_tail_repaired()
    }
}

#[cfg(test)]
mod tests {
    use larry_contracts::command::{Command, CommandContent, CommandKind};
    use larry_contracts::event::{Event, EventKind, EventSequence};
    use larry_contracts::ids::{CommandId, RunId, TenantId};

    use lgwks_bot::journal::{JournalError, RecordJournal};

    use super::super::Journal;
    use super::{JournalEntry, decode, encode};
    use crate::error::RuntimeError;

    /// Fallible test outcome.
    type Outcome = Result<(), Box<dyn std::error::Error>>;

    /// One command and one event over identities derived from `seed`.
    fn pair(seed: u8) -> Result<[JournalEntry; 2], RuntimeError> {
        let tenant = TenantId::from_bytes([seed.saturating_add(1); 16]);
        let command = Command::new(
            CommandId::from_bytes([seed.saturating_add(2); 16]),
            tenant,
            Vec::new(),
            CommandContent::new(CommandKind::Submit, vec![seed; 3].into_boxed_slice()),
        )?;
        let event = Event::new(
            tenant,
            RunId::from_bytes([seed.saturating_add(3); 16]),
            EventSequence::new(u64::from(seed)),
            Vec::new(),
            EventKind::CommandAccepted,
            vec![seed, 7].into_boxed_slice(),
        )?;
        Ok([JournalEntry::Command(command), JournalEntry::Event(event)])
    }

    #[test]
    fn sim_every_entry_round_trips_through_its_record_for_every_seed() -> Outcome {
        for seed in 0_u8..64 {
            for entry in pair(seed)? {
                let back = decode(&encode(&entry)?)?;
                assert_eq!(back, entry, "seed {seed}: the record rebuilds the entry");
            }
        }
        Ok(())
    }

    #[test]
    fn a_record_carries_no_digest_and_a_tampered_part_rederives_another() -> Outcome {
        let [command, event] = pair(9)?;
        for entry in [&command, &event] {
            let text = String::from_utf8(encode(entry)?)?;
            assert!(!text.contains("digest"), "no digest is written: {text}");
        }
        let text = String::from_utf8(encode(&command)?)?;
        let tampered = text.replacen("[9,9,9]", "[9,9,8]", 1);
        assert_ne!(tampered, text, "the payload is in the record");
        let JournalEntry::Command(ref original) = command else {
            return Err("pair builds a command first".into());
        };
        let JournalEntry::Command(back) = decode(tampered.as_bytes())? else {
            return Err("a command record rebuilds a command".into());
        };
        assert_ne!(
            back.envelope_digest(),
            original.envelope_digest(),
            "the digest follows the bytes on disk, never a stored claim"
        );
        Ok(())
    }

    /// A submit command numbered `n` under the tenant `seed` derives.
    fn command(seed: u8, n: u8) -> Result<Command, RuntimeError> {
        Ok(Command::new(
            CommandId::from_bytes([n; 16]),
            TenantId::from_bytes([seed.saturating_add(1); 16]),
            Vec::new(),
            CommandContent::new(CommandKind::Submit, vec![seed, n].into_boxed_slice()),
        )?)
    }

    /// A journal file for `name` and `seed`, absent when the drive starts.
    fn fresh(name: &str, seed: u8) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!(
            "larry-durable-{}-{name}-{seed}.journal",
            std::process::id()
        ));
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
            Err(error) => Err(error.into()),
        }
    }

    /// One seeded drive of the ceiling, staging and corruption rules.
    fn ceiling_drive(seed: u8) -> Result<(u64, usize), Box<dyn std::error::Error>> {
        let path = fresh("ceiling", seed)?;
        let mut journal = Journal::open(&path, 1)?;
        assert!(journal.is_durable(), "open attaches the file");
        assert!(!journal.clone().is_durable(), "a clone is memory-only");
        journal.accept_command(command(seed, 28)?)?;
        assert_eq!(journal.unsynced(), 1, "an accepted entry is staged");
        assert_eq!(std::fs::metadata(&path)?.len(), 0, "staging writes nothing");
        journal.sync()?;
        assert_eq!(journal.unsynced(), 0, "one sync takes every staged entry");
        let length = std::fs::metadata(&path)?.len();
        assert!(length > 0, "seed {seed}: the sync reached the file");
        assert!(
            matches!(
                journal.accept_command(command(seed, 29)?),
                Err(RuntimeError::JournalFull { capacity: 1 })
            ),
            "seed {seed}: the ceiling refuses the second command"
        );
        journal.sync()?;
        assert_eq!(
            std::fs::metadata(&path)?.len(),
            length,
            "seed {seed}: a refused append writes nothing"
        );
        drop(journal);
        let mut wide = Journal::open(&path, 3)?;
        assert!(
            !wide.torn_tail_repaired(),
            "a clean close leaves no torn tail"
        );
        wide.accept_command(command(seed, 29)?)?;
        wide.sync()?;
        // Staged and never synced: a crash here loses an entry nobody saw.
        wide.accept_command(command(seed, 30)?)?;
        assert_eq!(wide.unsynced(), 1, "the third entry is only staged");
        drop(wide);
        assert!(
            matches!(
                Journal::open(&path, 1),
                Err(RuntimeError::JournalFull { capacity: 1 })
            ),
            "seed {seed}: a file over the ceiling is refused"
        );
        assert_eq!(
            Journal::open(&path, 2)?.len(),
            2,
            "the refusal kept it whole"
        );

        // A partial frame at the tail is taken back and reported.
        let whole = std::fs::read(&path)?;
        let torn = usize::from(seed % 40).saturating_add(1);
        let mut appended = whole.clone();
        appended.extend_from_slice(whole.get(..torn).ok_or("the file holds a frame")?);
        std::fs::write(&path, &appended)?;
        let repaired = Journal::open(&path, 8)?;
        assert!(
            repaired.torn_tail_repaired(),
            "seed {seed}: the torn tail is reported"
        );
        assert_eq!(
            repaired.len(),
            2,
            "seed {seed}: every synced entry survives"
        );
        drop(repaired);

        // A record repeated by another writer of the same format.
        let mut store = RecordJournal::<Vec<u8>>::open(&path)?;
        let first = store.replay()?.next().ok_or("the file holds a record")??;
        let repeated = first.record().clone();
        let tail = store.tail();
        store.compare_and_append(tail, &repeated)?;
        drop(store);
        assert!(
            matches!(
                Journal::open(&path, 8),
                Err(RuntimeError::DurableRepeat { index: 2 })
            ),
            "seed {seed}: a repeated command record is refused"
        );

        // One flipped committed byte breaks the chain and is never trimmed.
        let mut damaged = std::fs::read(&path)?;
        let middle = damaged.len().checked_div(3).ok_or("a non-empty file")?;
        let byte = damaged.get_mut(middle).ok_or("a byte to flip")?;
        *byte ^= 0x01;
        std::fs::write(&path, &damaged)?;
        let refused = Journal::open(&path, 8);
        assert!(
            matches!(
                refused,
                Err(RuntimeError::Durable(JournalError::Corrupt(_)) | RuntimeError::Codec(_))
            ),
            "seed {seed}: a flipped byte is refused: {refused:?}"
        );
        let kept = std::fs::read(&path)?.len();
        assert_eq!(kept, damaged.len(), "seed {seed}: the file is as it was");
        std::fs::remove_file(&path)?;
        Ok((length, kept))
    }

    /// larry#75: the entry ceiling refuses before the file grows; staging
    /// writes nothing and one sync takes every staged entry; an unsynced entry
    /// never reaches the file; a torn tail is taken back and reported; a file
    /// over a ceiling, a repeated record and a flipped byte are each refused.
    /// Source: write-ahead-log group commit and recovery (crates#875).
    #[test]
    fn sim_durable_capacity_refuses_before_the_file_grows() -> Outcome {
        for seed in 0_u8..8 {
            assert_eq!(ceiling_drive(seed)?, ceiling_drive(seed)?, "seed {seed}");
        }
        Ok(())
    }

    #[test]
    fn malformed_records_are_typed_codec_refusals() {
        for bytes in [
            &b""[..],
            b"{",
            b"{\"Command\":{}}",
            b"{\"Other\":{}}",
            b"[1,2]",
        ] {
            assert!(
                matches!(decode(bytes), Err(RuntimeError::Codec(_))),
                "{bytes:?} is a codec refusal"
            );
        }
    }
}
