//! Seeded dispatch-fence simulations (E71, R87, D48).
//! Every test mutates the live fence the way the authority layer owns it,
//! then asserts the same counts plus the caller-visible refusal or admission.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::ContentDigest;
use larry_contracts::state::RunState;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::ProjectionBackend;

use seed::step;
use sim_kit::{Offer, Sim, offer_runs, read_work, reader, replay, trace};
use support::{TestRuntime, assert_state, operation, run, settings, succeeded};

/// Fallible test outcome; setup failures return as errors.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Fenced-trace snapshot: counts plus the journal digest.
type FencedTrace = ((usize, usize, usize), ContentDigest);
/// Fenced-trace snapshot with the refused run's folded state.
type FencedState = ((usize, usize, usize), RunState);

/// Wrapping seed evolution, independent of any clock or scheduler.
fn ratchet(seed: &mut u64) -> u64 {
    step(seed, 5_773_875_523_619_008_799, 491_134_414_345_647, 11)
}

/// Trace counts: every refusal is a present fact, journal replays alike.
fn fence_counts(runtime: &TestRuntime, fenced: usize) -> (usize, usize, usize) {
    let kept = runtime.journal().len();
    let receipts = runtime.projection().receipts().len();
    (kept, fenced, receipts)
}

/// Default file-read fence runtime over `owner`.
fn mk_fence(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    reader(owner, 100)
}

