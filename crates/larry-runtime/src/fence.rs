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
//! # One owner: `lgwks_bot::broker` (larry#68 row 3, larry#76)
//!
//! The fence holds no rule of its own. Its state is one environment of an
//! `lgwks_bot::broker::Broker`, and every fact below is that broker's:
//!
//! 1. **A security epoch** ([`SecurityEpoch`]), the environment's generation.
//!    Moving it forward withdraws every authorization admitted below it, at
//!    one total-order instant. [`RevocationFence::authenticate`] raises it to
//!    what the authority source reports and absorbs an older report (E71).
//! 2. **A revoked set**, at most 65,536 operations withdrawn by name, one way.
//! 3. **An offline exposure bound**: how many mutating reaches may happen
//!    without the authority source being re-contacted. It is a count, not a
//!    duration, so neither sleeping nor a rolled-back clock recharges it.
//!
//! A warrant is minted when the reach is checked and presented again at the
//! handoff, where the broker re-runs the same check, so a withdrawal landing in
//! between is still honoured; the reach is counted once the owner returns an
//! outcome, and an owner's typed refusal, which guarantees nothing was
//! attempted, counts nothing (E65). Larry keeps
//! only the mapping from its identifiers onto the broker's: epoch `n` is
//! generation `n + 1`, so `u64::MAX` is past the last generation and an
//! authority source reporting it closes the fence for good.
//!
//! # What the fence deliberately does not gate
//!
//! It does not gate *observation*. An effect that had already been dispatched
//! when revocation landed must still be resolvable, because fencing the
//! status query would leave an external effect permanently unlabelled. That
//! is the "already-dispatched treatment" D48 asks to be defined, enforced by
//! shape: the warrant is asked for only on the mutating reach.

use core::fmt;
use core::num::{NonZeroU64, NonZeroU128};

use larry_contracts::authority::SecurityEpoch;
use larry_contracts::effect::{EffectIntent, ReconciliationStrategy};
use larry_contracts::ids::OperationId;
use lgwks_bot::broker::{Authority, Broker, BrokerError};
use lgwks_bot::effect::{
    ActionDigest, ActionId, AttemptId, EffectIdentity, EffectKey, EnvironmentEpoch, EnvironmentId,
    FlowRevision, Id128, RunId as BotRunId,
};

use crate::error::RuntimeError;

/// The one broker environment a runtime's fence is. Each runtime holds its
/// own broker, so the value only has to be non-zero.
pub(crate) const ENVIRONMENT: EnvironmentId =
    EnvironmentId::new(Id128::from_nonzero(NonZeroU128::MIN));

/// The flow revision of a Larry run. Larry admits no flow document, so its
/// revision is this domain-separated constant, one per binary.
const FLOW: &[u8] = b"larry.runtime.v1.flow";

/// The domain separator of the action digest.
const ACTION: &[u8] = b"larry.effect.v1";

/// A Larry identifier as a non-zero lgwks_bot identifier, or a typed refusal
/// naming the field that was all-zero.
fn bot_id(
    bytes: &[u8; 16],
    operation: OperationId,
    field: &'static str,
) -> Result<Id128, RuntimeError> {
    match NonZeroU128::new(u128::from_be_bytes(*bytes)) {
        Some(value) => Ok(Id128::from_nonzero(value)),
        None => Err(RuntimeError::EffectIdentity { operation, field }),
    }
}

/// The strategy's tag inside the action digest. The wildcard is a strategy
/// this build does not know; `reconcile_one` refuses it before any retry is
/// decided, so its tag only has to differ from the declared ones.
fn strategy_tag(strategy: ReconciliationStrategy) -> u8 {
    match strategy {
        ReconciliationStrategy::Queryable => 0,
        ReconciliationStrategy::IdempotentRetrySafe => 1,
        ReconciliationStrategy::RequiresHuman => 2,
        ReconciliationStrategy::AlwaysCompensate => 3,
        _ => u8::MAX,
    }
}

