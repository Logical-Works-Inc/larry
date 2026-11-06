//! Seeded replay simulations (E55, D04, R71).
//!
//! Every test drives the real `Runtime` twice from one seed, rebuilds a fresh
//! projection from the recorded journal, and asserts the rebuilt view matches
//! the live one plus the same fingerprint on both runs.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::ids::ContentDigest;
use rocco_contracts::replay::replay_log;
use rocco_contracts::state::RunState;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::ProjectionBackend;

use rocco_runtime::projection::{FoldProjection, replay_authorizations};

use seed::step;
use sim_kit::{Offer, Sim, offer_runs, read_work, reader, reopen, replay, trace, work};
use support::{
    TestRuntime, assert_state, command_id, operation, proposal, run, succeeded, tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn advance(seed: &mut u64) -> u64 {
    step(seed, 16_578_374_476_512_345_143, 17_130_477_736_030_129, 27)
}

/// E55: replaying an empty journal folds an empty projection.
#[test]
fn sim_replay_empty_journal_folds_empty() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(0_u64..6, |_| -> Sim<usize> {
        let runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_eq!(rebuilt.run_count(), 0, "no runs folded");
        assert_eq!(rebuilt.receipts().len(), 0, "no receipts folded");
        Ok(rebuilt.run_count())
    })
}

/// E55: a rebuilt projection reproduces every live run state.
#[test]
fn sim_replay_reproduces_run_states() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(6_u64..12, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(1)), succeeded(operation(2))]),
            100,
        )?;
        offer_runs(
            &mut runtime,
            &[(1_u8, 1_u8), (2, 2)],
            3,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_eq!(
            rebuilt.run_count(),
            runtime.projection().run_count(),
            "same run count"
        );
        for number in [1_u8, 2] {
            assert_eq!(
                rebuilt.run_state(run(number)),
                runtime.projection().run_state(run(number)),
                "run {number} folds alike"
            );
        }
        assert_state(&rebuilt, run(1), RunState::Completed)?;
        Ok(trace(runtime.journal()))
    })
}

/// FoldProjection must be clonable for the rebuilt-state assertion above.
#[test]
fn sim_replay_projection_clone_folds_alike() -> Outcome {
    replay(12_u64..14, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(3))]), 100)?;
        let body = read_work(3, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(3, &body)?;
        runtime.tick()?;
        let again = runtime.projection().clone();
        assert_eq!(
            again.receipts(),
            runtime.projection().receipts(),
            "clones fold alike"
        );
        Ok(again.receipts().len())
    })
}

/// E55: a rebuilt projection reproduces every recorded receipt.
#[test]
fn sim_replay_reproduces_receipts() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(14_u64..20, |_| -> Sim<usize> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(4)), succeeded(operation(5))]),
            100,
        )?;
        offer_runs(
            &mut runtime,
            &[(4_u8, 4_u8), (5, 5)],
            2,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_eq!(
            rebuilt.receipts(),
            runtime.projection().receipts(),
            "receipts fold identically"
        );
        Ok(rebuilt.receipts().len())
    })
}

/// E55: every recorded authorization re-derives bit-identically.
#[test]
fn sim_replay_authorizations_bit_identical() -> Outcome {
    replay(20_u64..26, |_| -> Sim<usize> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(6)), succeeded(operation(7))]),
            100,
        )?;
        offer_runs(
            &mut runtime,
            &[(6_u8, 6_u8), (7, 7)],
            4,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        runtime.tick()?;
        let report = replay_authorizations(runtime.projection());
        assert!(report.is_bit_identical(), "authorizations replay exactly");
        assert_eq!(report.divergences(), 0, "no authorization diverges");
        assert_eq!(report.findings().len(), 2, "two findings recorded");
        Ok(report.findings().len())
    })
}

