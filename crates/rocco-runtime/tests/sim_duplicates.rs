//! Seeded duplicate and conflicting-retry simulations (E28).
//! Every test resubmits a recorded identity and asserts the same trace plus
//! the suppression or refusal the second sighting produces.

mod sim_kit;
mod support;

use rocco_contracts::authority::Scope;
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use rocco_contracts::ids::ContentDigest;
use rocco_contracts::state::RunState;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::ProjectionBackend;

use sim_kit::{Offer, Sim, dispatch, read_work, reader, replay, trace, work};
use support::{
    TestRuntime, assert_state, cancel, command_id, operation, proposal, run, submit, succeeded,
    tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn mix_u64(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(3_931_663_978_901_238_133)
        .wrapping_add(2_862_931_710_398_331_809);
    seed.rotate_left(17)
}

/// Standard-budget FileRead runtime over `owner`.
fn mk_dupe(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    reader(owner, 100)
}

/// Submits a conflicting retry and asserts the typed refusal at whichever
/// stage reports it, returning the trace.
fn conflicting_retry(
    runtime: &mut TestRuntime,
) -> Result<ContentDigest, Box<dyn std::error::Error>> {
    let changed = read_work(4, 9, ReconciliationStrategy::IdempotentRetrySafe);
    match runtime.submit(submit(command_id(4), &changed)?) {
        Err(RuntimeError::Command(_)) => {}
        Ok(()) => assert!(
            matches!(runtime.tick(), Err(RuntimeError::Command(_))),
            "conflicting retry refused at tick"
        ),
        Err(other) => return Err(other.into()),
    }
    Ok(trace(runtime.journal()))
}

/// E28: the same identity submitted twice before one tick admits once.
#[test]
fn sim_duplicates_same_tick_admits_once() -> Outcome {
    replay(0_u64..8, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(1))]))?;
        let body = read_work(1, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(1, &body)?;
        rt.offer(1, &body)?;
        let tick = rt.tick()?;
        assert_eq!(tick.admitted(), &[command_id(1)], "first sighting admits");
        assert_eq!(
            tick.duplicates(),
            &[command_id(1)],
            "second sighting dedupes"
        );
        assert_eq!(rt.owner().dispatched().len(), 1, "one dispatch total");
        Ok(trace(rt.journal()))
    })
}

/// E28: the same content on the next tick is suppressed, not folded twice.
#[test]
fn sim_duplicates_next_tick_suppressed() -> Outcome {
    replay(8_u64..16, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(2))]))?;
        dispatch(
            &mut rt,
            2,
            2,
            &[Scope::FileRead],
            2,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let body = read_work(2, 2, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(2, &body)?;
        let second = rt.tick()?;
        assert_eq!(second.duplicates(), &[command_id(2)], "resubmit dedupes");
        assert!(second.admitted().is_empty(), "resubmit admits nothing");
        assert_eq!(rt.owner().dispatched().len(), 1, "no second dispatch");
        assert_eq!(rt.remaining_budget(), 98, "no second reservation");
        Ok(trace(rt.journal()))
    })
}

/// E28: repeated resubmits stay duplicates across many ticks.
#[test]
fn sim_duplicates_repeated_resubmits_stable() -> Outcome {
    replay(16_u64..24, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(3))]))?;
        dispatch(
            &mut rt,
            3,
            3,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        for _ in 0..5 {
            let body = read_work(3, 1, ReconciliationStrategy::IdempotentRetrySafe);
            rt.offer(3, &body)?;
            assert_eq!(
                rt.tick()?.duplicates(),
                &[command_id(3)],
                "every resubmit dedupes"
            );
        }
        assert_eq!(rt.owner().dispatched().len(), 1, "one dispatch total");
        assert_eq!(rt.projection().receipts().len(), 1, "one receipt total");
        Ok(trace(rt.journal()))
    })
}

/// E28: same identity with different content is refused, never folded.
#[test]
fn sim_duplicates_conflicting_content_refused() -> Outcome {
    replay(24_u64..32, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(4))]))?;
        dispatch(
            &mut rt,
            4,
            4,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        conflicting_retry(&mut rt)
    })
}

