//! Freshness at dispatch: the half of authorization that is not reproducible.
//!
//! D48 splits authorization in two, and requires the split to stay visible.
//! The first half is [`crate::gate::authorize`] — a pure reducer over pinned
//! inputs, recorded with the intent so that replaying the journal re-derives
//! the identical decision (R71, E55). The second half is *freshness*, and it
//! cannot be pinned at all: whether authority still stands is a fact about
//! **now**, held by the authority source, and yesterday's recorded decision
//! is exactly the evidence that must not be trusted today.
//!
//! [`RevocationFence`] is that second half. It is live host state — the
//! runtime never infers it from the journal and never records it as a
//! decision — and it is consulted at the one point where an effect would
//! reach its owner to *change* something.
//!
//! # The three facts a fence holds
//!
//! 1. **A security epoch** ([`SecurityEpoch`]). Revocation's linearization
//!    point. Moving the epoch forward withdraws every authorization admitted
//!    below it, at one total-order instant. The epoch only ever moves
//!    forward: [`RevocationFence::authenticate`] raises it to whatever the
//!    authority source reports and absorbs a replayed older snapshot instead
//!    of obeying it (E71).
//! 2. **A revoked set.** Named operations whose authority has been withdrawn
//!    individually. Revocation is idempotent and one-way.
//! 3. **An offline exposure bound.** How many *mutating* reaches may happen
//!    without the authority source having been re-contacted. Freshness is
//!    established by [`RevocationFence::authenticate`] and by nothing else.
//!    A suspended process that wakes up holds a staler fence, not a fresher
//!    one, and a rolled-back clock cannot recharge the bound, because the
//!    bound is a count and not a duration.
//!
//! The fence reads no clock. That is a property, not an omission: an
//! authority check that consulted time could be extended by time, and the
//! three facts above are exactly the ones that must not be.
//!
//! # What the fence deliberately does not gate
//!
//! It does not gate *observation*. An effect that had already been dispatched
//! when revocation landed must still be resolvable, because fencing the
//! status query would leave an external effect permanently unlabelled — a
//! worse outcome than the query it prevents. Revocation stops new external
//! change; it does not stop the runtime from finding out what already
//! happened. That is the "already-dispatched treatment" D48 asks to be
//! defined, and it is enforced structurally rather than by convention:
//! [`RevocationFence::check`] is called from exactly one place in the
//! runtime, the mutating reach, and never from the observing one.

use core::fmt;
use std::collections::BTreeSet;

use rocco_contracts::authority::SecurityEpoch;
use rocco_contracts::effect::EffectIntent;
use rocco_contracts::ids::OperationId;

/// Why the fence refused to let an effect reach its owner.
///
/// Each variant is a distinct, separately reportable fact: an operator
/// deciding what to do about a fenced effect needs to know whether it was
/// withdrawn by name, left behind by a generation change, or held back
/// because the authority source has gone quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FenceRefusal {
    /// The operation's authority was withdrawn by name.
    Revoked {
        /// The withdrawn operation.
        operation: OperationId,
    },
    /// The effect was admitted below the fence's current security epoch, so
    /// its authorization belongs to a generation the authority source has
    /// left behind (E71).
    StaleEpoch {
        /// The generation the reservation was admitted under.
        admitted: SecurityEpoch,
        /// The generation the fence now stands at.
        current: SecurityEpoch,
    },
    /// The authority source has not been reached for `bound` mutating
    /// reaches. Offline authority is spent and the fence fails closed.
    OfflineExposureExhausted {
        /// The configured offline exposure ceiling.
        bound: u32,
    },
}

impl fmt::Display for FenceRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Revoked { operation } => {
                write!(f, "operation {operation} was revoked by name")
            }
            Self::StaleEpoch { admitted, current } => write!(
                f,
                "admitted at {admitted} but the fence stands at {current}: stale authority"
            ),
            Self::OfflineExposureExhausted { bound } => write!(
                f,
                "offline authority spent: {bound} reaches without the authority source"
            ),
        }
    }
}