/// E55: mutating a recorded decision surfaces an explicit divergence.
#[test]
fn sim_replay_mutated_record_diverges() -> Outcome {
    replay(26_u64..32, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(8))]), 100)?;
        let body = read_work(8, 40, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(8, &body)?;
        runtime.tick()?;
        let mut mutated = runtime.projection().authorizations().to_vec();
        assert_eq!(mutated.len(), 1, "one recorded authorization");
        let mut changed = mutated[0].clone();
        changed.pinned.budget.remaining = 1;
        mutated[0] = changed;
        let report = replay_log(&mutated);
        assert_eq!(report.divergences(), 1, "mutation diverges explicitly");
        assert!(
            !report.is_bit_identical(),
            "mutated history is not identical"
        );
        Ok(report.divergences())
    })
}

/// E55: replaying after restore matches the live restored projection.
#[test]
fn sim_replay_after_restore_matches_live() -> Outcome {
    use rocco_runtime::runtime::replay as refold;

    replay(32_u64..38, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(9))]), 100)?;
        let body = read_work(9, 5, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(9, &body)?;
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 100)?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, restored.journal())?;
        assert_eq!(
            rebuilt.receipts(),
            restored.projection().receipts(),
            "replay matches the restored live view"
        );
        assert_eq!(
            rebuilt.run_count(),
            restored.projection().run_count(),
            "replay matches the restored run count"
        );
        Ok(trace(restored.journal()))
    })
}

/// E55: replaying into two projections folds the same view twice.
#[test]
fn sim_replay_two_projections_fold_alike() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(38_u64..44, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(10))]), 100)?;
        let body = read_work(10, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(10, &body)?;
        runtime.tick()?;
        let mut first = FoldProjection::default();
        let mut second = FoldProjection::default();
        refold(&mut first, runtime.journal())?;
        refold(&mut second, runtime.journal())?;
        assert_eq!(first.receipts(), second.receipts(), "receipts fold alike");
        assert_eq!(
            first.run_count(),
            second.run_count(),
            "run counts fold alike"
        );
        assert_eq!(
            first.run_state(run(10)),
            second.run_state(run(10)),
            "states fold alike"
        );
        Ok(trace(runtime.journal()))
    })
}

/// E55: a denied run replays to Failed with no authorization recorded.
#[test]
fn sim_replay_denied_run_folds_failed() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(44_u64..50, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 5)?;
        let body = read_work(11, 6, ReconciliationStrategy::Queryable);
        runtime.offer(11, &body)?;
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_state(&rebuilt, run(11), RunState::Failed)?;
        assert_eq!(
            rebuilt.authorizations().len(),
            0,
            "denial authorizes nothing"
        );
        Ok(trace(runtime.journal()))
    })
}

/// E55: a cancelled run replays to Cancelled with no receipt.
#[test]
fn sim_replay_cancelled_run_folds_cancelled() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    use support::cancel;
    replay(50_u64..56, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        let body = proposal(
            run(12),
            tenant(1),
            operation(12),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(cancel(command_id(12), &body)?)?;
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_state(&rebuilt, run(12), RunState::Cancelled)?;
        assert_eq!(rebuilt.receipts().len(), 0, "cancel receipts nothing");
        Ok(trace(runtime.journal()))
    })
}

/// E55: an unknown-permanent run replays to OutcomeUnknown.
#[test]
fn sim_replay_unknown_permanent_folds_outcome_unknown() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(56_u64..62, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(13))]), 100)?;
        let body = read_work(13, 1, ReconciliationStrategy::RequiresHuman);
        runtime.offer(13, &body)?;
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_state(&rebuilt, run(13), RunState::OutcomeUnknown)?;
        Ok(trace(runtime.journal()))
    })
}

/// E28/E55: a duplicate never double-folds the same run.
#[test]
fn sim_replay_duplicate_never_double_folds() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(62_u64..68, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(14))]), 100)?;
        let body = read_work(14, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(14, &body)?;
        runtime.tick()?;
        runtime.offer(14, &body)?;
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_state(&rebuilt, run(14), RunState::Completed)?;
        assert_eq!(rebuilt.receipts().len(), 1, "duplicate folds once");
        Ok(trace(runtime.journal()))
    })
}

