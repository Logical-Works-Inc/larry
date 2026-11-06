//! Seeded authorization-denial simulations (D36, E63).
//!
//! Every test drives the real `Runtime` twice from one seed against the pure
//! gate and asserts the same fingerprint plus the deterministic first-failing
//! reason. Check order is fixed: policy, then capability, then budget.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::ContentDigest;
use larry_contracts::replay::DenialReason;
use larry_contracts::state::RunState;
use larry_runtime::owner::fixture::ScriptedOwner;

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, reopen, replay, trace};
use support::{
    TestRuntime, assert_state, operation, proposal, run, runtime_with, succeeded, tenant,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn advance(seed: &mut u64) -> u64 {
    step(seed, 17_179_287_269_428_366_711, 11_253_257_634_551_369, 23)
}

/// D36: all three checks fail, but policy is reported first.
#[test]
fn sim_denials_all_bad_reports_policy_first() -> Outcome {
    replay(0_u64..8, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 0)?;
        let body = proposal(
            run(1),
            tenant(1),
            operation(1),
            &[Scope::NetworkEgress],
            99,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(1, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(1),
                DenialReason::PolicyRefuses {
                    scope: Scope::NetworkEgress
                }
            )],
            "policy is checked before capability and budget"
        );
        assert_state(runtime.projection(), run(1), RunState::Failed)?;
        Ok(trace(runtime.journal()))
    })
}

/// D36: capability and budget fail, but capability is reported.
#[test]
fn sim_denials_capability_before_budget() -> Outcome {
    replay(8_u64..16, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = runtime_with(
            ScriptedOwner::new(),
            &[Scope::FileRead, Scope::FileWrite],
            &[Scope::FileRead],
            0,
        )?;
        let body = proposal(
            run(2),
            tenant(1),
            operation(2),
            &[Scope::FileWrite],
            99,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(2, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(2),
                DenialReason::CapabilityMissing {
                    scope: Scope::FileWrite
                }
            )],
            "capability is checked before budget"
        );
        Ok(trace(runtime.journal()))
    })
}

/// D36: only the budget fails, with exact required and remaining.
#[test]
fn sim_denials_budget_reports_exact_amounts() -> Outcome {
    replay(16_u64..24, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 7)?;
        let body = read_work(3, 8, ReconciliationStrategy::Queryable);
        runtime.offer(3, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(3),
                DenialReason::BudgetExhausted {
                    required: 8,
                    remaining: 7
                }
            )],
            "budget names required and remaining"
        );
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 7, "seed {seed}: denial reserves nothing");
        Ok((first_budget, first_fp))
    })
}

/// D36: a zero-cost effect authorizes even against a zero budget.
#[test]
fn sim_denials_zero_cost_authorizes_on_zero_budget() -> Outcome {
    replay(24_u64..32, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(4))]), 0)?;
        let body = read_work(4, 0, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(4, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized(), &[operation(4)], "zero cost authorizes");
        assert_eq!(tick.receipts().len(), 1, "zero cost receipts");
        Ok(trace(runtime.journal()))
    })
}

/// D36: an exact-cost effect authorizes the budget down to zero.
#[test]
fn sim_denials_exact_cost_authorizes_to_zero() -> Outcome {
    replay(32_u64..40, |_| -> Sim<u64> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(5))]), 9)?;
        let body = read_work(5, 9, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(5, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized(), &[operation(5)], "exact cost authorizes");
        assert_eq!(runtime.remaining_budget(), 0, "budget reaches exactly zero");
        Ok(runtime.remaining_budget())
    })
}

/// D36: one unit over the budget denies with the exact shortfall.
#[test]
fn sim_denials_one_over_budget_denies() -> Outcome {
    replay(40_u64..48, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 9)?;
        let body = read_work(6, 10, ReconciliationStrategy::Queryable);
        runtime.offer(6, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(6),
                DenialReason::BudgetExhausted {
                    required: 10,
                    remaining: 9
                }
            )],
            "one unit over denies"
        );
        Ok(trace(runtime.journal()))
    })
}

