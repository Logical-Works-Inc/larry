//! Seeded security-epoch simulations (E71, R87, D48).
//!
//! Every test drives the real `Runtime` twice from one seed through the live
//! fence's generations and asserts the same fingerprint plus the
//! caller-visible admission or refusal the generation produces.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::state::RunState;
use larry_runtime::owner::fixture::ScriptedOwner;

use seed::step;
use sim_kit::{Offer, Sim, offer_runs, read_work, reader, replay, work};
use support::{TestRuntime, assert_state, operation, run, settings, succeeded, unknown};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Counts plus the epoch trace for the seeded-journey replay test.
type JourneyReplay = ((usize, usize), (usize, u64));

/// Wrapping seed evolution, independent of any clock or scheduler.
fn advance(seed: &mut u64) -> u64 {
    step(seed, 14_363_276_836_458_258_123, 21_593_398_131_336_969, 9)
}

/// Trace counts plus the generation the fence stands at.
fn epoch_tally(runtime: &TestRuntime) -> (usize, u64) {
    (runtime.journal().len(), runtime.fence().epoch().value())
}

/// E71: submitting at the initial generation dispatches without refusal.
#[test]
fn sim_epochs_initial_generation_dispatches() -> Outcome {
    replay(0_u64..8, |_| -> Sim<(usize, u64)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(1))]), 100)?;
        let body = read_work(1, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(1, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.authorized(),
            &[operation(1)],
            "initial generation admits"
        );
        assert_eq!(tick.fenced().len(), 0, "initial generation fences nothing");
        assert_eq!(
            runtime.fence().epoch(),
            SecurityEpoch::INITIAL,
            "the fence never moved"
        );
        Ok(epoch_tally(&runtime))
    })
}

/// E71: two advances stand the fence two generations above admission.
#[test]
fn sim_epochs_two_advances_stand_two_above() -> Outcome {
    replay(8_u64..16, |seed| -> Sim<((usize, u64), bool)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        let one = SecurityEpoch::INITIAL
            .successor()
            .ok_or("the initial epoch must advance")?;
        let two = one.successor().ok_or("the next epoch must advance")?;
        runtime.fence_mut().advance_epoch()?;
        runtime.fence_mut().advance_epoch()?;
        let (first_fp, first_two) = (epoch_tally(&runtime), runtime.fence().epoch() == two);
        assert!(first_two, "seed {seed}: two generations above");
        assert!(first_two, "seed {seed}: two generations above");
        Ok((first_fp, first_two))
    })
}

/// E71: authenticating a newer generation jumps the fence forward.
#[test]
fn sim_epochs_authenticate_newer_jumps_forward() -> Outcome {
    replay(16_u64..24, |seed| -> Sim<((usize, u64), bool)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(2))]), 100)?;
        let body = read_work(2, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(2, &body)?;
        runtime.tick()?;
        assert_eq!(runtime.fence().offline_spent(), 1, "one reach spent");
        let newer = SecurityEpoch::new(9);
        let stood = runtime.fence_mut().authenticate(newer);
        assert_eq!(stood, newer, "authentication jumps to the newer generation");
        assert_eq!(
            runtime.fence().offline_spent(),
            0,
            "authentication recharges"
        );
        let (first_fp, first_jumped) = (epoch_tally(&runtime), runtime.fence().epoch() == newer);
        assert!(first_jumped, "seed {seed}: fence jumps forward");
        assert!(first_jumped, "seed {seed}: fence jumps forward");
        Ok((first_fp, first_jumped))
    })
}

/// E71: authenticating the same generation recharges without moving.
#[test]
fn sim_epochs_authenticate_same_recharges_without_moving() -> Outcome {
    replay(24_u64..32, |seed| -> Sim<((usize, u64), u32)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(3))]), 100)?;
        let body = read_work(3, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(3, &body)?;
        runtime.tick()?;
        let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
        assert_eq!(stood, SecurityEpoch::INITIAL, "generation never moved");
        let (first_fp, first_spent) = (epoch_tally(&runtime), runtime.fence().offline_spent());
        assert_eq!(first_spent, 0, "seed {seed}: spent recharged");
        Ok((first_fp, first_spent))
    })
}