/// The lgwks_bot key for an attempt at a Larry effect: the one key the fence
/// warrants and the retry rule reads (larry#76, larry#78).
///
/// The run, the operation (as the action) and the attempt map one to one, and
/// the environment is the fence's. The tenant, run, strategy and binding are
/// in the action digest, so two tenants never share a key. The epoch is the
/// admission epoch's generation, checked: an epoch with none is refused rather
/// than saturated onto its neighbour, which would let a reservation admitted
/// one epoch below the fence pass it.
///
/// # Errors
///
/// [`RuntimeError::EffectIdentity`] naming the all-zero `tenant`, `run` or
/// `operation`, or `admission_epoch` when it is `u64::MAX`.
pub(crate) fn effect_key(intent: &EffectIntent, attempts: u8) -> Result<EffectKey, RuntimeError> {
    let operation = intent.operation;
    // The tenant is not a key field, but an all-zero one is an uninitialised
    // identity and still names no effect.
    bot_id(intent.tenant.as_bytes(), operation, "tenant")?;
    let identity = EffectIdentity::new(
        BotRunId::new(bot_id(intent.run.as_bytes(), operation, "run")?),
        ENVIRONMENT,
        FlowRevision::new(lgwks_std::hash::blake3(FLOW)),
    );
    let attempt = match NonZeroU64::new(u64::from(attempts)) {
        Some(value) => value,
        None => NonZeroU64::MIN,
    };
    let epoch = generation(intent.admission_epoch).ok_or(RuntimeError::EffectIdentity {
        operation,
        field: "admission_epoch",
    })?;
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(ACTION);
    hasher.update(intent.operation.as_bytes());
    hasher.update(intent.run.as_bytes());
    hasher.update(intent.tenant.as_bytes());
    hasher.update(&[strategy_tag(intent.strategy)]);
    match intent.binding() {
        Some(binding) => hasher.update(binding.digest().as_bytes()),
        None => hasher.update(&[0_u8]),
    };
    Ok(identity.key(
        ActionId::new(bot_id(intent.operation.as_bytes(), operation, "operation")?),
        AttemptId::new(attempt),
        ActionDigest::new(hasher.finalize()),
        epoch,
    ))
}

/// The broker generation a security epoch is: one past it, because a
/// generation is never zero. `None` for `u64::MAX`, which has none.
pub(crate) const fn generation(epoch: SecurityEpoch) -> Option<EnvironmentEpoch> {
    match epoch.value().checked_add(1) {
        Some(next) => match NonZeroU64::new(next) {
            Some(value) => Some(EnvironmentEpoch::new(value)),
            None => None,
        },
        None => None,
    }
}

/// The security epoch a broker generation is.
const fn security(generation: EnvironmentEpoch) -> SecurityEpoch {
    SecurityEpoch::new(generation.get().saturating_sub(1))
}

/// The broker action an operation is, or `None` for the all-zero operation,
/// which names no action. The same big-endian mapping as the effect key's.
fn action(operation: OperationId) -> Option<ActionId> {
    NonZeroU128::new(u128::from_be_bytes(*operation.as_bytes()))
        .map(|value| ActionId::new(Id128::from_nonzero(value)))
}

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
    /// The effect names an epoch later than the fence has ever stood at: a
    /// journal restored under an authority older than the one that admitted
    /// it. The opposite investigation to [`Self::StaleEpoch`].
    NeverIssued {
        /// The generation the reservation claims.
        admitted: SecurityEpoch,
        /// The generation the fence stands at.
        current: SecurityEpoch,
    },
    /// The authority source has not been reached for `bound` mutating
    /// reaches. Offline authority is spent and the fence fails closed.
    OfflineExposureExhausted {
        /// The configured offline exposure ceiling.
        bound: u32,
    },
    /// The fence is closed: the authority source reported an epoch past the
    /// last one it can hold, so no reach is permitted again.
    Closed,
}

impl fmt::Display for FenceRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Revoked { operation } => {
                write!(f, "operation {operation} was revoked by name")
            }
            Self::StaleEpoch { admitted, current } => write!(
                f,
                "admitted at {admitted} but the fence stands at {current}: stale authority"
            ),
            Self::NeverIssued { admitted, current } => write!(
                f,
                "admitted at {admitted}, which the fence never reached; it stands at {current}"
            ),
            Self::OfflineExposureExhausted { bound } => write!(
                f,
                "offline authority spent: {bound} reaches without the authority source"
            ),
            Self::Closed => write!(f, "the fence is closed: its epoch space is spent"),
        }
    }
}

impl std::error::Error for FenceRefusal {}

/// A refused change to, or read of, the fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FenceError {
    /// The epoch cannot advance or be represented. Refused rather than
    /// wrapped: a wrapped epoch would alias a withdrawn generation with a
    /// live one, re-authorizing effects that revocation had withdrawn.
    EpochExhausted,
    /// The fence already holds as many withdrawals by name as it can; the
    /// operation is not withdrawn, and advancing the epoch still fences it.
    RevocationsFull {
        /// How many withdrawals it holds.
        limit: usize,
    },
    /// The all-zero operation names no action and cannot be withdrawn.
    ZeroOperation,
    /// The fence was closed by an epoch past its space and takes no change.
    Closed,
    /// The broker does not hold the fence's environment, or refused it in a
    /// way no path here produces (registered twice, or a refusal newer than
    /// this build).
    Unowned,
}