/// D36: a scope outside the policy is denied whatever the budget.
#[test]
fn sim_denials_policy_denial_ignores_budget() -> Outcome {
    replay(48_u64..56, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 1_000)?;
        let body = proposal(
            run(7),
            tenant(1),
            operation(7),
            &[Scope::FileWrite],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(7, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 1, "policy denies regardless of budget");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 1_000, "seed {seed}: denial reserves nothing");
        Ok((first_budget, first_fp))
    })
}

/// D36: a capability denial holds the budget untouched.
#[test]
fn sim_denials_capability_denial_holds_budget() -> Outcome {
    replay(56_u64..64, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = runtime_with(
            ScriptedOwner::new(),
            &[Scope::FileRead, Scope::FileWrite],
            &[Scope::FileRead],
            50,
        )?;
        let body = proposal(
            run(8),
            tenant(1),
            operation(8),
            &[Scope::FileWrite],
            25,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(8, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 1, "capability denies");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 50, "seed {seed}: denial reserves nothing");
        Ok((first_budget, first_fp))
    })
}

/// D36: a denied run fails with no receipt and no owner reach.
#[test]
fn sim_denials_denied_run_fails_cleanly() -> Outcome {
    replay(64_u64..72, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        let body = proposal(
            run(9),
            tenant(1),
            operation(9),
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(9, &body)?;
        let tick = runtime.tick()?;
        assert_state(runtime.projection(), run(9), RunState::Failed)?;
        assert_eq!(tick.receipts().len(), 0, "denial receipts nothing");
        assert!(
            runtime.owner().dispatched().is_empty(),
            "denial reaches no owner"
        );
        assert!(
            runtime.owner().reconciled().is_empty(),
            "denial queries nothing"
        );
        Ok(trace(runtime.journal()))
    })
}

/// D36: a failed run stays failed across later quiet ticks.
#[test]
fn sim_denials_failed_state_survives_quiet_ticks() -> Outcome {
    replay(72_u64..80, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        let body = proposal(
            run(10),
            tenant(1),
            operation(10),
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(10, &body)?;
        runtime.tick()?;
        runtime.tick()?;
        let third = runtime.tick()?;
        assert!(third.is_idle(), "quiet ticks stay idle");
        assert_state(runtime.projection(), run(10), RunState::Failed)?;
        Ok(trace(runtime.journal()))
    })
}

/// D36: one denied and one allowed submission split the tick report.
#[test]
fn sim_denials_mixed_tick_splits_report() -> Outcome {
    replay(80_u64..88, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(12))]), 100)?;
        let denied_body = proposal(
            run(11),
            tenant(1),
            operation(11),
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(11, &denied_body)?;
        let allowed_body = read_work(12, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(12, &allowed_body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 1, "one denial recorded");
        assert_eq!(
            tick.authorized(),
            &[operation(12)],
            "one authorization recorded"
        );
        assert_eq!(tick.receipts().len(), 1, "one receipt published");
        assert_state(runtime.projection(), run(11), RunState::Failed)?;
        assert_state(runtime.projection(), run(12), RunState::Completed)?;
        Ok(trace(runtime.journal()))
    })
}

/// D36: a denial folds to Failed identically after restore.
#[test]
fn sim_denials_denial_folds_failed_after_restore() -> Outcome {
    replay(88_u64..96, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 5)?;
        let body = read_work(13, 6, ReconciliationStrategy::Queryable);
        runtime.offer(13, &body)?;
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 5)?;
        assert_state(restored.projection(), run(13), RunState::Failed)?;
        assert_eq!(restored.remaining_budget(), 5, "denial reserves nothing");
        Ok(trace(restored.journal()))
    })
}