/// E71: a revoked operation stays fenced after an advance and a new submit.
#[test]
fn sim_epochs_revoked_stays_fenced_after_advance() -> Outcome {
    replay(32_u64..40, |_| -> Sim<(usize, u64)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(5))]), 100)?;
        offer_runs(
            &mut runtime,
            &[(4_u8, 4_u8), (5, 5)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert!(
            runtime.fence_mut().revoke(operation(4)),
            "withdrawal reports the change"
        );
        runtime.tick()?;
        runtime.fence_mut().advance_epoch()?;
        let fresh = read_work(6, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(6, &fresh)?;
        let tick = runtime.tick()?;
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert_eq!(tick.fenced().len(), 1, "revoked operation still fenced");
        assert!(
            refusal.contains("Revoked"),
            "revocation outranks the new epoch, got {refusal}"
        );
        assert_eq!(
            runtime.owner().dispatched(),
            &[operation(5), operation(6)],
            "unrevoked operations dispatch"
        );
        Ok(epoch_tally(&runtime))
    })
}

/// E71: a stranded admission stays stranded after re-authentication.
#[test]
fn sim_epochs_stranded_stays_stranded_after_reauthenticate() -> Outcome {
    replay(40_u64..48, |_| -> Sim<(usize, u64)> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 1;
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            ScriptedOwner::scripted([succeeded(operation(7))]),
            support::clock()?,
        )?;
        offer_runs(
            &mut runtime,
            &[(7_u8, 7_u8), (8, 8)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        runtime.tick()?;
        runtime.fence_mut().advance_epoch()?;
        let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
        assert_eq!(
            stood,
            runtime.fence().epoch(),
            "authentication reports standing"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "stranded admission still fenced");
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert!(
            refusal.contains("StaleEpoch"),
            "re-authentication cannot refresh an old admission, got {refusal}"
        );
        Ok(epoch_tally(&runtime))
    })
}

/// E71/D48: advancing to u64::MAX then once more refuses without wrapping.
#[test]
fn sim_epochs_advance_to_max_then_refuses() -> Outcome {
    for seed in 48_u64..56 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let mut topped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
            topped.security_epoch = SecurityEpoch::new(u64::MAX.saturating_sub(1));
            let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
                topped,
                ScriptedOwner::new(),
                support::clock()?,
            )?;
            let max = runtime.fence_mut().advance_epoch()?;
            assert_eq!(
                max,
                SecurityEpoch::new(u64::MAX),
                "advance reaches the maximum"
            );
            let refused = runtime.fence_mut().advance_epoch();
            assert!(refused.is_err(), "past-maximum advance refuses");
            if let Err(error) = refused {
                let report = format!("{error:?}");
                assert!(
                    report.contains("EpochExhausted"),
                    "refusal names exhaustion, got {report}"
                );
            }
            let _ = seed;
            Ok(true)
        };
        assert!(drive()?, "seed {seed}: maximum refused");
        assert!(drive()?, "seed {seed}: maximum refused twice alike");
    }
    Ok(())
}

/// R87: offline spending survives an advance and clears on authenticate.
#[test]
fn sim_epochs_spent_survives_advance_clears_on_authenticate() -> Outcome {
    replay(56_u64..64, |seed| -> Sim<((usize, u64), u32)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(9))]), 100)?;
        let body = read_work(9, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(9, &body)?;
        runtime.tick()?;
        assert_eq!(runtime.fence().offline_spent(), 1, "one reach spent");
        runtime.fence_mut().advance_epoch()?;
        assert_eq!(
            runtime.fence().offline_spent(),
            1,
            "advancing never recharges"
        );
        let here = runtime.fence().epoch();
        let stood = runtime.fence_mut().authenticate(here);
        assert_eq!(
            runtime.fence().offline_spent(),
            0,
            "authentication recharges"
        );
        let _ = stood;
        let (first_fp, first_spent) = (epoch_tally(&runtime), runtime.fence().offline_spent());
        assert_eq!(first_spent, 0, "seed {seed}: recharged to zero");
        Ok((first_fp, first_spent))
    })
}

