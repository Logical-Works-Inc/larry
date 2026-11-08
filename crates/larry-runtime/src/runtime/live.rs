//! The runtime's private witnesses: which tenant owns a run, what the runtime
//! knows about each reserved effect, and which way it is about to reach an owner.

use std::time::SystemTime;

use larry_contracts::effect::EffectIntent;
use larry_contracts::ids::TenantId;

/// A run the runtime is tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RunLive {
    /// Tenant established by command admission.
    pub(super) tenant: TenantId,
}

/// An effect the runtime has reserved.
#[derive(Debug, Clone)]
pub(super) struct LiveEffect {
    /// Durable reservation associated with this live witness.
    pub(super) intent: EffectIntent,
    /// The authority lease a mutating reach presents (larry#77).
    pub(super) proof: crate::gate::Proof,
    /// Whether this effect may have reached its owner. Set when a mutating
    /// reach is actually taken, and set by `restore` for every recorded
    /// reservation, because the journal proves the effect was reserved and
    /// cannot prove it was never sent.
    ///
    /// This is the witness the journal does not carry. `EventKind` has no
    /// "dispatched" fact, so a journal alone cannot tell a refused reach from
    /// a crash inside the dispatch window; the live table can, because a
    /// refused reach calls no owner at all and this flag is set only on a call
    /// that happened. It is what keeps an endpoint's answer from being read as
    /// evidence about an effect that never left the process.
    pub(super) sent: bool,
    /// How many times the owner has been asked to *perform* the effect. Bounded
    /// by `MAX_EFFECT_ATTEMPTS` for strategies that declare retry safe.
    pub(super) dispatches: u8,
    /// How many times the owner has been asked *what happened*. Bounded
    /// separately, because reconciliation is not retry: a crash must not stop
    /// a queryable effect from ever being queried.
    pub(super) reconciliations: u8,
    /// The tick the effect was dispatched in. Reconciliation may not run in
    /// that same tick: an effect is only `Unknown` because time passed, so
    /// querying an endpoint in the instant it was called would be a fabricated
    /// timeout rather than an observed one.
    pub(super) dispatched_at_tick: u64,
    /// The instant the authorization pinned its observation (E30). With
    /// `valid_until`, the effect's authority lease, which bounds a retry.
    pub(super) admitted_at: SystemTime,
    /// The end of the authority's validity (E30). A retry past it is refused,
    /// because the approval behind the effect no longer stands.
    pub(super) valid_until: SystemTime,
}

/// Which way the runtime is about to reach the owner.
///
/// The distinction is the whole of the fence's scope, so it is a type rather
/// than a boolean argument at the call site: `Mutate` can change the world
/// outside this process and is fenced; `Observe` cannot and is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnerReach {
    /// Cause, or re-cause, an external change.
    Mutate,
    /// Ask what already happened.
    Observe,
}
