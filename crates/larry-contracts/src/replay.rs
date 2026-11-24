//! The pure authorization reducer and recorded-log replay (spec R71, D36,
//! E55; work package C02).
//!
//! D36: authorization is a pure function of the declared effect, the
//! policy snapshot, the capability state and the reserved budget snapshot.
//! [`decide`] is that function: no clock reads, no entropy, no I/O, no
//! ambient state — time and every snapshot are pinned inputs. Denial
//! checks run in a fixed order (validity, policy, capability, budget), so
//! the first denial reason is itself deterministic.
//!
//! R71/E55: replaying the recorded effect log against pinned inputs yields
//! **bit-identical** authorization decisions. [`replay_log`] re-derives
//! every decision from its pinned snapshots and reports each finding
//! explicitly; a mutated snapshot diverges only as an explicit
//! [`ReplayFinding::Diverged`] record — never as a silent pass.
//!
//! NOT CLAIMED HERE: P5 (deterministic replay equivalence) is a proof
//! obligation to be discharged by property tests and model checking over
//! this surface in a later work package; this module provides the typed
//! surface the proof will attach to.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use crate::authority::{BudgetUnit, Scope};
// lgwks_bot 9.0.0 exports no root items (crates#1016): the grant and capability types come
// through the supported `kernel` surface, and `admit`'s error through its own module.
use lgwks_bot::error::KernelError;
use lgwks_bot::kernel::cap::Cap;
use lgwks_bot::kernel::gate::GrantSet;

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// The pinned policy snapshot (D36 input).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct PolicySnapshot {
    /// The policy revision the decision is made under; a decision is only
    /// comparable within one revision (D18: pinned, versioned inputs).
    revision: u64,
    /// The scopes the policy allows at this revision.
    allowed_scopes: BTreeSet<Scope>,
}

impl PolicySnapshot {
    /// Pins a policy revision and its admitted scopes.
    #[must_use]
    pub const fn new(revision: u64, allowed_scopes: BTreeSet<Scope>) -> Self {
        Self {
            revision,
            allowed_scopes,
        }
    }

    /// Returns the policy revision this snapshot was pinned at.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the scopes admitted by this immutable policy snapshot.
    #[must_use]
    pub fn allowed_scopes(&self) -> &BTreeSet<Scope> {
        &self.allowed_scopes
    }
}

/// The pinned budget snapshot (D36 input): what remains of the run's
/// reserved budget at decision time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct BudgetSnapshot {
    /// The remaining amount.
    pub remaining: u64,
    /// The unit both the remaining amount and the requested cost use.
    pub unit: BudgetUnit,
}

impl BudgetSnapshot {
    /// Pins a remaining balance without changing its denomination.
    #[must_use]
    pub const fn new(remaining: u64, unit: BudgetUnit) -> Self {
        Self { remaining, unit }
    }
}

/// The pinned capability snapshot (D36 input): what the requesting worker
/// is currently capable of, independent of policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct CapabilitySnapshot {
    /// The worker's capabilities.
    capabilities: BTreeSet<Scope>,
}

impl CapabilitySnapshot {
    /// Pins the worker's observed capabilities.
    #[must_use]
    pub const fn new(capabilities: BTreeSet<Scope>) -> Self {
        Self { capabilities }
    }

    /// Returns the capabilities observed in this immutable snapshot.
    #[must_use]
    pub fn capabilities(&self) -> &BTreeSet<Scope> {
        &self.capabilities
    }
}

/// The pinned authority-validity snapshot: the instant the backing grant
/// stops being valid, and how far two honest clocks may disagree about it.
/// Authority-supplied, never worker-supplied (Cell 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct ValiditySnapshot {
    /// The instant validity ends; `observed_at >= valid_until` is expired.
    pub valid_until: SystemTime,
    /// The largest disagreement the deployment tolerates between the clock
    /// that granted the lease and the one judging it. Zero for one host.
    /// Pinned with the expiry so a replay re-derives the same decision.
    #[cfg_attr(feature = "serde", serde(default))]
    pub skew_bound: Duration,
}

impl ValiditySnapshot {
    /// Pins the exclusive authority expiry instant, with no skew allowance.
    #[must_use]
    pub const fn new(valid_until: SystemTime) -> Self {
        Self::with_skew(valid_until, Duration::ZERO)
    }

    /// Pins the expiry and the allowance the judging clock gets against it.
    #[must_use]
    pub const fn with_skew(valid_until: SystemTime, skew_bound: Duration) -> Self {
        Self {
            valid_until,
            skew_bound,
        }
    }
}

/// The declared effect whose admission is being decided.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct EffectRequest {
    /// The stable operation ID of the effect to dispatch.
    operation: crate::ids::OperationId,
    /// The scopes the effect requires; every one must pass.
    required_scopes: BTreeSet<Scope>,
    /// The cost the effect declares.
    cost: u64,
    /// The unit the cost is declared in; it must match the budget snapshot.
    unit: BudgetUnit,
    /// The pinned observation instant (E30: expiry is a pinned input, so
    /// an expired approval is a typed denial, never consent by timeout).
    observed_at: SystemTime,
}