/// R87: a zero bound fences even the first mutating reach.
#[test]
fn sim_epochs_zero_bound_fences_first_reach() -> Outcome {
    replay(64_u64..72, |_| -> Sim<(usize, u64)> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 0;
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            ScriptedOwner::scripted([succeeded(operation(10))]),
            support::clock()?,
        )?;
        let body = read_work(10, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(10, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "zero bound fences everything");
        assert!(
            runtime.owner().dispatched().is_empty(),
            "zero bound dispatches nothing"
        );
        assert_eq!(tick.receipts().len(), 0, "fenced reach receipts nothing");
        Ok(epoch_tally(&runtime))
    })
}

/// E71/R72: a status query stays open even when the bound is spent.
/// The runtime keeps asking, but a never-sent effect is never resolved by
/// the answer, so the run stays Reconciling with no receipt.
#[test]
fn sim_epochs_status_query_open_when_bound_spent() -> Outcome {
    replay(72_u64..80, |_| -> Sim<(usize, u64)> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 0;
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            ScriptedOwner::scripted([unknown(operation(11)), unknown(operation(11))]),
            support::clock()?,
        )?;
        let body = read_work(11, 1, ReconciliationStrategy::Queryable);
        runtime.offer(11, &body)?;
        let first = runtime.tick()?;
        assert_eq!(first.fenced().len(), 1, "the mutating reach fences");
        for _ in 0..2 {
            let tick = runtime.tick()?;
            assert_eq!(tick.reconciled(), &[operation(11)], "the query stays open");
            assert_eq!(
                tick.receipts().len(),
                0,
                "no answer resolves a never-sent effect"
            );
            assert_state(runtime.projection(), run(11), RunState::Reconciling)?;
        }
        assert!(
            runtime.owner().dispatched().is_empty(),
            "fenced reach dispatches nothing"
        );
        Ok(epoch_tally(&runtime))
    })
}

/// E71: a revoked dispatch stays reserved while its query stays open.
/// The runtime keeps asking the endpoint, but the answer resolves nothing
/// about an effect the runtime never sent, so the run stays Reconciling.
#[test]
fn sim_epochs_revoked_query_stays_open_without_resolution() -> Outcome {
    replay(80_u64..88, |_| -> Sim<(usize, u64)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([unknown(operation(12)), unknown(operation(12))]),
            100,
        )?;
        let body = read_work(12, 1, ReconciliationStrategy::Queryable);
        runtime.offer(12, &body)?;
        assert!(
            runtime.fence_mut().revoke(operation(12)),
            "withdrawal reports the change"
        );
        let first = runtime.tick()?;
        assert_eq!(first.fenced().len(), 1, "the mutating reach fences");
        assert!(
            runtime.owner().dispatched().is_empty(),
            "revoked reach dispatches nothing"
        );
        for _ in 0..2 {
            let tick = runtime.tick()?;
            assert_eq!(tick.reconciled(), &[operation(12)], "the query stays open");
            assert_eq!(
                tick.receipts().len(),
                0,
                "no answer resolves a never-sent effect"
            );
            assert_state(runtime.projection(), run(12), RunState::Reconciling)?;
        }
        Ok(epoch_tally(&runtime))
    })
}

