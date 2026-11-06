//! The authorization gate: two checks, both pure, neither optional.
//!
//! D36 makes authorization a pure function of the declared effect, the policy
//! snapshot, the capability state and the reserved budget snapshot, and E55
//! requires that replaying the recorded log re-derive bit-identical
//! decisions. That is [`authorize`]: it calls the contract reducer and
//! returns the [`ReplayRecord`] that must be persisted with the intent. The
//! record is what makes the decision replayable later — a decision recorded
//! without its inputs is not replayable, only assertable.
//!
//! The second check is the effect-boundary re-validation
//! ([`revalidate_at_boundary`]). Authority that was valid at admission can
//! have been revoked or expired by the time the effect is about to leave, so
//! the grant is re-validated against live policy at the dispatch boundary.
//! Passing the gate is not a licence to dispatch later.

use std::time::SystemTime;

use larry_contracts::authority::{Grant, ResourcePolicy, Scope};
use larry_contracts::replay::{Decision, PinnedInputs, ReplayRecord};

use crate::error::RuntimeError;

/// Decides an effect under pinned inputs and returns the replayable record.
///
/// # Errors
///
/// Returns [`RuntimeError::Denied`] carrying the deterministic first-failing
/// [`DenialReason`](larry_contracts::replay::DenialReason). The check order
/// is fixed by the contract reducer (validity, policy, capability, budget),
/// so the recorded reason is itself reproducible.
pub fn authorize(pinned: &PinnedInputs) -> Result<ReplayRecord, RuntimeError> {
    let recorded = pinned.decide();
    match recorded {
        Decision::Authorized => Ok(ReplayRecord::new(pinned.clone(), recorded)),
        Decision::Denied { reason } => Err(RuntimeError::Denied {
            operation: pinned.request.operation,
            reason,
        }),
        other => Err(RuntimeError::UnsupportedDecision(other)),
    }
}

/// Re-validates the backing grant against live policy at the effect boundary.
///
/// # Errors
///
/// Returns [`RuntimeError::Authority`] when the grant has expired, when the
/// required scope was never granted or has fallen outside current policy, or
/// when the cost exceeds the granted ceiling.
pub fn revalidate_at_boundary(
    grant: &Grant,
    required: Scope,
    cost: u64,
    policy: &ResourcePolicy,
    at: SystemTime,
) -> Result<(), RuntimeError> {
    Ok(larry_contracts::authority::validate_at_effect_boundary(
        grant, required, cost, policy, at,
    )?)
}