impl std::error::Error for FenceRefusal {}

/// A refused movement of the security epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FenceError {
    /// The epoch is at `u64::MAX` and cannot advance. Refused rather than
    /// wrapped: a wrapped epoch would alias a withdrawn generation with a
    /// live one, re-authorizing effects that revocation had withdrawn.
    EpochExhausted,
}

impl fmt::Display for FenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EpochExhausted => {
                write!(f, "security epoch space is exhausted; it never wraps")
            }
        }
    }
}

impl std::error::Error for FenceError {}

/// The live freshness state consulted at dispatch (R87, D48, E71).
///
/// Host-owned: the authority layer revokes, advances and authenticates
/// through this type; the runtime only reads it. Nothing here is folded from
/// the journal, and nothing here is written to it, because freshness is not
/// a recorded fact — it is a fact about the present that a record cannot
/// carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationFence {
    epoch: SecurityEpoch,
    revoked: BTreeSet<OperationId>,
    offline_bound: u32,
    offline_spent: u32,
}

impl RevocationFence {
    /// A fence standing at `epoch` that permits `offline_bound` mutating
    /// reaches without a fresh authentication.
    ///
    /// A bound of zero is a kill switch, not a mistake: no mutating reach is
    /// ever permitted, and [`RevocationFence::authenticate`] does not lift
    /// it, because the bound is a ceiling on reaches-between-authentications
    /// rather than a credit that authentication hands out. An operator who
    /// needs to stop every outbound effect sets it to zero and can rely on
    /// the fence refusing even a freshly authenticated dispatch.
    #[must_use]
    pub const fn new(epoch: SecurityEpoch, offline_bound: u32) -> Self {
        Self {
            epoch,
            revoked: BTreeSet::new(),
            offline_bound,
            offline_spent: 0,
        }
    }

    /// The generation the fence currently stands at.
    #[must_use]
    pub const fn epoch(&self) -> SecurityEpoch {
        self.epoch
    }

    /// The configured offline exposure ceiling.
    #[must_use]
    pub const fn offline_bound(&self) -> u32 {
        self.offline_bound
    }

    /// How much offline authority has been spent since the last
    /// authentication.
    #[must_use]
    pub const fn offline_spent(&self) -> u32 {
        self.offline_spent
    }

    /// Whether `operation` has been withdrawn by name.
    #[must_use]
    pub fn is_revoked(&self, operation: OperationId) -> bool {
        self.revoked.contains(&operation)
    }

    /// Withdraws `operation`'s authority by name. Idempotent.
    ///
    /// Returns whether this call was the one that revoked it, so a caller
    /// can distinguish a new withdrawal from a repeated one.
    ///
    /// The withdrawal is one-way for the life of the fence. There is no
    /// `unrevoke`, because restoring authority is a *new* authorization with
    /// its own pinned inputs and its own reservation — not the erasure of a
    /// withdrawal, which would let a replay of the pre-revocation state
    /// resurrect the effect.
    pub fn revoke(&mut self, operation: OperationId) -> bool {
        self.revoked.insert(operation)
    }

    /// Moves the fence to the next epoch: revocation's linearization point.
    ///
    /// Every effect admitted below the new epoch stops being dispatchable at
    /// that instant, and no other state changes. Offline exposure is *not*
    /// reset: a local epoch bump is a withdrawal, not evidence that the
    /// authority source was reached.
    ///
    /// # Errors
    ///
    /// Returns [`FenceError::EpochExhausted`] at `u64::MAX`.
    pub fn advance_epoch(&mut self) -> Result<SecurityEpoch, FenceError> {
        self.epoch = self.epoch.successor().ok_or(FenceError::EpochExhausted)?;
        Ok(self.epoch)
    }