/// E71: seeded fence journeys replay the same refusal counts on both runs.
#[test]
fn sim_epochs_seeded_journeys_replay_refusal_counts() -> Outcome {
    replay(88_u64..100, |seed| -> Sim<JourneyReplay> {
        let mut random = seed.saturating_add(61);
        let revoke_first = advance(&mut random) & 1 == 0;
        let advance_epoch = advance(&mut random) & 1 == 0;
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(13)), succeeded(operation(14))]),
            100,
        )?;
        offer_runs(
            &mut runtime,
            &[(13_u8, 13_u8), (14, 14)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        if revoke_first {
            assert!(
                runtime.fence_mut().revoke(operation(13)),
                "withdrawal reports the change"
            );
        }
        if advance_epoch {
            runtime.fence_mut().advance_epoch()?;
        }
        let tick = runtime.tick()?;
        Ok((
            (tick.authorized().len(), tick.fenced().len()),
            epoch_tally(&runtime),
        ))
    })
}

/// E71: authenticating newer then older keeps the newer generation.
#[test]
fn sim_epochs_newer_then_older_keeps_newer() -> Outcome {
    for seed in 100_u64..108 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
            let newer = SecurityEpoch::new(7);
            let stood = runtime.fence_mut().authenticate(newer);
            assert_eq!(stood, newer, "authentication jumps forward");
            let kept = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
            let _ = seed;
            Ok(kept == newer && runtime.fence().epoch() == newer)
        };
        assert!(drive()?, "seed {seed}: newer generation kept");
        assert!(drive()?, "seed {seed}: newer generation kept twice alike");
    }
    Ok(())
}

/// E71: three withdrawals accumulate and fence all three reaches.
#[test]
fn sim_epochs_three_withdrawals_fence_three_reaches() -> Outcome {
    replay(108_u64..116, |_| -> Sim<(usize, u64)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([
                succeeded(operation(15)),
                succeeded(operation(16)),
                succeeded(operation(17)),
            ]),
            100,
        )?;
        for (number, effect) in [(15_u8, 15_u8), (16, 16), (17, 17)] {
            let body = work(
                number,
                1,
                effect,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
            assert!(
                runtime.fence_mut().revoke(operation(effect)),
                "each withdrawal reports the change"
            );
        }
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized().len(), 3, "the gate authorizes all three");
        assert_eq!(tick.fenced().len(), 3, "all three reaches fenced");
        assert!(
            runtime.owner().dispatched().is_empty(),
            "no withdrawn reach dispatches"
        );
        Ok(epoch_tally(&runtime))
    })
}

/// E71: a withdrawal preceding the submission still fences the reach.
#[test]
fn sim_epochs_withdrawal_before_submit_still_fences() -> Outcome {
    replay(116_u64..124, |_| -> Sim<(usize, u64)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(18))]), 100)?;
        assert!(
            runtime.fence_mut().revoke(operation(18)),
            "withdrawal reports the change"
        );
        let body = read_work(18, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(18, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "pre-submission withdrawal fences");
        assert!(
            runtime.owner().dispatched().is_empty(),
            "withdrawn reach dispatches nothing"
        );
        Ok(epoch_tally(&runtime))
    })
}

/// R87: a very high bound admits every reach and counts each one.
#[test]
fn sim_epochs_high_bound_counts_every_reach() -> Outcome {
    replay(124_u64..132, |seed| -> Sim<((usize, u64), u32)> {
        let mut loose = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        loose.offline_dispatch_bound = u32::MAX;
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            loose,
            ScriptedOwner::scripted([
                succeeded(operation(19)),
                succeeded(operation(20)),
                succeeded(operation(21)),
            ]),
            support::clock()?,
        )?;
        offer_runs(
            &mut runtime,
            &[(19_u8, 19_u8), (20, 20), (21, 21)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized().len(), 3, "high bound admits every reach");
        assert_eq!(tick.fenced().len(), 0, "high bound fences nothing");
        let (first_fp, first_spent) = (epoch_tally(&runtime), runtime.fence().offline_spent());
        assert_eq!(first_spent, 3, "seed {seed}: three reaches spent");
        Ok((first_fp, first_spent))
    })
}
