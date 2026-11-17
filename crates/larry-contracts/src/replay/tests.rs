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
            skew_bound: std::time::Duration::ZERO,
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
            skew_bound: std::time::Duration::ZERO,
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
fn a_pinned_skew_bound_admits_a_lease_just_past_expiry_and_zero_does_not() {
    // The expiry ends at 1,000 s. An observer 10 s past it is refused at zero skew,
    // the one-host default, and admitted inside a 30 s allowance (larry#212).
    let request = EffectRequest {
        observed_at: instant(1_010),
        ..allowed_request(10)
    };
    let decide_with = |skew: std::time::Duration| {
        decide(
            &request,
            &open_policy(),
            &budget(50),
            &CapabilitySnapshot {
                capabilities: scopes(&[Scope::FileRead]),
            },
            &ValiditySnapshot::with_skew(instant(1_000), skew),
        )
    };
    assert_eq!(
        decide_with(std::time::Duration::ZERO),
        Decision::Denied {
            reason: DenialReason::AuthorityExpired
        }
    );
    assert_eq!(
        decide_with(std::time::Duration::from_secs(30)),
        Decision::Authorized
    );
    // An expiry before the pinned instant leaves nothing to judge; the same allowance
    // carries that reading too, which is the no-remaining branch of the reducer.
    let decide_at = |valid_until: std::time::SystemTime, skew: std::time::Duration| {
        decide(
            &request,
            &open_policy(),
            &budget(50),
            &CapabilitySnapshot {
                capabilities: scopes(&[Scope::FileRead]),
            },
            &ValiditySnapshot::with_skew(valid_until, skew),
        )
    };
    assert_eq!(
        decide_at(instant(1_005), std::time::Duration::ZERO),
        Decision::Denied {
            reason: DenialReason::AuthorityExpired
        }
    );
    assert_eq!(
        decide_at(instant(1_005), std::time::Duration::from_secs(30)),
        Decision::Authorized
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
                skew_bound: std::time::Duration::ZERO,
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
fn e55_replay_is_bit_identical_under_pinned_snapshots() -> Result<(), Box<dyn std::error::Error>> {
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

#[test]
fn larry_keeps_no_coverage_rule_of_its_own() {
    let source = include_str!("../replay.rs");
    assert!(
        source.contains("GrantSet"),
        "the coverage rule is lgwks_bot's GrantSet"
    );
    assert!(
        !source.contains(".find(|scope| !policy.allowed_scopes().contains(*scope))"),
        "no inline coverage loop survives beside GrantSet"
    );
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