    /// Establishes freshness: the authority source was reached and reports
    /// the generation it currently stands at.
    ///
    /// The fence moves to the later of the two values and never back, so a
    /// replayed old device, grant or policy snapshot is absorbed rather than
    /// obeyed, and stale evidence never independently authorizes execution
    /// (E71). Offline exposure resets, because this call *is* the authority
    /// source being reached — that is the only thing that resets it.
    ///
    /// Returns the generation the fence now stands at.
    #[must_use = "the returned epoch is the generation the fence now stands at"]
    pub fn authenticate(&mut self, reported: SecurityEpoch) -> SecurityEpoch {
        self.epoch = self.epoch.later_of(reported);
        self.offline_spent = 0;
        self.epoch
    }

    /// Whether the fence would let `intent` reach its owner to change
    /// something.
    ///
    /// The checks run in a fixed order — named revocation, then the epoch,
    /// then the offline bound — so the reported refusal is as deterministic
    /// as the reducer's denial reason, and the most specific fact about this
    /// effect is named first.
    ///
    /// Crate-internal on purpose. The host withdraws authority and
    /// re-authenticates; it does not get to pre-clear a reach. The only way
    /// to learn that a reach was refused is to make it and read
    /// [`TickReport::fenced`](crate::runtime::TickReport::fenced), which
    /// keeps the enforcement point single and keeps a would-be caller from
    /// substituting its own check for the runtime's.
    ///
    /// # Errors
    ///
    /// Returns the first-failing [`FenceRefusal`].
    pub(crate) fn check(&self, intent: &EffectIntent) -> Result<(), FenceRefusal> {
        if self.revoked.contains(&intent.operation) {
            return Err(FenceRefusal::Revoked {
                operation: intent.operation,
            });
        }
        if self.epoch.is_later_than(intent.admission_epoch) {
            return Err(FenceRefusal::StaleEpoch {
                admitted: intent.admission_epoch,
                current: self.epoch,
            });
        }
        if self.offline_spent >= self.offline_bound {
            return Err(FenceRefusal::OfflineExposureExhausted {
                bound: self.offline_bound,
            });
        }
        Ok(())
    }

    /// Records that one mutating reach happened without the authority source
    /// having been re-contacted, consuming one unit of offline authority.
    ///
    /// Crate-internal: exposure is spent by the runtime actually dispatching,
    /// never by a caller asserting that a dispatch happened. Saturating
    /// rather than wrapping, because a wrapped counter would hand back
    /// exposure that was already spent.
    pub(crate) fn spend_offline(&mut self) {
        self.offline_spent = self.offline_spent.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocco_contracts::effect::ReconciliationStrategy;
    use rocco_contracts::ids::{EffectId, RunId, TenantId};

    fn operation(byte: u8) -> OperationId {
        OperationId::from_bytes([byte; 16])
    }

    fn admitted_at(operation: OperationId, epoch: SecurityEpoch) -> EffectIntent {
        EffectIntent {
            id: EffectId::from_bytes(*operation.as_bytes()),
            operation,
            run: RunId::from_bytes([0x11; 16]),
            tenant: TenantId::from_bytes([0x22; 16]),
            strategy: ReconciliationStrategy::Queryable,
            admission_epoch: epoch,
        }
    }

    fn fence(bound: u32) -> RevocationFence {
        RevocationFence::new(SecurityEpoch::INITIAL, bound)
    }

    #[test]
    fn e71_a_fresh_fence_at_the_admission_epoch_admits_the_reach() {
        let fence = fence(4);
        assert_eq!(
            fence.check(&admitted_at(operation(1), SecurityEpoch::INITIAL)),
            Ok(())
        );
        assert_eq!(fence.epoch(), SecurityEpoch::INITIAL);
        assert_eq!(fence.offline_spent(), 0);
        assert!(!fence.is_revoked(operation(1)));
    }

    #[test]
    fn e71_revocation_by_name_is_one_way_and_idempotent() {
        let mut fence = fence(4);
        let intent = admitted_at(operation(7), SecurityEpoch::INITIAL);
        assert!(fence.revoke(operation(7)), "the first call withdraws");
        assert!(!fence.revoke(operation(7)), "the second call is a no-op");
        assert_eq!(
            fence.check(&intent),
            Err(FenceRefusal::Revoked {
                operation: operation(7)
            })
        );
        // A different operation is untouched: revocation is not a blanket.
        assert_eq!(
            fence.check(&admitted_at(operation(8), SecurityEpoch::INITIAL)),
            Ok(())
        );
        assert!(fence.is_revoked(operation(7)));
        assert!(!fence.is_revoked(operation(8)));
        // There is no unrevoke: the withdrawal survives an advance and an
        // authentication, so replaying the pre-revocation state cannot
        // resurrect the effect.
        assert!(fence.advance_epoch().is_ok());
        assert_eq!(
            fence.authenticate(SecurityEpoch::new(0)),
            SecurityEpoch::new(1),
            "authentication never lowers the epoch, so the replay is absorbed"
        );
        assert!(fence.is_revoked(operation(7)));
    }

    #[test]
    fn e71_an_advanced_epoch_fences_everything_admitted_below_it() {
        let mut fence = fence(4);
        let stale = admitted_at(operation(1), SecurityEpoch::INITIAL);
        assert!(fence.advance_epoch().is_ok());
        assert_eq!(
            fence.check(&stale),
            Err(FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::INITIAL,
                current: SecurityEpoch::new(1)
            })
        );
        // An effect admitted at the new generation is unaffected: the
        // epoch fences what it withdrew, not everything.
        assert_eq!(
            fence.check(&admitted_at(operation(2), SecurityEpoch::new(1))),
            Ok(())
        );
    }