impl EffectRequest {
    /// Declares an effect for validation; construction grants no authority.
    #[must_use]
    pub const fn new(
        operation: crate::ids::OperationId,
        required_scopes: BTreeSet<Scope>,
        cost: u64,
        unit: BudgetUnit,
        observed_at: SystemTime,
    ) -> Self {
        Self {
            operation,
            required_scopes,
            cost,
            unit,
            observed_at,
        }
    }

    /// Returns the scopes this effect requires at admission.
    #[must_use]
    pub fn required_scopes(&self) -> &BTreeSet<Scope> {
        &self.required_scopes
    }

    /// The stable operation ID of the effect to dispatch.
    #[must_use]
    pub const fn operation(&self) -> crate::ids::OperationId {
        self.operation
    }

    /// The cost the effect declares.
    #[must_use]
    pub const fn cost(&self) -> u64 {
        self.cost
    }

    /// The unit the declared cost is denominated in.
    #[must_use]
    pub const fn unit(&self) -> BudgetUnit {
        self.unit
    }

    /// The pinned observation instant the expiry is judged at.
    #[must_use]
    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }
}

/// Why a request was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum DenialReason {
    /// Authority validity had already ended at the pinned instant (E30).
    AuthorityExpired,
    /// The policy revision refuses a required scope; the first refused
    /// scope in sorted order is named.
    PolicyRefuses {
        /// The first required scope the policy disallows.
        scope: Scope,
    },
    /// The worker lacks a required capability; the first missing scope in
    /// sorted order is named.
    CapabilityMissing {
        /// The first required scope without a capability.
        scope: Scope,
    },
    /// The declared cost exceeds the remaining budget.
    BudgetExhausted {
        /// The declared cost.
        required: u64,
        /// The remaining budget.
        remaining: u64,
    },
    /// The cost and the budget are denominated in different units; the
    /// request is refused rather than coerced.
    UnitMismatch {
        /// The unit the cost was declared in.
        requested: BudgetUnit,
        /// The unit the budget is denominated in.
        budget: BudgetUnit,
    },
    /// A question bound to this effect was rejected, timed out or cancelled,
    /// or its approval does not cover the effect's content and scopes; a
    /// timeout is never consent (D25, E30).
    NotApproved,
}

/// The authorization decision (D36 output).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum Decision {
    /// The effect is admitted under the pinned snapshots.
    Authorized,
    /// The effect is refused for exactly one typed reason (the first in
    /// the fixed check order).
    Denied {
        /// The deterministic first-failing reason.
        reason: DenialReason,
    },
}

/// The estate capability a scope is exercised under (larry#77).
#[must_use]
pub fn capability(scope: Scope) -> Cap {
    let name = match scope {
        Scope::FileRead => "larry.file.read",
        Scope::FileWrite => "larry.file.write",
        Scope::NetworkEgress => "larry.net.egress",
        Scope::ProcessSpawn => "larry.process.spawn",
        Scope::ModelInvoke => "larry.model.invoke",
        Scope::ModelCommand => "larry.model.command",
    };
    Cap::new(name)
}

/// The first required scope (in sorted order) that `granted` does not cover, judged by
/// lgwks_bot's `GrantSet::admit`, so coverage has one owner (larry#77). Any refusal that
/// is not a named deficit fails closed on the first required scope.
fn first_uncovered(required: &BTreeSet<Scope>, granted: &BTreeSet<Scope>) -> Option<Scope> {
    let grants = granted.iter().fold(GrantSet::empty(), |grants, &scope| {
        grants.grant(capability(scope))
    });
    let caps: Vec<Cap> = required.iter().map(|&scope| capability(scope)).collect();
    match grants.admit(&caps) {
        Ok(()) => None,
        Err(KernelError::CapabilityDenied { deficit }) => {
            let missing = deficit.first().required();
            required
                .iter()
                .copied()
                .find(|&scope| capability(scope) == *missing)
        }
        Err(_) => required.iter().copied().next(),
    }
}

