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
use std::time::SystemTime;

use crate::authority::{BudgetUnit, Scope};

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// The pinned policy snapshot (D36 input).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct PolicySnapshot {
    /// The policy revision the decision is made under; a decision is only
    /// comparable within one revision (D18: pinned, versioned inputs).
    pub revision: u64,
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
/// stops being valid. Authority-supplied, never worker-supplied (Cell 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct ValiditySnapshot {
    /// The instant validity ends; `observed_at >= valid_until` is expired.
    pub valid_until: SystemTime,
}

impl ValiditySnapshot {
    /// Pins the exclusive authority expiry instant.
    #[must_use]
    pub const fn new(valid_until: SystemTime) -> Self {
        Self { valid_until }
    }
}

/// The declared effect whose admission is being decided.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct EffectRequest {
    /// The stable operation ID of the effect to dispatch.
    pub operation: crate::ids::OperationId,
    /// The scopes the effect requires; every one must pass.
    required_scopes: BTreeSet<Scope>,
    /// The cost the effect declares.
    pub cost: u64,
    /// The unit the cost is declared in; it must match the budget snapshot.
    pub unit: BudgetUnit,
    /// The pinned observation instant (E30: expiry is a pinned input, so
    /// an expired approval is a typed denial, never consent by timeout).
    pub observed_at: SystemTime,
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
    if request.observed_at >= validity.valid_until {
        return denied(DenialReason::AuthorityExpired);
    }
    let first_refused = request
        .required_scopes()
        .iter()
        .find(|scope| !policy.allowed_scopes().contains(*scope));
    if let Some(scope) = first_refused {
        return denied(DenialReason::PolicyRefuses { scope: *scope });
    }
    let first_missing = request
        .required_scopes()
        .iter()
        .find(|scope| !capabilities.capabilities().contains(*scope));
    if let Some(scope) = first_missing {
        return denied(DenialReason::CapabilityMissing { scope: *scope });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::instant;
    use crate::ids::OperationId;

    fn op(byte: u8) -> OperationId {
        OperationId::from_bytes([byte; 16])
    }

    fn scopes(list: &[Scope]) -> BTreeSet<Scope> {
        list.iter().copied().collect()
    }

    fn pinned_inputs(
        request: EffectRequest,
        policy: PolicySnapshot,
        budget: BudgetSnapshot,
    ) -> PinnedInputs {
        PinnedInputs {
            request,
            policy,
            budget,
            capabilities: CapabilitySnapshot {
                capabilities: scopes(&[Scope::FileRead, Scope::FileWrite]),
            },
            validity: ValiditySnapshot {
                valid_until: instant(1_000),
            },
        }
    }

    fn allowed_request(cost: u64) -> EffectRequest {
        EffectRequest {
            operation: op(1),
            required_scopes: scopes(&[Scope::FileRead]),
            cost,
            unit: BudgetUnit::Tokens,
            observed_at: instant(500),
        }
    }

    fn open_policy() -> PolicySnapshot {
        PolicySnapshot {
            revision: 7,
            allowed_scopes: scopes(&[Scope::FileRead, Scope::FileWrite]),
        }
    }

    fn budget(remaining: u64) -> BudgetSnapshot {
        BudgetSnapshot {
            remaining,
            unit: BudgetUnit::Tokens,
        }
    }

    /// `request` against the open policy, 50 tokens, a read-only capability
    /// and validity until 1 000 s: the world every single-reason probe shares.
    fn decide_read_only(request: &EffectRequest) -> Decision {
        decide(
            request,
            &open_policy(),
            &budget(50),
            &CapabilitySnapshot {
                capabilities: scopes(&[Scope::FileRead]),
            },
            &ValiditySnapshot {
                valid_until: instant(1_000),
            },
        )
    }

    #[test]
    fn an_in_scope_affordable_fresh_request_is_authorized() {
        assert_eq!(decide_read_only(&allowed_request(10)), Decision::Authorized);
    }

    #[test]
    fn e30_an_expired_approval_is_a_typed_denial_never_consent_from_timeout() {
        let request = EffectRequest {
            observed_at: instant(1_000), // exactly at expiry: already invalid
            ..allowed_request(10)
        };
        let decision = decide_read_only(&request);
        assert_eq!(
            decision,
            Decision::Denied {
                reason: DenialReason::AuthorityExpired
            }
        );
    }

    #[test]
    fn denial_checks_run_in_a_fixed_deterministic_order() {
        // Expired AND policy-refused AND capability-missing AND overspent:
        // the reported reason is validity first.
        let hostile_request = || EffectRequest {
            operation: op(2),
            required_scopes: scopes(&[Scope::FileRead, Scope::ModelInvoke]),
            cost: 9_999,
            unit: BudgetUnit::Tokens,
            observed_at: instant(2_000),
        };
        let decide_hostile = |request: EffectRequest, budget: BudgetSnapshot, capable: &[Scope]| {
            decide(
                &request,
                &open_policy(),
                &budget,
                &CapabilitySnapshot {
                    capabilities: scopes(capable),
                },
                &ValiditySnapshot {
                    valid_until: instant(1_000),
                },
            )
        };
        let decision = decide_hostile(hostile_request(), budget(1), &[]);
        assert_eq!(
            decision,
            Decision::Denied {
                reason: DenialReason::AuthorityExpired
            }
        );
        // Alive, but policy refuses the first sorted refused scope.
        let decision = decide_hostile(
            EffectRequest {
                observed_at: instant(100),
                ..hostile_request()
            },
            budget(1),
            &[],
        );
        assert_eq!(
            decision,
            Decision::Denied {
                reason: DenialReason::PolicyRefuses {
                    scope: Scope::ModelInvoke
                }
            }
        );
        // Alive, policy-allowed, but capability missing.
        let decision = decide_hostile(
            EffectRequest {
                required_scopes: scopes(&[Scope::FileWrite]),
                observed_at: instant(100),
                ..hostile_request()
            },
            budget(1),
            &[Scope::FileRead],
        );
        assert_eq!(
            decision,
            Decision::Denied {
                reason: DenialReason::CapabilityMissing {
                    scope: Scope::FileWrite
                }
            }
        );
        // Alive, allowed, capable, but units differ.
        let decision = decide_hostile(
            EffectRequest {
                unit: BudgetUnit::MilliCents,
                required_scopes: scopes(&[Scope::FileWrite]),
                observed_at: instant(100),
                ..hostile_request()
            },
            budget(1),
            &[Scope::FileWrite],
        );
        assert!(matches!(
            decision,
            Decision::Denied {
                reason: DenialReason::UnitMismatch { .. }
            }
        ));
        // Everything passes but the budget.
        let decision = decide_hostile(
            EffectRequest {
                unit: BudgetUnit::Tokens,
                required_scopes: scopes(&[Scope::FileWrite]),
                observed_at: instant(100),
                ..hostile_request()
            },
            budget(1),
            &[Scope::FileWrite],
        );
        assert_eq!(
            decision,
            Decision::Denied {
                reason: DenialReason::BudgetExhausted {
                    required: 9_999,
                    remaining: 1
                }
            }
        );
    }

    #[test]
    fn e55_replay_is_bit_identical_under_pinned_snapshots() -> Result<(), Box<dyn std::error::Error>>
    {
        let log: Vec<ReplayRecord> = vec![
            allowed_request(10),
            allowed_request(50),
            EffectRequest {
                operation: op(3),
                required_scopes: scopes(&[Scope::FileWrite]),
                ..allowed_request(5)
            },
        ]
        .into_iter()
        .map(|request| {
            let pinned = pinned_inputs(request, open_policy(), budget(50));
            let recorded = pinned.decide();
            ReplayRecord { pinned, recorded }
        })
        .collect();
        let report = replay_log(&log);
        assert!(
            report.is_bit_identical(),
            "pinned replay must match: {report:?}"
        );
        assert_eq!(report.divergences(), 0);
        assert_eq!(report.findings().len(), log.len());
        for finding in report.findings() {
            assert!(matches!(finding, ReplayFinding::Match { .. }));
        }
        // Bit-identical is stronger than equal: the recorded and re-derived
        // decisions serialize to identical bytes.
        #[cfg(feature = "serde")]
        for record in &log {
            let recorded_bytes = lgwks_std::json::to_string(&record.recorded)?;
            let rederived = record.pinned.decide();
            let rederived_bytes = lgwks_std::json::to_string(&rederived)?;
            assert_eq!(
                recorded_bytes, rederived_bytes,
                "recorded and re-derived decisions must be byte-identical"
            );
        }
        Ok(())
    }

    #[test]
    fn e55_a_mutated_snapshot_diverges_only_as_an_explicit_versioned_finding() {
        let pinned = pinned_inputs(allowed_request(10), open_policy(), budget(50));
        let recorded = pinned.decide();
        assert_eq!(recorded, Decision::Authorized);
        let mut mutated = pinned;
        mutated.policy.revision = 8;
        mutated.policy.allowed_scopes = scopes(&[Scope::FileWrite]);
        let log = vec![ReplayRecord {
            pinned: mutated,
            recorded,
        }];
        let report = replay_log(&log);
        assert!(
            !report.is_bit_identical(),
            "a mutated snapshot must not pass silently"
        );
        assert_eq!(report.divergences(), 1);
        assert_eq!(
            report.findings().first().cloned(),
            Some(ReplayFinding::Diverged {
                index: 0,
                recorded: Decision::Authorized,
                rederived: Decision::Denied {
                    reason: DenialReason::PolicyRefuses {
                        scope: Scope::FileRead
                    }
                }
            })
        );
    }

    #[test]
    fn an_empty_log_replays_to_an_identical_empty_report() {
        let report = replay_log(&[]);
        assert!(report.is_bit_identical());
        assert!(report.findings().is_empty());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn decisions_roundtrip_through_json() {
        let decisions = [
            Decision::Authorized,
            Decision::Denied {
                reason: DenialReason::PolicyRefuses {
                    scope: Scope::NetworkEgress,
                },
            },
        ];
        for decision in decisions {
            let decoded = lgwks_std::json::to_string(&decision)
                .and_then(|encoded| lgwks_std::json::from_str::<Decision>(&encoded))
                .map_err(|error| error.to_string());
            assert_eq!(decoded, Ok(decision), "roundtrip failed");
        }
    }
}