/// E28: same identity with a different cost is refused as a conflict.
#[test]
fn sim_duplicates_conflicting_cost_refused() -> Outcome {
    replay(32_u64..40, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(5))]))?;
        dispatch(
            &mut rt,
            5,
            5,
            &[Scope::FileRead],
            4,
            ReconciliationStrategy::Queryable,
        )?;
        let changed = read_work(5, 5, ReconciliationStrategy::Queryable);
        match rt.submit(submit(command_id(5), &changed)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => assert!(
                matches!(rt.tick(), Err(RuntimeError::Command(_))),
                "conflicting cost refused at tick"
            ),
            Err(other) => return Err(other.into()),
        }
        assert_eq!(rt.remaining_budget(), 96, "original reservation stands");
        Ok(trace(rt.journal()))
    })
}

/// E28: same identity with a different strategy is refused as a conflict.
#[test]
fn sim_duplicates_conflicting_strategy_refused() -> Outcome {
    replay(40_u64..48, |_| -> Sim<ContentDigest> {
        let owner = ScriptedOwner::scripted([unknown(operation(6)), succeeded(operation(6))]);
        let mut rt = mk_dupe(owner)?;
        dispatch(
            &mut rt,
            6,
            6,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        let changed = read_work(6, 1, ReconciliationStrategy::RequiresHuman);
        match rt.submit(submit(command_id(6), &changed)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => assert!(
                matches!(rt.tick(), Err(RuntimeError::Command(_))),
                "conflicting strategy refused at tick"
            ),
            Err(other) => return Err(other.into()),
        }
        Ok(trace(rt.journal()))
    })
}

/// E28/E63: the same command identity from another tenant is refused.
#[test]
fn sim_duplicates_cross_tenant_identity_refused() -> Outcome {
    replay(48_u64..56, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(7))]))?;
        dispatch(
            &mut rt,
            7,
            7,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let foreign = work(7, 2, 7, 1, ReconciliationStrategy::IdempotentRetrySafe);
        match rt.submit(submit(command_id(7), &foreign)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => {
                let result = rt.tick();
                assert!(
                    matches!(
                        &result,
                        Err(RuntimeError::Command(_)) | Err(RuntimeError::RunTenantMismatch { .. })
                    ),
                    "cross-tenant identity refused at tick, got {result:?}"
                );
            }
            Err(other) => return Err(other.into()),
        }
        assert_state(rt.projection(), run(7), RunState::Completed)?;
        Ok(trace(rt.journal()))
    })
}

/// E28: identical bytes under a different envelope tenant are a conflict.
#[test]
fn sim_duplicates_envelope_tenant_change_is_conflict() -> Outcome {
    replay(56_u64..64, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(8))]))?;
        let body = read_work(8, 1, ReconciliationStrategy::IdempotentRetrySafe);
        let payload = lgwks_std::json::to_vec(&body)?;
        dispatch(
            &mut rt,
            8,
            8,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let relabelled = Command::new(
            command_id(8),
            tenant(2),
            Vec::new(),
            CommandContent::new(CommandKind::Submit, payload.into_boxed_slice()),
        )?;
        match rt.submit(relabelled) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => {
                let result = rt.tick();
                assert!(
                    matches!(
                        &result,
                        Err(RuntimeError::Command(_)) | Err(RuntimeError::RunTenantMismatch { .. })
                    ),
                    "relabelled envelope refused at tick, got {result:?}"
                );
            }
            Err(other) => return Err(other.into()),
        }
        Ok(trace(rt.journal()))
    })
}

/// E28: a duplicate cancel is suppressed and the run stays cancelled.
#[test]
fn sim_duplicates_cancel_resubmit_suppressed() -> Outcome {
    replay(64_u64..72, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::new())?;
        let body = proposal(
            run(9),
            tenant(1),
            operation(9),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        rt.submit(cancel(command_id(9), &body)?)?;
        rt.tick()?;
        rt.submit(cancel(command_id(9), &body)?)?;
        let second = rt.tick()?;
        assert_eq!(
            second.duplicates(),
            &[command_id(9)],
            "cancel resubmit dedupes"
        );
        assert_state(rt.projection(), run(9), RunState::Cancelled)?;
        assert_eq!(
            rt.projection().receipts().len(),
            0,
            "cancel receipts nothing"
        );
        Ok(trace(rt.journal()))
    })
}