/// E37/E55: two tenants' runs replay to the same two states.
#[test]
fn sim_replay_two_tenants_fold_both() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(68_u64..74, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(15)), succeeded(operation(16))]),
            100,
        )?;
        for (number, scope) in [(15_u8, 1_u8), (16, 2)] {
            let body = work(
                number,
                scope,
                number,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let mut rebuilt = FoldProjection::default();
        refold(&mut rebuilt, runtime.journal())?;
        assert_eq!(rebuilt.run_count(), 2, "both tenants' runs folded");
        assert_eq!(rebuilt.receipts().len(), 2, "both tenants' receipts folded");
        Ok(trace(runtime.journal()))
    })
}

/// E55: the finding count always matches the authorization count.
#[test]
fn sim_replay_findings_match_authorizations() -> Outcome {
    replay(74_u64..80, |seed| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([
                succeeded(operation(17)),
                succeeded(operation(18)),
                succeeded(operation(19)),
            ]),
            100,
        )?;
        offer_runs(
            &mut runtime,
            &[(17_u8, 17_u8), (18, 18), (19, 19)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        runtime.tick()?;
        let report = replay_authorizations(runtime.projection());
        assert_eq!(
            report.findings().len(),
            runtime.projection().authorizations().len(),
            "one finding per authorization"
        );
        let (first_findings, first_auths) = (
            report.findings().len(),
            runtime.projection().authorizations().len(),
        );
        assert_eq!(first_findings, 3, "seed {seed}: three findings");
        Ok((first_findings, first_auths))
    })
}

/// E55/R71: each decision replays against its own pinned budget snapshot.
#[test]
fn sim_replay_holds_each_decision_to_own_snapshot() -> Outcome {
    replay(80_u64..86, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(20)), succeeded(operation(21))]),
            100,
        )?;
        for (number, cost) in [(20_u8, 30_u64), (21, 20)] {
            let body = work(
                number,
                1,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let records = runtime.projection().authorizations();
        assert_eq!(records.len(), 2, "two decisions recorded");
        assert_eq!(
            records[0].pinned.budget.remaining, 100,
            "first sees full budget"
        );
        assert_eq!(
            records[1].pinned.budget.remaining, 70,
            "second sees remainder"
        );
        assert_eq!(runtime.remaining_budget(), 50, "both reservations held");
        let report = replay_authorizations(runtime.projection());
        assert!(report.is_bit_identical(), "snapshots replay exactly");
        Ok(trace(runtime.journal()))
    })
}

/// E55: a cloned journal replays identically to the original.
#[test]
fn sim_replay_cloned_journal_replays_identically() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(86_u64..92, |_| -> Sim<ContentDigest> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(22))]), 100)?;
        let body = read_work(22, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(22, &body)?;
        runtime.tick()?;
        let mut first = FoldProjection::default();
        let mut second = FoldProjection::default();
        refold(&mut first, runtime.journal())?;
        refold(&mut second, &runtime.journal().clone())?;
        assert_eq!(first.receipts(), second.receipts(), "clone replays alike");
        Ok(trace(runtime.journal()))
    })
}

/// E55: seeded run shapes replay to the same live view on both runs.
#[test]
fn sim_replay_seeded_shapes_match_live() -> Outcome {
    use rocco_runtime::runtime::replay as refold;
    replay(92_u64..100, |seed| -> Sim<ContentDigest> {
        let mut random = seed.saturating_add(41);
        let count = usize::try_from(advance(&mut random) & 3)?.saturating_add(1);
        let mut owners = Vec::new();
        for index in 0..count {
            let number = u8::try_from(index)?.saturating_add(23);
            owners.push(succeeded(operation(number)));
        }
        let mut runtime: TestRuntime = reader(ScriptedOwner::scripted(owners), 100)?;
        let mut runs = Vec::new();
        for index in 0..count {
            let number = u8::try_from(index)?.saturating_add(23);
            runs.push((number, number));
        }
        offer_runs(
            &mut runtime,
            &runs,
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let mut rebuilt = FoldProjection::default();
        runtime.tick()?;
        refold(&mut rebuilt, runtime.journal())?;
        assert_eq!(
            rebuilt.run_count(),
            runtime.projection().run_count(),
            "seeded runs fold alike"
        );
        assert_eq!(
            rebuilt.receipts().len(),
            runtime.projection().receipts().len(),
            "seeded receipts fold alike"
        );
        Ok(trace(runtime.journal()))
    })
}
