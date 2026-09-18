//! Recorded events: per-run monotonic sequencing and causal parents
//! (spec Cell 04 ordering, work package C02).
//!
//! Ordering rules encoded here:
//!
//! - **Per-run sequence assignment is total and gap-free.** Accepted
//!   events take `previous + 1`; the first accepted event of a run takes
//!   sequence 0. Gaps, duplicates and out-of-order candidates are typed
//!   refusals, never silent fills (E35).
//! - **Causal parents cross runs.** A record may cite the `(run, sequence)`
//!   pairs it was caused by; the parent list is canonicalized to sorted,
//!   duplicate-free form so equality and digests are independent of the
//!   order a crash-and-retry happened to produce (E28).
//! - **Records are bounded.** A record accepts at most
//!   [`MAX_CAUSAL_PARENTS`] causal parents (Cell 05 resource bounds).

use lgwks_std::hash;

use crate::error::{OrderingError, SequenceError};
use crate::ids::{ContentDigest, RunId, TenantId};

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// Maximum causal parents a single record may cite (Cell 05 bounds).
pub const MAX_CAUSAL_PARENTS: usize = 64;

/// A per-run, gap-free, monotonically increasing event sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct EventSequence(u64);

impl EventSequence {
    /// The sequence number of the first accepted event of a run.
    pub const FIRST: EventSequence = EventSequence(0);

    /// Builds a sequence from its raw value.
    ///
    /// Validation against a log happens through [`validate_append`]; this
    /// constructor exists for rehydrating recorded values.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw sequence number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// The next gap-free sequence number.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceError::Exhausted`] when the `u64` space is
    /// exhausted; sequence numbers never wrap (a wrapped sequence would
    /// alias history).
    pub const fn successor(self) -> Result<EventSequence, SequenceError> {
        match self.0.checked_add(1) {
            Some(next) => Ok(EventSequence(next)),
            None => Err(SequenceError::Exhausted),
        }
    }
}

/// The total, pure sequence allocator: the next gap-free sequence number
/// for a run whose last accepted event carried `previous`.
///
/// # Errors
///
/// Returns [`SequenceError::Exhausted`] when the sequence space is spent.
pub const fn assign_sequence(
    previous: Option<EventSequence>,
) -> Result<EventSequence, SequenceError> {
    match previous {
        None => Ok(EventSequence::FIRST),
        Some(last) => last.successor(),
    }
}

/// Validates that `candidate` may be appended after `previous` without a
/// silent gap or a duplicate (E35).
///
/// # Errors
///
/// Returns [`SequenceError::NotFirst`] when the log is empty and the
/// candidate is not sequence 0, [`SequenceError::Gap`] when the candidate
/// skips or reuses numbers, and [`SequenceError::Exhausted`] when the
/// previous sequence is the last representable number.
pub const fn validate_append(
    previous: Option<EventSequence>,
    candidate: EventSequence,
) -> Result<(), SequenceError> {
    match previous {
        None => {
            if candidate.0 == 0 {
                Ok(())
            } else {
                Err(SequenceError::NotFirst { got: candidate.0 })
            }
        }
        Some(last) => match last.0.checked_add(1) {
            None => Err(SequenceError::Exhausted),
            Some(expected) if expected == candidate.0 => Ok(()),
            Some(expected) => Err(SequenceError::Gap {
                expected,
                got: candidate.0,
            }),
        },
    }
}

/// A causal edge pointing at an event in some run, possibly another run
/// than the citing record's own (Cell 04: causal parents across runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct CausalRef {
    /// The run the caused-by event belongs to.
    pub run: RunId,
    /// The gap-free sequence of the caused-by event within that run.
    pub sequence: EventSequence,
}

/// The vocabulary of recorded facts the reducer folds (D01).
///
/// The set is closed by C02; extending it is a specification change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub enum EventKind {
    /// A command was accepted (initial or idempotent replay, E28).
    CommandAccepted,
    /// The run-state algebra produced a transition.
    StateTransition,
    /// An effect intent was reserved and recorded pre-dispatch.
    EffectIntentRecorded,
    /// An effect outcome was observed and persisted.
    EffectOutcomeRecorded,
    /// A terminal receipt was published.
    TerminalReceiptPublished,
}

/// A recorded fact in a run's event log.
///
/// Construction validates the causal-parent bound and canonicalizes the
/// parent list; the payload digest is computed at construction so the
/// recorded value carries its own content identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct Event {
    tenant: TenantId,
    run: RunId,
    sequence: EventSequence,
    causal_parents: Vec<CausalRef>,
    kind: EventKind,
    payload: Box<[u8]>,
    payload_digest: ContentDigest,
}

impl Event {
    /// Records an event with gap-free sequencing and canonical causal
    /// parents.
    ///
    /// The parent list is sorted and exact duplicates are collapsed; the
    /// payload digest is computed over the event kind tag and payload
    /// bytes (BLAKE3, deterministic).
    ///
    /// # Errors
    ///
    /// Returns [`OrderingError::CausalParentsExceeded`] when more than
    /// [`MAX_CAUSAL_PARENTS`] parents are supplied after collapsing.
    pub fn new(
        tenant: TenantId,
        run: RunId,
        sequence: EventSequence,
        mut causal_parents: Vec<CausalRef>,
        kind: EventKind,
        payload: Box<[u8]>,
    ) -> Result<Self, OrderingError> {
        causal_parents.sort_unstable();
        causal_parents.dedup();
        if causal_parents.len() > MAX_CAUSAL_PARENTS {
            return Err(OrderingError::CausalParentsExceeded {
                len: causal_parents.len(),
                max: MAX_CAUSAL_PARENTS,
            });
        }
        let mut hasher = hash::Hasher::new();
        hasher.update(&[kind as u8]);
        hasher.update(&payload);
        let payload_digest = ContentDigest::from_blake3(hasher.finalize());
        Ok(Self {
            tenant,
            run,
            sequence,
            causal_parents,
            kind,
            payload,
            payload_digest,
        })
    }