/// E28: duplicates never consume budget twice.
#[test]
fn sim_duplicates_hold_budget_constant() -> Outcome {
    replay(72_u64..80, |seed| -> Sim<(u64, ContentDigest)> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(10))]))?;
        dispatch(
            &mut rt,
            10,
            10,
            &[Scope::FileRead],
            7,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let before = rt.remaining_budget();
        let body = read_work(10, 7, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(10, &body)?;
        rt.tick()?;
        assert_eq!(rt.remaining_budget(), before, "duplicate reserves nothing");
        let (first_budget, first_fp) = (before, trace(rt.journal()));
        assert_eq!(first_budget, 93, "seed {seed}: one reservation");
        Ok((first_budget, first_fp))
    })
}

/// E28: duplicates never redispatch the owner.
#[test]
fn sim_duplicates_never_redispatch() -> Outcome {
    replay(80_u64..88, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(11))]))?;
        dispatch(
            &mut rt,
            11,
            11,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        for _ in 0..2 {
            let body = read_work(11, 1, ReconciliationStrategy::IdempotentRetrySafe);
            rt.offer(11, &body)?;
            rt.tick()?;
        }
        assert_eq!(
            rt.owner().dispatched(),
            &[operation(11)],
            "one dispatch total"
        );
        Ok(trace(rt.journal()))
    })
}

/// E28: a duplicate under Queryable triggers no new reconciliation.
#[test]
fn sim_duplicates_queryable_triggers_no_reconcile() -> Outcome {
    replay(88_u64..96, |_| -> Sim<ContentDigest> {
        let owner = ScriptedOwner::scripted([unknown(operation(12)), succeeded(operation(12))]);
        let mut rt = mk_dupe(owner)?;
        dispatch(
            &mut rt,
            12,
            12,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        let body = read_work(12, 1, ReconciliationStrategy::Queryable);
        rt.offer(12, &body)?;
        let tick = rt.tick()?;
        assert_eq!(tick.duplicates(), &[command_id(12)], "resubmit dedupes");
        assert_eq!(
            tick.reconciled(),
            &[operation(12)],
            "duplicate disturbs no reconcile"
        );
        assert_eq!(tick.receipts().len(), 1, "reconcile still receipts once");
        assert_state(rt.projection(), run(12), RunState::Completed)?;
        Ok(trace(rt.journal()))
    })
}

/// E28: the receipt count is unchanged by duplicate resubmits.
#[test]
fn sim_duplicates_receipt_count_stable() -> Outcome {
    replay(96_u64..104, |_| -> Sim<usize> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(13))]))?;
        dispatch(
            &mut rt,
            13,
            13,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let body = read_work(13, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(13, &body)?;
        assert_eq!(
            rt.tick()?.duplicates(),
            &[command_id(13)],
            "resubmit dedupes"
        );
        Ok(rt.projection().receipts().len())
    })
}

/// E28: a duplicate cancel of an untracked run stays suppressed.
#[test]
fn sim_duplicates_untracked_cancel_resubmit() -> Outcome {
    replay(104_u64..112, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::new())?;
        let body = proposal(
            run(14),
            tenant(1),
            operation(14),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        rt.submit(cancel(command_id(14), &body)?)?;
        rt.tick()?;
        rt.submit(cancel(command_id(14), &body)?)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.duplicates(),
            &[command_id(14)],
            "cancel resubmit dedupes"
        );
        assert_state(rt.projection(), run(14), RunState::Cancelled)?;
        Ok(trace(rt.journal()))
    })
}

/// E28: seeded identities duplicate-suppress across distinct tenants.
#[test]
fn sim_duplicates_seeded_identities_suppress() -> Outcome {
    replay(112_u64..120, |seed| -> Sim<ContentDigest> {
        let mut random = seed.saturating_add(11);
        let byte = u8::try_from(mix_u64(&mut random) & 0xFF)?;
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(byte))]))?;
        dispatch(
            &mut rt,
            byte,
            byte,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let body = work(
            byte,
            1,
            byte,
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        rt.offer(byte, &body)?;
        assert_eq!(
            rt.tick()?.duplicates(),
            &[command_id(byte)],
            "seeded resubmit dedupes"
        );
        assert_state(rt.projection(), run(byte), RunState::Completed)?;
        Ok(trace(rt.journal()))
    })
}