/// The pure authorization reducer (D36, R71).
///
/// Check order is fixed and total: validity, then policy, then capability,
/// then budget. Identical pinned inputs always produce a bit-identical
/// decision — the function is side-effect free and reads nothing but its
/// arguments.
#[must_use]
pub fn decide(
    request: &EffectRequest,
    policy: &PolicySnapshot,
    budget: &BudgetSnapshot,
    capabilities: &CapabilitySnapshot,
    validity: &ValiditySnapshot,
) -> Decision {
    let denied = |reason: DenialReason| Decision::Denied { reason };
    // The lease is judged by its estate owner (larry#74): the expiry as a reading on the
    // observer's own timeline, whose origin is the pinned instant, so no epoch is assumed.
    // The pinned validity carries the deployment's skew allowance (larry#212), so a foreign
    // clock within the bound still holds; zero is one host, where any disagreement is a defect.
    let remaining = match validity.valid_until.duration_since(request.observed_at()) {
        Ok(remaining) => remaining,
        // The expiry precedes the pinned instant: nothing remains. The judge below
        // reads that against the bound, which is a typed denial at zero and the
        // declared allowance on an ahead clock.
        Err(_) => Duration::ZERO,
    };
    let holds =
        lgwks_bot::kernel::skew::lease_holds(remaining, Duration::ZERO, validity.skew_bound);
    if !holds {
        return denied(DenialReason::AuthorityExpired);
    }
    if let Some(scope) = first_uncovered(request.required_scopes(), policy.allowed_scopes()) {
        return denied(DenialReason::PolicyRefuses { scope });
    }
    if let Some(scope) = first_uncovered(request.required_scopes(), capabilities.capabilities()) {
        return denied(DenialReason::CapabilityMissing { scope });
    }
    if request.unit != budget.unit {
        return denied(DenialReason::UnitMismatch {
            requested: request.unit,
            budget: budget.unit,
        });
    }
    if request.cost > budget.remaining {
        return denied(DenialReason::BudgetExhausted {
            required: request.cost,
            remaining: budget.remaining,
        });
    }
    Decision::Authorized
}

/// One pinned replay input bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct PinnedInputs {
    /// The declared effect.
    pub request: EffectRequest,
    /// The pinned policy snapshot.
    pub policy: PolicySnapshot,
    /// The pinned budget snapshot.
    pub budget: BudgetSnapshot,
    /// The pinned capability snapshot.
    pub capabilities: CapabilitySnapshot,
    /// The pinned validity snapshot.
    pub validity: ValiditySnapshot,
}

impl PinnedInputs {
    /// Bundles the exact inputs used for one deterministic authorization.
    #[must_use]
    pub const fn new(
        request: EffectRequest,
        policy: PolicySnapshot,
        budget: BudgetSnapshot,
        capabilities: CapabilitySnapshot,
        validity: ValiditySnapshot,
    ) -> Self {
        Self {
            request,
            policy,
            budget,
            capabilities,
            validity,
        }
    }

    /// Re-derives the decision from the pinned inputs alone.
    #[must_use]
    pub fn decide(&self) -> Decision {
        decide(
            &self.request,
            &self.policy,
            &self.budget,
            &self.capabilities,
            &self.validity,
        )
    }
}

/// One recorded effect-log entry: the pinned inputs and the decision the
/// reducer produced when the effect was originally authorized (R71).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct ReplayRecord {
    /// The pinned inputs recorded with the effect.
    pub pinned: PinnedInputs,
    /// The decision recorded at authorization time.
    pub recorded: Decision,
}

impl ReplayRecord {
    /// Records a decision with its original inputs for independent replay.
    #[must_use]
    pub const fn new(pinned: PinnedInputs, recorded: Decision) -> Self {
        Self { pinned, recorded }
    }
}

/// One replay finding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum ReplayFinding {
    /// The re-derived decision equals the recorded one.
    Match {
        /// The zero-based log position.
        index: usize,
        /// The re-derived (== recorded) decision.
        decision: Decision,
    },
    /// The re-derived decision differs from the recorded one. This is the
    /// explicit, versioned divergence report (E55): divergence under
    /// mutated inputs is surfaced, never passed silently.
    Diverged {
        /// The zero-based log position.
        index: usize,
        /// The decision recorded in the log.
        recorded: Decision,
        /// The decision re-derived from the (possibly mutated) inputs.
        rederived: Decision,
    },
}

/// The report of one [`replay_log`] pass.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct ReplayReport {
    /// One ordered finding for each replayed record.
    findings: Vec<ReplayFinding>,
}

impl ReplayReport {
    /// Every per-record finding, in log order; its length always equals
    /// the replayed log's length.
    #[must_use]
    pub fn findings(&self) -> &[ReplayFinding] {
        &self.findings
    }

    /// Whether every re-derived decision matched the recorded one.
    #[must_use]
    pub fn is_bit_identical(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| matches!(finding, ReplayFinding::Match { .. }))
    }

    /// The number of divergences.
    #[must_use]
    pub fn divergences(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| matches!(finding, ReplayFinding::Diverged { .. }))
            .count()
    }
}

/// Replays a recorded effect log: every decision is re-derived from its
/// pinned inputs and compared against the recorded one (R71, E55).
///
/// Pure and deterministic: the same log always produces the same report.
#[must_use]
pub fn replay_log(log: &[ReplayRecord]) -> ReplayReport {
    let findings = log
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let rederived = record.pinned.decide();
            if rederived == record.recorded {
                ReplayFinding::Match {
                    index,
                    decision: rederived,
                }
            } else {
                ReplayFinding::Diverged {
                    index,
                    recorded: record.recorded,
                    rederived,
                }
            }
        })
        .collect();
    ReplayReport { findings }
}