    /// The tenant the event belongs to.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// The run the event belongs to.
    #[must_use]
    pub const fn run(&self) -> RunId {
        self.run
    }

    /// The gap-free per-run sequence number.
    #[must_use]
    pub const fn sequence(&self) -> EventSequence {
        self.sequence
    }

    /// The canonicalized causal parents, sorted and duplicate-free.
    #[must_use]
    pub fn causal_parents(&self) -> &[CausalRef] {
        &self.causal_parents
    }

    /// The recorded fact's kind.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        self.kind
    }

    /// The recorded payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// The BLAKE3 digest over kind tag and payload bytes.
    #[must_use]
    pub const fn payload_digest(&self) -> ContentDigest {
        self.payload_digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(byte: u8) -> RunId {
        RunId::from_bytes([byte; 16])
    }

    fn tid() -> TenantId {
        TenantId::from_bytes([0xEE; 16])
    }

    fn causal(run: RunId, sequence: u64) -> CausalRef {
        CausalRef {
            run,
            sequence: EventSequence::new(sequence),
        }
    }

    #[test]
    fn sequences_are_assigned_gap_free_from_zero() {
        let first = assign_sequence(None);
        assert_eq!(first, Ok(EventSequence::FIRST));
        let second = first.and_then(EventSequence::successor);
        assert_eq!(second, Ok(EventSequence::new(1)));
        let third = second.and_then(EventSequence::successor);
        assert_eq!(third, Ok(EventSequence::new(2)));
    }

    #[test]
    fn e35_sequence_gap_is_rejected_not_silently_filled() {
        let appended = validate_append(Some(EventSequence::new(4)), EventSequence::new(6));
        assert_eq!(
            appended,
            Err(SequenceError::Gap {
                expected: 5,
                got: 6
            })
        );
    }

    #[test]
    fn e35_duplicate_and_out_of_order_appends_are_rejected() {
        let duplicate = validate_append(Some(EventSequence::new(4)), EventSequence::new(4));
        assert_eq!(
            duplicate,
            Err(SequenceError::Gap {
                expected: 5,
                got: 4
            })
        );
        let stale = validate_append(Some(EventSequence::new(4)), EventSequence::new(2));
        assert_eq!(
            stale,
            Err(SequenceError::Gap {
                expected: 5,
                got: 2
            })
        );
    }

    #[test]
    fn e35_first_sequence_must_be_zero() {
        let bad = validate_append(None, EventSequence::new(7));
        assert_eq!(bad, Err(SequenceError::NotFirst { got: 7 }));
        assert_eq!(validate_append(None, EventSequence::FIRST), Ok(()));
    }

    #[test]
    fn e35_sequence_exhaustion_is_typed_never_wrapped() {
        let last = EventSequence::new(u64::MAX);
        assert_eq!(last.successor(), Err(SequenceError::Exhausted));
        assert_eq!(assign_sequence(Some(last)), Err(SequenceError::Exhausted));
        assert_eq!(
            validate_append(Some(last), EventSequence::new(0)),
            Err(SequenceError::Exhausted)
        );
    }

    #[test]
    fn causal_parents_are_canonicalized_sorted_and_deduplicated() {
        let built = Event::new(
            tid(),
            rid(1),
            EventSequence::FIRST,
            vec![causal(rid(9), 3), causal(rid(2), 8), causal(rid(9), 3)],
            EventKind::CommandAccepted,
            Box::from(b"payload".as_slice()),
        );
        let event = crate::test_support::expect_ok(built);
        assert_eq!(
            event.causal_parents(),
            &[causal(rid(2), 8), causal(rid(9), 3)]
        );
    }

    #[test]
    fn excess_causal_parents_are_typed_refusals() {
        let too_many: Vec<CausalRef> = (0..=MAX_CAUSAL_PARENTS)
            .map(|index| causal(rid(u8::try_from(index).unwrap_or(u8::MAX)), 0))
            .collect();
        let built = Event::new(
            tid(),
            rid(1),
            EventSequence::FIRST,
            too_many,
            EventKind::StateTransition,
            Box::from([].as_slice()),
        );
        assert_eq!(
            built,
            Err(OrderingError::CausalParentsExceeded {
                len: MAX_CAUSAL_PARENTS + 1,
                max: MAX_CAUSAL_PARENTS
            })
        );
    }

    #[test]
    fn identical_content_yields_identical_payload_digests() {
        let left = Event::new(
            tid(),
            rid(1),
            EventSequence::FIRST,
            Vec::new(),
            EventKind::EffectIntentRecorded,
            Box::from(b"same".as_slice()),
        );
        let right = Event::new(
            tid(),
            rid(1),
            EventSequence::new(1),
            Vec::new(),
            EventKind::EffectIntentRecorded,
            Box::from(b"same".as_slice()),
        );
        assert_eq!(
            left.map(|event| event.payload_digest()),
            right.map(|event| event.payload_digest())
        );
    }
}
