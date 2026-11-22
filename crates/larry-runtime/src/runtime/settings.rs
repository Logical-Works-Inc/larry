//! The runtime's configuration and its one validation rule (larry#212).
//!
//! Every bound the runtime enforces is a field of [`RuntimeSettings`]: the
//! capacities it refuses past, the payload byte ceiling `admit` applies, the
//! skew allowance a lease is judged with, the dispatch attempts an idempotent
//! effect gets, the protocol range it negotiates, and the admission lease its
//! host mints validity from. The defaults are the Cell 05 values this crate
//! shipped with, so a host that names none gets exactly the old behaviour.
//!
//! [`Runtime::new`](super::Runtime::new) and [`restore`](super::restore) both
//! call [`RuntimeSettings::validate`] before they build anything. A zero or
//! out-of-range bound is a typed [`RuntimeError::BadSetting`] naming the field
//! and its range, never a silent clamp: a host that sets 4,096 runs must get
//! 4,096 runs, and a host that sets zero must be told, not quietly repaired.

use std::time::Duration;

use larry_contracts::authority::{BudgetUnit, SecurityEpoch};
use larry_contracts::binding::MAX_BOUND_FIELD_BYTES;
use larry_contracts::effect::MAX_EFFECT_ATTEMPTS;
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};

use crate::error::RuntimeError;
use crate::protocol::{MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, ProtocolVersionRange};
use crate::{
    DEFAULT_ADMISSION_LEASE, MAX_ADMISSION_LEASE, MAX_DISPLAY_GRANTS, MAX_JOURNAL_CAPACITY,
    MAX_JOURNAL_ENTRIES, MAX_PENDING_QUESTIONS, MAX_RUN_CAPACITY, MAX_RUNS_PER_TICK,
    MAX_SKEW_BOUND, MAX_TENANTS, MAX_TRACKED_RUNS,
};

/// The runtime's configuration: the pinned snapshots and the bounds.
///
/// The policy, capability and validity snapshots are host-supplied and pinned
/// for the runtime's lifetime in this slice. A later work package replaces
/// them with live, versioned snapshots activated at safe boundaries (D26);
/// the runtime already threads them as inputs rather than reading them from
/// ambient state, so that change is a wiring change and not a rewrite.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RuntimeSettings {
    /// The pinned policy snapshot.
    pub policy: PolicySnapshot,
    /// The pinned capability snapshot.
    pub capabilities: CapabilitySnapshot,
    /// The pinned authority-validity snapshot.
    pub validity: ValiditySnapshot,
    /// The unit the run budget is denominated in.
    pub unit: BudgetUnit,
    /// The initially reserved budget.
    pub initial_budget: u64,
    /// The security generation the fence starts at (R87, D48). Host-supplied
    /// like the snapshots above, and for the same reason: it is a fact about
    /// authority, not about this process.
    pub security_epoch: SecurityEpoch,
    /// How many mutating reaches the fence permits before the authority
    /// source must be re-contacted.
    pub offline_dispatch_bound: u32,
    /// The journal entry ceiling.
    pub journal_capacity: usize,
    /// The run-table ceiling. The projection is built at the same capacity, so
    /// a journal this runtime wrote is one a restore can always reopen.
    pub run_capacity: usize,
    /// How many ready commands one tick may admit.
    pub max_runs_per_tick: usize,
    /// The ceiling on pending questions.
    pub max_pending_questions: usize,
    /// How many tenants share the run table, the pending set and the pending
    /// questions. Each tenant may hold at most `capacity / max_tenants`, never
    /// less than one, so one tenant cannot take a cap every tenant shares
    /// (larry#333). The default of one leaves the whole cap to a single
    /// tenant; a host serving several sets its tenant count before it builds
    /// its runtime or restores one.
    pub max_tenants: usize,
    /// The ceiling on the grants one display may offer.
    pub max_grants: usize,
    /// The largest payload one admitted command may carry, in bytes.
    pub max_payload_bytes: usize,
    /// How far two honest clocks may disagree when a lease is judged, pinned
    /// with each decision through [`ValiditySnapshot::with_skew`].
    pub skew_bound: Duration,
    /// The dispatch attempts an idempotent effect gets.
    pub max_effect_attempts: u8,
    /// The lowest protocol version this runtime negotiates.
    pub min_protocol_version: u16,
    /// The highest protocol version this runtime negotiates.
    pub max_protocol_version: u16,
    /// The admission lease a host mints its validity from. The executor's gate
    /// reads it instead of keeping a constant of its own.
    pub lease: Duration,
}