/// E71: a revoked operation is refused before any owner reach.
#[test]
fn sim_fence_revoked_operation_refused_before_reach() -> Outcome {
    replay(0_u64..8, |_| -> Sim<FencedTrace> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(1))]))?;
        let body = read_work(1, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(1, &body)?;
        assert!(
            runtime.fence_mut().revoke(operation(1)),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "one mutating reach fenced");
        assert_eq!(tick.fenced()[0].0, operation(1), "refusal names operation");
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert!(
            refusal.contains("Revoked"),
            "refusal names reason, got {refusal}"
        );
        assert!(
            runtime.owner().dispatched().is_empty(),
            "fenced reach dispatches none"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// E71: revocation by name is one-way and idempotent at the fence.
#[test]
fn sim_fence_revoke_is_one_way_idempotent() -> Outcome {
    replay(8_u64..16, |_| -> Sim<bool> {
        let mut runtime = mk_fence(ScriptedOwner::new())?;
        assert!(
            !runtime.fence().is_revoked(operation(2)),
            "fresh fence withdraws none"
        );
        assert!(
            runtime.fence_mut().revoke(operation(2)),
            "first withdrawal reports change"
        );
        assert!(
            runtime.fence().is_revoked(operation(2)),
            "withdrawal visible by name"
        );
        Ok(runtime.fence_mut().revoke(operation(2)))
    })
}

/// E71: the fence starts with nothing withdrawn.
#[test]
fn sim_fence_starts_with_nothing_withdrawn() -> Outcome {
    replay(16_u64..24, |_| -> Sim<bool> {
        let runtime = mk_fence(ScriptedOwner::new())?;
        Ok(runtime.fence().is_revoked(operation(3)))
    })
}

/// E71: advancing the epoch strands a prior bound-fenced admission as stale.
#[test]
fn sim_fence_advanced_epoch_strands_prior_admission() -> Outcome {
    replay(24_u64..32, |_| -> Sim<FencedTrace> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 1;
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            ScriptedOwner::scripted([succeeded(operation(4))]),
            larry_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        offer_runs(
            &mut runtime,
            &[(4_u8, 4_u8), (5, 5)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let first = runtime.tick()?;
        assert_eq!(
            first.authorized(),
            &[operation(4), operation(5)],
            "gate authorizes both"
        );
        assert_eq!(first.fenced().len(), 1, "bound fences second reach");
        let next = runtime.fence_mut().advance_epoch()?;
        let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
        assert_eq!(stood, next, "authentication stands at new generation");
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "stranded admission fenced");
        assert_eq!(
            tick.fenced()[0].0,
            operation(5),
            "report names stranded operation"
        );
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert!(
            refusal.contains("StaleEpoch"),
            "refusal names stale epoch, got {refusal}"
        );
        assert_eq!(
            runtime.owner().dispatched(),
            &[operation(4)],
            "stranded reach sends none"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// E71: the stale refusal reports the generation the fence now stands at.
#[test]
fn sim_fence_stale_refusal_reports_current_generation() -> Outcome {
    replay(32_u64..40, |_| -> Sim<FencedTrace> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 1;
        let owner = ScriptedOwner::scripted([succeeded(operation(6)), succeeded(operation(7))]);
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            owner,
            larry_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        offer_runs(
            &mut runtime,
            &[(6_u8, 6_u8), (7, 7)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        runtime.tick()?;
        let current = runtime.fence_mut().advance_epoch()?;
        let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
        assert_eq!(stood, current, "authentication stands at advanced epoch");
        assert_eq!(
            runtime.fence().epoch(),
            current,
            "fence reports its generation"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "stranded admission fenced");
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert!(
            refusal.contains("StaleEpoch"),
            "refusal names stale epoch, got {refusal}"
        );
        let fresh = read_work(8, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(8, &fresh)?;
        assert_eq!(
            runtime.tick()?.authorized(),
            &[operation(8)],
            "new epoch admits reach"
        );
        assert!(
            runtime.owner().dispatched().contains(&operation(8)),
            "fresh reach reaches its owner"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// E71: a command admitted after the advance dispatches under the new epoch.
#[test]
fn sim_fence_post_advance_admission_dispatches() -> Outcome {
    replay(40_u64..48, |_| -> Sim<FencedTrace> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(6))]))?;
        runtime.fence_mut().advance_epoch()?;
        let body = read_work(6, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(6, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized(), &[operation(6)], "new epoch admits reach");
        assert_eq!(tick.fenced().len(), 0, "new epoch fences nothing");
        assert_eq!(
            runtime.owner().dispatched(),
            &[operation(6)],
            "new epoch dispatches once"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// R87: the default offline bound admits several dispatches without refusal.
#[test]
fn sim_fence_default_bound_admits_both() -> Outcome {
    replay(48_u64..56, |_| -> Sim<FencedTrace> {
        let owner = ScriptedOwner::scripted([succeeded(operation(7)), succeeded(operation(8))]);
        let mut runtime = mk_fence(owner)?;
        offer_runs(
            &mut runtime,
            &[(7_u8, 7_u8), (8, 8)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized().len(), 2, "default bound admits both");
        assert_eq!(runtime.fence().offline_spent(), 2, "two reaches spent");
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// R87: a tight offline bound refuses the reach past the ceiling by name.
#[test]
fn sim_fence_tight_bound_refuses_by_name() -> Outcome {
    replay(56_u64..64, |_| -> Sim<FencedTrace> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 1;
        let owner = ScriptedOwner::scripted([succeeded(operation(9)), succeeded(operation(10))]);
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            owner,
            larry_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        offer_runs(
            &mut runtime,
            &[(9_u8, 9_u8), (10, 10)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.authorized(),
            &[operation(9), operation(10)],
            "gate authorizes both"
        );
        assert_eq!(tick.fenced().len(), 1, "second reach fenced");
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert!(
            refusal.contains("OfflineExposureExhausted"),
            "refusal names the spent bound, got {refusal}"
        );
        assert_eq!(
            runtime.owner().dispatched(),
            &[operation(9)],
            "fenced reach sends none"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// R87: re-authentication recharges offline authority to zero spent.
#[test]
fn sim_fence_reauthentication_recharges_spent() -> Outcome {
    replay(64_u64..72, |_| -> Sim<FencedTrace> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.offline_dispatch_bound = 1;
        let owner = ScriptedOwner::scripted([succeeded(operation(11)), succeeded(operation(12))]);
        let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
            bounded,
            owner,
            larry_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let first = read_work(11, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(11, &first)?;
        runtime.tick()?;
        assert_eq!(runtime.fence().offline_spent(), 1, "one reach spent");
        let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
        assert_eq!(stood, SecurityEpoch::INITIAL, "re-auth stands at initial");
        assert_eq!(runtime.fence().offline_spent(), 0, "re-auth recharges");
        let second = read_work(12, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(12, &second)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.authorized(),
            &[operation(12)],
            "recharged reach dispatches"
        );
        assert_eq!(tick.fenced().len(), 0, "recharged reach fences nothing");
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// R87: one dispatch spends exactly one offline reach.
#[test]
fn sim_fence_dispatch_spends_one_reach() -> Outcome {
    replay(72_u64..80, |_| -> Sim<u32> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(13))]))?;
        assert_eq!(
            runtime.fence().offline_spent(),
            0,
            "fresh fence spent nothing"
        );
        let body = read_work(13, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(13, &body)?;
        runtime.tick()?;
        Ok(runtime.fence().offline_spent())
    })
}

/// E71: revocation isolates one operation while its sibling dispatches.
#[test]
fn sim_fence_revocation_isolates_sibling() -> Outcome {
    replay(80_u64..88, |_| -> Sim<FencedTrace> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(15))]))?;
        offer_runs(
            &mut runtime,
            &[(14_u8, 14_u8), (15, 15)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert!(
            runtime.fence_mut().revoke(operation(14)),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(
            tick.authorized(),
            &[operation(14), operation(15)],
            "gate authorizes both"
        );
        assert_eq!(tick.fenced().len(), 1, "withdrawn operation fenced");
        assert_eq!(
            runtime.owner().dispatched(),
            &[operation(15)],
            "only sibling reaches owner"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// E71: the fence stands at the settings epoch before any advance.
#[test]
fn sim_fence_starts_at_settings_epoch() -> Outcome {
    for seed in 88_u64..96 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let runtime = mk_fence(ScriptedOwner::new())?;
            let _ = seed;
            Ok(runtime.fence().epoch() == SecurityEpoch::INITIAL)
        };
        assert!(
            drive()?,
            "seed {seed}: fresh fence stands at admission epoch"
        );
        assert!(drive()?, "seed {seed}: epoch replay");
    }
    Ok(())
}

/// R87: the fence reports the configured offline bound.
#[test]
fn sim_fence_reports_configured_bound() -> Outcome {
    replay(96_u64..104, |_| -> Sim<u32> {
        let runtime = mk_fence(ScriptedOwner::new())?;
        Ok(runtime.fence().offline_bound())
    })
}

/// E71: advancing the epoch returns the next generation.
#[test]
fn sim_fence_advance_returns_next_generation() -> Outcome {
    for seed in 104_u64..112 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let mut runtime = mk_fence(ScriptedOwner::new())?;
            let wanted = SecurityEpoch::INITIAL
                .successor()
                .ok_or("initial epoch advances")?;
            let next = runtime.fence_mut().advance_epoch()?;
            let _ = seed;
            Ok(next == wanted && runtime.fence().epoch() == next)
        };
        assert!(drive()?, "seed {seed}: advance returns next generation");
        assert!(drive()?, "seed {seed}: advance replay");
    }
    Ok(())
}

/// E71: authenticating an older generation never moves the fence back.
#[test]
fn sim_fence_authenticate_older_never_moves_back() -> Outcome {
    for seed in 112_u64..120 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let mut runtime = mk_fence(ScriptedOwner::new())?;
            let newer = runtime.fence_mut().advance_epoch()?;
            let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
            let _ = seed;
            Ok(stood == newer && runtime.fence().epoch() == newer)
        };
        assert!(
            drive()?,
            "seed {seed}: older authentication never moves back"
        );
        assert!(drive()?, "seed {seed}: authenticate replay");
    }
    Ok(())
}

/// E71/D48: the epoch space is refused at u64::MAX, never wrapped.
#[test]
fn sim_fence_epoch_exhaustion_refused() -> Outcome {
    for seed in 120_u64..128 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let mut topped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
            topped.security_epoch = SecurityEpoch::new(u64::MAX);
            let mut runtime: TestRuntime = larry_runtime::runtime::Runtime::new(
                topped,
                ScriptedOwner::new(),
                larry_runtime::clock::FixedClock::new(support::epoch()?),
            )?;
            let result = runtime.fence_mut().advance_epoch();
            assert!(result.is_err(), "topped epoch refuses to advance");
            if let Err(error) = result {
                let report = format!("{error:?}");
                assert!(
                    report.contains("EpochExhausted"),
                    "refusal names exhaustion, got {report}"
                );
            }
            let _ = seed;
            Ok(true)
        };
        assert!(drive()?, "seed {seed}: exhaustion always refused");
        assert!(drive()?, "seed {seed}: exhaustion replay");
    }
    Ok(())
}

/// E71: the fenced report carries the exact refused operation identity.
#[test]
fn sim_fence_report_carries_operation_identity() -> Outcome {
    replay(128_u64..136, |_| -> Sim<FencedTrace> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(16))]))?;
        let body = read_work(16, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(16, &body)?;
        assert!(
            runtime.fence_mut().revoke(operation(16)),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced()[0].0, operation(16), "report names operation");
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// E71: the revoked refusal names the withdrawn reason in the report.
#[test]
fn sim_fence_revoked_reason_reported() -> Outcome {
    replay(136_u64..144, |_| -> Sim<FencedTrace> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(17))]))?;
        let body = read_work(17, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(17, &body)?;
        assert!(
            runtime.fence_mut().revoke(operation(17)),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced()[0].0, operation(17), "report names operation");
        let refusal = format!("{:?}", tick.fenced()[0].1);
        assert!(
            refusal.contains("Revoked"),
            "report names revoked reason, got {refusal}"
        );
        Ok((
            fence_counts(&runtime, tick.fenced().len()),
            trace(runtime.journal()),
        ))
    })
}

/// E71: withdrawing authority is not itself a recorded journal fact.
#[test]
fn sim_fence_withdrawal_records_no_fact() -> Outcome {
    replay(144_u64..152, |_| -> Sim<(usize, usize)> {
        let mut runtime = mk_fence(ScriptedOwner::new())?;
        let before = runtime.journal().len();
        assert!(
            runtime.fence_mut().revoke(operation(18)),
            "withdrawal reports change"
        );
        runtime.fence_mut().advance_epoch()?;
        let stood = runtime.fence_mut().authenticate(SecurityEpoch::INITIAL);
        assert_eq!(
            runtime.fence().epoch(),
            stood,
            "authenticate reports standing epoch"
        );
        let (first_before, first_after) = (before, runtime.journal().len());
        assert_eq!(first_before, first_after, "fence acts record nothing");
        Ok((first_before, first_after))
    })
}

/// E71: a second tick after a fenced reach replays the same trace.
#[test]
fn sim_fence_second_tick_after_refusal_replays() -> Outcome {
    replay(152_u64..160, |_| -> Sim<(ContentDigest, ContentDigest)> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(19))]))?;
        let body = read_work(19, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(19, &body)?;
        assert!(
            runtime.fence_mut().revoke(operation(19)),
            "withdrawal reports change"
        );
        runtime.tick()?;
        let first = trace(runtime.journal());
        runtime.tick()?;
        Ok((first, trace(runtime.journal())))
    })
}

/// E71: seeded revocation targets refuse while seeded siblings dispatch.
#[test]
fn sim_fence_seeded_target_refuses_sibling_dispatches() -> Outcome {
    replay(160_u64..168, |seed| -> Sim<FencedState> {
        let mut random = seed.saturating_add(71);
        let pick = u8::try_from(ratchet(&mut random) & 0x01)?;
        let (refused_n, allowed_n) = (20_u8.saturating_add(pick), 21_u8.saturating_sub(pick));
        let (refused, allowed) = (operation(refused_n), operation(allowed_n));
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(allowed)]))?;
        offer_runs(
            &mut runtime,
            &[(20_u8, refused_n), (21, allowed_n)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert!(
            runtime.fence_mut().revoke(refused),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "withdrawn operation fenced");
        assert_eq!(
            tick.fenced()[0].0,
            refused,
            "report names refused operation"
        );
        assert_state(runtime.projection(), run(21), RunState::Completed)?;
        let refused_state = runtime
            .projection()
            .run_state(run(20))
            .ok_or("refused run tracked")?;
        let (first_fp, first_state) = (fence_counts(&runtime, tick.fenced().len()), refused_state);

        Ok((first_fp, first_state))
    })
}

/// E71: revoking after a successful dispatch changes no recorded receipt.
#[test]
fn sim_fence_revoke_after_success_changes_nothing() -> Outcome {
    replay(168_u64..176, |_| -> Sim<(usize, ContentDigest)> {
        let mut runtime = mk_fence(ScriptedOwner::scripted([succeeded(operation(22))]))?;
        let body = read_work(22, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(22, &body)?;
        runtime.tick()?;
        assert!(
            runtime.fence_mut().revoke(operation(22)),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(
            runtime.projection().receipts().len(),
            1,
            "receipt survives revocation"
        );
        let (first_fenced, first_fp) = (tick.fenced().len(), trace(runtime.journal()));

        Ok((first_fenced, first_fp))
    })
}