/// D36: every configured scope authorizes when policy and capability allow.
#[test]
fn sim_denials_every_configured_scope_authorizes() -> Outcome {
    replay(96_u64..104, |_| -> Sim<usize> {
        let scopes = [Scope::FileRead, Scope::FileWrite, Scope::NetworkEgress];
        let mut runtime: TestRuntime = runtime_with(
            ScriptedOwner::scripted([
                succeeded(operation(14)),
                succeeded(operation(15)),
                succeeded(operation(16)),
            ]),
            &scopes,
            &scopes,
            100,
        )?;
        for (number, scope) in [
            (14_u8, Scope::FileRead),
            (15, Scope::FileWrite),
            (16, Scope::NetworkEgress),
        ] {
            let body = proposal(
                run(number),
                tenant(1),
                operation(number),
                &[scope],
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 0, "no configured scope denied");
        assert_eq!(tick.authorized().len(), 3, "every scope authorizes");
        Ok(tick.receipts().len())
    })
}

/// D36: a capability subset denies the scope it lacks.
#[test]
fn sim_denials_capability_subset_denies_extra_scope() -> Outcome {
    replay(104_u64..112, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = runtime_with(
            ScriptedOwner::new(),
            &[Scope::FileRead, Scope::FileWrite, Scope::NetworkEgress],
            &[Scope::FileRead, Scope::NetworkEgress],
            100,
        )?;
        let body = proposal(
            run(17),
            tenant(1),
            operation(17),
            &[Scope::FileWrite],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(17, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(17),
                DenialReason::CapabilityMissing {
                    scope: Scope::FileWrite
                }
            )],
            "missing capability denied"
        );
        Ok(trace(runtime.journal()))
    })
}

/// D36: seeded scope and authority sets authorize exactly when allowed.
#[test]
fn sim_denials_seeded_scope_matrix_authorizes_exactly() -> Outcome {
    replay(112_u64..128, |seed| -> Sim<(bool, ContentDigest)> {
        let mut random = seed.saturating_add(3);
        let scope = match advance(&mut random) & 3 {
            0 => Scope::FileRead,
            1 => Scope::FileWrite,
            _ => Scope::NetworkEgress,
        };
        let policy_open = advance(&mut random) & 1 == 0;
        let capable_open = advance(&mut random) & 1 == 0;
        let policy: &[Scope] = if policy_open {
            &[Scope::FileRead, Scope::FileWrite, Scope::NetworkEgress]
        } else {
            &[Scope::FileRead]
        };
        let capable: &[Scope] = if capable_open {
            &[Scope::FileRead, Scope::FileWrite, Scope::NetworkEgress]
        } else {
            &[Scope::FileRead]
        };
        let mut runtime: TestRuntime = runtime_with(
            ScriptedOwner::scripted([succeeded(operation(18))]),
            policy,
            capable,
            100,
        )?;
        let body = proposal(
            run(18),
            tenant(1),
            operation(18),
            &[scope],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        runtime.offer(18, &body)?;
        let tick = runtime.tick()?;
        let authorized = !tick.authorized().is_empty();
        let expected = policy.contains(&scope) && capable.contains(&scope);
        assert_eq!(authorized, expected, "gate authorizes exactly when allowed");
        assert_eq!(
            tick.denied().is_empty(),
            expected,
            "denial mirrors authorization"
        );
        Ok((authorized, trace(runtime.journal())))
    })
}

/// D36: two denials in one tick name their own scopes.
#[test]
fn sim_denials_two_denials_name_own_scopes() -> Outcome {
    replay(128_u64..136, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 0)?;
        let first = proposal(
            run(19),
            tenant(1),
            operation(19),
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(19, &first)?;
        let second = read_work(20, 1, ReconciliationStrategy::Queryable);
        runtime.offer(20, &second)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 2, "both effects denied");
        assert!(
            tick.denied().iter().any(|denied| denied
                == &(
                    operation(19),
                    DenialReason::PolicyRefuses {
                        scope: Scope::NetworkEgress
                    }
                )),
            "policy denial names its scope"
        );
        assert!(
            tick.denied().iter().any(|denied| denied
                == &(
                    operation(20),
                    DenialReason::BudgetExhausted {
                        required: 1,
                        remaining: 0
                    }
                )),
            "budget denial names its amounts"
        );
        Ok(trace(runtime.journal()))
    })
}