    #[test]
    fn e71_authentication_raises_to_the_reported_epoch_and_absorbs_an_older_one() {
        let mut fence = fence(4);
        fence.spend_offline();
        assert_eq!(fence.offline_spent(), 1);
        // A genuine advance from the authority source revokes by generation.
        assert_eq!(
            fence.authenticate(SecurityEpoch::new(9)),
            SecurityEpoch::new(9)
        );
        assert_eq!(fence.offline_spent(), 0, "reaching the source refreshes");
        assert_eq!(
            fence.check(&admitted_at(operation(1), SecurityEpoch::new(8))),
            Err(FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::new(8),
                current: SecurityEpoch::new(9)
            })
        );
        // A replayed older snapshot is absorbed: the fence does not lower.
        assert_eq!(
            fence.authenticate(SecurityEpoch::new(2)),
            SecurityEpoch::new(9)
        );
        assert_eq!(fence.epoch(), SecurityEpoch::new(9));
    }

    #[test]
    fn e71_time_passing_does_not_authenticate_and_a_rolled_back_clock_is_irrelevant() {
        // The fence holds no clock and no duration: there is no API on this
        // type that accepts a time, so no amount of elapsed or reversed time
        // can restore offline authority. Sleeping until the bound expires
        // changes nothing, and the bound is reached by counting reaches.
        let mut fence = RevocationFence::new(SecurityEpoch::INITIAL, 1);
        let intent = admitted_at(operation(1), SecurityEpoch::INITIAL);
        assert_eq!(fence.check(&intent), Ok(()));
        fence.spend_offline();
        assert_eq!(
            fence.check(&intent),
            Err(FenceRefusal::OfflineExposureExhausted { bound: 1 })
        );
        // Only the authority source being reached recharges it.
        assert_eq!(
            fence.authenticate(SecurityEpoch::INITIAL),
            SecurityEpoch::INITIAL
        );
        assert_eq!(fence.check(&intent), Ok(()));
    }

    #[test]
    fn e71_a_zero_bound_refuses_every_reach_even_a_freshly_authenticated_one() {
        // The bound is a ceiling, not a credit. Zero therefore does not mean
        // "authenticate before each dispatch" — it means "never", and that is
        // the property an operator reaching for a kill switch depends on. The
        // check is `spent >= bound`, and authentication clears `spent` to
        // zero, which is itself still at the ceiling.
        let mut fence = fence(0);
        let intent = admitted_at(operation(1), SecurityEpoch::INITIAL);
        assert_eq!(
            fence.check(&intent),
            Err(FenceRefusal::OfflineExposureExhausted { bound: 0 })
        );
        assert_eq!(
            fence.authenticate(SecurityEpoch::INITIAL),
            SecurityEpoch::INITIAL
        );
        assert_eq!(
            fence.offline_spent(),
            0,
            "authentication did clear the count"
        );
        assert_eq!(
            fence.check(&intent),
            Err(FenceRefusal::OfflineExposureExhausted { bound: 0 }),
            "and the reach is still refused: zero is a ceiling, not a credit"
        );
    }

    #[test]
    fn e71_a_one_bound_permits_exactly_one_reach_between_authentications() {
        let mut fence = fence(1);
        let intent = admitted_at(operation(1), SecurityEpoch::INITIAL);
        assert_eq!(fence.check(&intent), Ok(()));
        fence.spend_offline();
        assert_eq!(
            fence.check(&intent),
            Err(FenceRefusal::OfflineExposureExhausted { bound: 1 })
        );
        assert_eq!(
            fence.authenticate(SecurityEpoch::INITIAL),
            SecurityEpoch::INITIAL
        );
        assert_eq!(fence.check(&intent), Ok(()));
    }

    #[test]
    fn e71_epoch_exhaustion_refuses_rather_than_wrapping() {
        let mut fence = RevocationFence::new(SecurityEpoch::new(u64::MAX), 1);
        assert_eq!(fence.advance_epoch(), Err(FenceError::EpochExhausted));
        assert_eq!(fence.epoch(), SecurityEpoch::new(u64::MAX));
    }

    #[test]
    fn e71_refusals_are_ordered_revoked_then_stale_then_offline() {
        // An effect that is revoked *and* stale *and* out of offline
        // authority reports one deterministic reason, exactly as the reducer
        // reports the first of its ordered checks.
        let mut fence = fence(0);
        let intent = admitted_at(operation(3), SecurityEpoch::INITIAL);
        assert!(fence.revoke(operation(3)));
        assert!(fence.advance_epoch().is_ok());
        assert_eq!(
            fence.check(&intent),
            Err(FenceRefusal::Revoked {
                operation: operation(3)
            })
        );
        // Clear the revocation for a fresh operation: now the epoch is the
        // first-failing check.
        assert_eq!(
            fence.check(&admitted_at(operation(4), SecurityEpoch::INITIAL)),
            Err(FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::INITIAL,
                current: SecurityEpoch::new(1)
            })
        );
        // Current generation, not revoked: the offline bound is last.
        assert_eq!(
            fence.check(&admitted_at(operation(5), SecurityEpoch::new(1))),
            Err(FenceRefusal::OfflineExposureExhausted { bound: 0 })
        );
    }

    #[test]
    fn offline_exposure_saturates_rather_than_wrapping() {
        let mut fence = RevocationFence::new(SecurityEpoch::INITIAL, u32::MAX);
        fence.offline_spent = u32::MAX;
        fence.spend_offline();
        assert_eq!(
            fence.offline_spent(),
            u32::MAX,
            "a wrapped counter would hand back exposure already spent"
        );
    }

    #[test]
    fn an_epoch_serializes_as_a_bare_number() {
        // The wire form matters: this value is persisted inside a recorded
        // intent, so a nested-object encoding would change the journal's
        // bytes for a security fact. Asserting the exact string checks the
        // property rather than trusting a serde default.
        let encoded = lgwks_std::json::to_string(&SecurityEpoch::new(7));
        assert!(encoded.is_ok(), "serialize failed: {encoded:?}");
        assert_eq!(encoded.unwrap_or_default(), "7");
        let decoded: Result<SecurityEpoch, _> = lgwks_std::json::from_str("7");
        assert!(decoded.is_ok(), "deserialize failed: {decoded:?}");
        assert_eq!(
            decoded.unwrap_or(SecurityEpoch::new(0)),
            SecurityEpoch::new(7)
        );
    }
}