/// E28: a duplicate after a terminal receipt publishes no second receipt.
#[test]
fn sim_duplicates_after_terminal_no_second_receipt() -> Outcome {
    replay(120_u64..128, |_| -> Sim<ContentDigest> {
        let owner = ScriptedOwner::scripted([EffectOutcome::Failed {
            operation: operation(15),
        }]);
        let mut rt = mk_dupe(owner)?;
        dispatch(
            &mut rt,
            15,
            15,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_state(rt.projection(), run(15), RunState::Failed)?;
        let body = read_work(15, 1, ReconciliationStrategy::Queryable);
        rt.offer(15, &body)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.duplicates(),
            &[command_id(15)],
            "post-terminal resubmit dedupes"
        );
        assert_eq!(rt.projection().receipts().len(), 1, "one receipt total");
        Ok(trace(rt.journal()))
    })
}

/// E28: duplicates of two commands in one tick suppress both.
#[test]
fn sim_duplicates_two_commands_one_tick() -> Outcome {
    replay(128_u64..136, |_| -> Sim<ContentDigest> {
        let owner = ScriptedOwner::scripted([succeeded(operation(16)), succeeded(operation(17))]);
        let mut rt = mk_dupe(owner)?;
        for number in [16_u8, 17] {
            dispatch(
                &mut rt,
                number,
                number,
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            )?;
        }
        rt.tick()?;
        for number in [16_u8, 17] {
            let body = work(
                number,
                1,
                number,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            rt.offer(number, &body)?;
        }
        let tick = rt.tick()?;
        assert_eq!(
            tick.duplicates(),
            &[command_id(16), command_id(17)],
            "both resubmits dedupe"
        );
        assert_eq!(rt.owner().dispatched().len(), 2, "no new dispatches");
        Ok(trace(rt.journal()))
    })
}

/// E28: a duplicate appends no command entry, so the length is fixed.
#[test]
fn sim_duplicates_journal_length_fixed() -> Outcome {
    replay(136_u64..144, |seed| -> Sim<(usize, usize)> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(18))]))?;
        dispatch(
            &mut rt,
            18,
            18,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let before = rt.journal().len();
        let body = read_work(18, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(18, &body)?;
        rt.tick()?;
        assert_eq!(rt.journal().len(), before, "duplicate records no new fact");
        let (first_before, first_after) = (before, rt.journal().len());
        assert_eq!(first_before, first_after, "seed {seed}: fixed length");
        Ok((first_before, first_after))
    })
}

/// E28: a conflicting retry after a successful tick refuses at one stage.
#[test]
fn sim_duplicates_conflict_after_success() -> Outcome {
    replay(144_u64..152, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::scripted([succeeded(operation(19))]))?;
        dispatch(
            &mut rt,
            19,
            19,
            &[Scope::FileRead],
            2,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let changed = read_work(19, 3, ReconciliationStrategy::IdempotentRetrySafe);
        match rt.submit(submit(command_id(19), &changed)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => assert!(
                matches!(rt.tick(), Err(RuntimeError::Command(_))),
                "conflicting cost refused at tick"
            ),
            Err(other) => return Err(other.into()),
        }
        assert_eq!(rt.projection().receipts().len(), 1, "one receipt total");
        Ok(trace(rt.journal()))
    })
}

/// E28: a conflicting cancel identity is refused, never folded.
#[test]
fn sim_duplicates_conflicting_cancel_refused() -> Outcome {
    replay(152_u64..160, |_| -> Sim<ContentDigest> {
        let mut rt = mk_dupe(ScriptedOwner::new())?;
        let body = proposal(
            run(20),
            tenant(1),
            operation(20),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        rt.submit(cancel(command_id(20), &body)?)?;
        rt.tick()?;
        let changed = proposal(
            run(20),
            tenant(1),
            operation(21),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        match rt.submit(cancel(command_id(20), &changed)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => assert!(
                matches!(rt.tick(), Err(RuntimeError::Command(_))),
                "conflicting cancel refused at tick"
            ),
            Err(other) => return Err(other.into()),
        }
        assert_state(rt.projection(), run(20), RunState::Cancelled)?;
        Ok(trace(rt.journal()))
    })
}