impl fmt::Display for FenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EpochExhausted => {
                write!(f, "security epoch space is exhausted; it never wraps")
            }
            Self::RevocationsFull { limit } => write!(
                f,
                "the fence already holds {limit} withdrawals; advance the epoch to fence the rest"
            ),
            Self::ZeroOperation => write!(f, "the all-zero operation cannot be withdrawn"),
            Self::Closed => write!(f, "the fence is closed and takes no change"),
            Self::Unowned => write!(f, "the broker does not hold the fence's environment"),
        }
    }
}

impl std::error::Error for FenceError {}

/// The fence change a broker refusal is.
fn refused(error: BrokerError) -> FenceError {
    match error {
        BrokerError::Exhausted { .. } => FenceError::EpochExhausted,
        BrokerError::RevocationsFull { limit, .. } => FenceError::RevocationsFull { limit },
        BrokerError::Closed { .. } => FenceError::Closed,
        _ => FenceError::Unowned,
    }
}

/// The reach refusal a broker refusal is, for `intent`.
fn fenced(error: BrokerError, intent: &EffectIntent) -> FenceRefusal {
    match error {
        BrokerError::Revoked { .. } => FenceRefusal::Revoked {
            operation: intent.operation,
        },
        BrokerError::Superseded { current, .. } => FenceRefusal::StaleEpoch {
            admitted: intent.admission_epoch,
            current: security(current),
        },
        BrokerError::NeverIssued { current, .. } => FenceRefusal::NeverIssued {
            admitted: intent.admission_epoch,
            current: security(current),
        },
        BrokerError::OfflineExposureExhausted { bound, .. } => {
            FenceRefusal::OfflineExposureExhausted { bound }
        }
        _ => FenceRefusal::Closed,
    }
}

/// The live freshness state consulted at dispatch (R87, D48, E71).
///
/// Host-owned: the authority layer revokes, advances and authenticates
/// through this type; the runtime only reads it. Nothing here is folded from
/// the journal, and nothing here is written to it, because freshness is not
/// a recorded fact — it is a fact about the present that a record cannot
/// carry.
#[derive(Debug)]
pub struct RevocationFence {
    /// The broker whose one environment, [`ENVIRONMENT`], is this fence.
    broker: Broker,
}

impl RevocationFence {
    /// A fence standing at `epoch` that permits `offline_bound` mutating
    /// reaches without a fresh authentication.
    ///
    /// A bound of zero is a kill switch, not a mistake: no mutating reach is
    /// ever permitted, and [`RevocationFence::authenticate`] does not lift
    /// it, because the bound is a ceiling on reaches-between-authentications
    /// rather than a credit that authentication hands out.
    ///
    /// # Errors
    ///
    /// [`FenceError::EpochExhausted`] for `u64::MAX`, which has no generation.
    pub fn new(epoch: SecurityEpoch, offline_bound: u32) -> Result<Self, FenceError> {
        let at = generation(epoch).ok_or(FenceError::EpochExhausted)?;
        let mut broker = Broker::new();
        broker.register(ENVIRONMENT).map_err(refused)?;
        broker.authenticate(ENVIRONMENT, at).map_err(refused)?;
        broker
            .set_offline_bound(ENVIRONMENT, offline_bound)
            .map_err(refused)?;
        Ok(Self { broker })
    }

    /// The generation the fence currently stands at.
    ///
    /// # Errors
    ///
    /// [`FenceError::Unowned`] if the broker no longer holds the environment.
    pub fn epoch(&self) -> Result<SecurityEpoch, FenceError> {
        let at = self.broker.epoch(ENVIRONMENT).ok_or(FenceError::Unowned)?;
        Ok(security(at))
    }

    /// The configured offline exposure ceiling.
    ///
    /// # Errors
    ///
    /// [`FenceError::Unowned`] if the broker stopped counting.
    pub fn offline_bound(&self) -> Result<u32, FenceError> {
        self.broker
            .offline_bound(ENVIRONMENT)
            .ok_or(FenceError::Unowned)
    }

    /// How much offline authority has been spent since the last
    /// authentication.
    ///
    /// # Errors
    ///
    /// [`FenceError::Unowned`] if the broker stopped counting.
    pub fn offline_spent(&self) -> Result<u32, FenceError> {
        self.broker
            .offline_spent(ENVIRONMENT)
            .ok_or(FenceError::Unowned)
    }