impl RuntimeSettings {
    /// Settings with the specification's Cell 05 default bounds.
    #[must_use]
    pub fn with_defaults(
        policy: PolicySnapshot,
        capabilities: CapabilitySnapshot,
        validity: ValiditySnapshot,
        unit: BudgetUnit,
        initial_budget: u64,
    ) -> Self {
        Self {
            policy,
            capabilities,
            validity,
            unit,
            initial_budget,
            security_epoch: SecurityEpoch::INITIAL,
            offline_dispatch_bound: crate::MAX_OFFLINE_DISPATCHES,
            journal_capacity: MAX_JOURNAL_ENTRIES,
            run_capacity: MAX_TRACKED_RUNS,
            max_runs_per_tick: MAX_RUNS_PER_TICK,
            max_pending_questions: MAX_PENDING_QUESTIONS,
            // One tenant is the old behaviour: every cap whole to the tenant
            // that holds it, so a journal recorded before larry#333 replays the
            // same refusals at the same instant.
            max_tenants: 1,
            max_grants: MAX_DISPLAY_GRANTS,
            max_payload_bytes: MAX_BOUND_FIELD_BYTES,
            skew_bound: Duration::ZERO,
            max_effect_attempts: MAX_EFFECT_ATTEMPTS,
            min_protocol_version: MIN_PROTOCOL_VERSION,
            max_protocol_version: MAX_PROTOCOL_VERSION,
            lease: DEFAULT_ADMISSION_LEASE,
        }
    }

    /// The protocol version range this runtime negotiates.
    #[must_use]
    pub const fn protocol_range(&self) -> ProtocolVersionRange {
        ProtocolVersionRange::new(self.min_protocol_version, self.max_protocol_version)
    }

    /// `capacity` divided by the declared tenant count, never less than one.
    ///
    /// One tenant cannot hold more than this of a cap every tenant shares
    /// (larry#333). Integer division may leave a remainder no tenant may use;
    /// a capacity below the tenant count gives every tenant one.
    #[must_use]
    pub(crate) const fn tenant_share(&self, capacity: usize) -> usize {
        // `checked_div` keeps the lint set's ban on bare arithmetic; validate
        // refuses a zero tenant count, and the floor keeps a capacity below
        // the tenant count at one.
        let share = match capacity.checked_div(self.max_tenants) {
            Some(share) => share,
            None => 1,
        };
        if share == 0 { 1 } else { share }
    }

    /// The settings with the skew allowance pinned into the validity snapshot
    /// every decision carries.
    ///
    /// [`Runtime::new`](super::Runtime::new) and [`restore`](super::restore)
    /// apply this before they build anything, so exactly one knob —
    /// [`RuntimeSettings::skew_bound`] — decides the allowance, and a decision
    /// replayed from its pinned inputs re-derives it (larry#212).
    #[must_use]
    pub(super) fn pinned(mut self) -> Self {
        self.validity = ValiditySnapshot::with_skew(self.validity.valid_until, self.skew_bound);
        self
    }

    /// Refuses every bound outside the range its meaning allows.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BadSetting`] naming the first field, its value and the
    /// range it must fall in.
    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        // The declared ceiling binds: a host past it is told, as for
        // `max_pending_questions` (larry#333, larry#295).
        in_range(
            "journal_capacity",
            self.journal_capacity,
            1,
            MAX_JOURNAL_CAPACITY,
        )?;
        in_range("run_capacity", self.run_capacity, 1, MAX_RUN_CAPACITY)?;
        in_range("max_tenants", self.max_tenants, 1, MAX_TENANTS)?;
        // The declared ceiling binds: a host past it is told, as for `max_grants` (larry#295).
        in_range(
            "max_pending_questions",
            self.max_pending_questions,
            1,
            MAX_PENDING_QUESTIONS,
        )?;
        in_range("max_grants", self.max_grants, 1, MAX_DISPLAY_GRANTS)?;
        in_range(
            "max_payload_bytes",
            self.max_payload_bytes,
            1,
            MAX_BOUND_FIELD_BYTES,
        )?;
        in_range("max_effect_attempts", self.max_effect_attempts, 1, u8::MAX)?;
        in_range(
            "skew_bound_secs",
            self.skew_bound.as_secs(),
            0,
            MAX_SKEW_BOUND.as_secs(),
        )?;
        in_range(
            "lease_secs",
            self.lease.as_secs(),
            1,
            MAX_ADMISSION_LEASE.as_secs(),
        )?;
        in_range(
            "min_protocol_version",
            self.min_protocol_version,
            1,
            u16::MAX,
        )?;
        in_range(
            "max_protocol_version",
            self.max_protocol_version,
            self.min_protocol_version,
            u16::MAX,
        )
    }
}

/// Refuses `value` outside `min..=max`, naming the setting and its range.
fn in_range<T>(name: &'static str, value: T, min: T, max: T) -> Result<(), RuntimeError>
where
    T: Copy + Ord + TryInto<u128>,
    <T as TryInto<u128>>::Error: std::error::Error + Send + Sync + 'static,
{
    let (value, min, max) = (widen(name, value)?, widen(name, min)?, widen(name, max)?);
    if value < min || value > max {
        return Err(RuntimeError::BadSetting {
            name,
            range: Box::new(crate::error::SettingRange { value, min, max }),
        });
    }
    Ok(())
}

/// `value` in the report's unit. Every integer that reaches it widens without
/// loss; one the platform cannot read in that unit is refused as its own typed
/// failure, keeping the cause, so a caller can see why no bound could judge it.
fn widen<T>(name: &'static str, value: T) -> Result<u128, RuntimeError>
where
    T: TryInto<u128>,
    <T as TryInto<u128>>::Error: std::error::Error + Send + Sync + 'static,
{
    value
        .try_into()
        .map_err(|cause| RuntimeError::SettingUnreadable {
            name,
            cause: Box::new(cause),
        })
}