    /// Whether `operation` has been withdrawn by name.
    #[must_use]
    pub fn is_revoked(&self, operation: OperationId) -> bool {
        action(operation).is_some_and(|action| self.broker.is_revoked(ENVIRONMENT, action))
    }

    /// Withdraws `operation`'s authority by name. Idempotent, and accepted
    /// even by a closed fence, because a withdrawal must never fail for being
    /// late.
    ///
    /// Returns whether this call was the one that revoked it. There is no
    /// `unrevoke`: restoring authority is a *new* authorization, not the
    /// erasure of a withdrawal a replay could then resurrect.
    ///
    /// # Errors
    ///
    /// [`FenceError::ZeroOperation`], or [`FenceError::RevocationsFull`] when
    /// the operation is new and the fence holds all it can.
    pub fn revoke(&mut self, operation: OperationId) -> Result<bool, FenceError> {
        let action = action(operation).ok_or(FenceError::ZeroOperation)?;
        self.broker
            .revoke_operation(ENVIRONMENT, action)
            .map_err(refused)
    }

    /// Moves the fence to the next epoch: revocation's linearization point.
    ///
    /// Every effect admitted below the new epoch stops being dispatchable at
    /// that instant. Offline exposure is *not* reset: a local epoch bump is a
    /// withdrawal, not evidence that the authority source was reached.
    ///
    /// # Errors
    ///
    /// [`FenceError::EpochExhausted`] at `u64::MAX - 1`, the last epoch the
    /// fence can hold, and [`FenceError::Closed`] once it is closed.
    pub fn advance_epoch(&mut self) -> Result<SecurityEpoch, FenceError> {
        self.broker
            .replace(ENVIRONMENT)
            .map(security)
            .map_err(refused)
    }

    /// Establishes freshness: the authority source was reached and reports
    /// the generation it currently stands at.
    ///
    /// The fence moves to the later of the two and never back, so a replayed
    /// old device, grant or policy snapshot is absorbed rather than obeyed
    /// (E71), and offline exposure resets, because this call *is* the source
    /// being reached. Returns the generation the fence now stands at.
    ///
    /// # Errors
    ///
    /// [`FenceError::EpochExhausted`] for a report of `u64::MAX`, which has no
    /// generation: the fence closes rather than stand below what the source
    /// reported, and every later reach and authentication is refused.
    /// [`FenceError::Closed`] once it is closed.
    pub fn authenticate(&mut self, reported: SecurityEpoch) -> Result<SecurityEpoch, FenceError> {
        let Some(at) = generation(reported) else {
            self.broker.close(ENVIRONMENT).map_err(refused)?;
            return Err(FenceError::EpochExhausted);
        };
        self.broker
            .authenticate(ENVIRONMENT, at)
            .map(security)
            .map_err(refused)
    }

    /// Mints the warrant for `key`, the mutating reach `intent` would make.
    ///
    /// The broker's checks run in a fixed order — closed, named revocation,
    /// the epoch, then the offline bound — so the reported refusal is as
    /// deterministic as the reducer's denial reason. Crate-internal: the only
    /// way to learn that a reach was refused is to make it and read
    /// [`TickReport::fenced`](crate::runtime::TickReport::fenced).
    ///
    /// # Errors
    ///
    /// The first-failing [`FenceRefusal`].
    pub(crate) fn authorize(
        &self,
        key: EffectKey,
        intent: &EffectIntent,
    ) -> Result<Authority, FenceRefusal> {
        self.broker
            .authorize(key)
            .map_err(|error| fenced(error, intent))
    }

    /// Presents `warrant` at the handoff, immediately before the owner: the
    /// same check again, counting nothing, so a withdrawal that landed since
    /// the mint still refuses the reach.
    ///
    /// # Errors
    ///
    /// The first-failing [`FenceRefusal`].
    pub(crate) fn revalidate(
        &self,
        warrant: &Authority,
        intent: &EffectIntent,
    ) -> Result<(), FenceRefusal> {
        self.broker
            .revalidate(warrant)
            .map_err(|error| fenced(error, intent))
    }

    /// Counts the reach `warrant` made: the check once more and one unit of
    /// offline authority, in one broker call. A refusal counts nothing.
    ///
    /// # Errors
    ///
    /// The first-failing [`FenceRefusal`].
    pub(crate) fn spend(
        &mut self,
        warrant: &Authority,
        intent: &EffectIntent,
    ) -> Result<(), FenceRefusal> {
        self.broker
            .spend(warrant)
            .map_err(|error| fenced(error, intent))
    }
}

#[cfg(test)]
mod tests;
