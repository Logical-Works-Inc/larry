//! Seeded crash-and-restore simulations (E19, E72, E02).
//! Every test crashes the real `Runtime` by taking its journal, restores a
//! fresh runtime, and asserts the same trace on two identical runs.

mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::ContentDigest;
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::ProjectionBackend;
use larry_runtime::projection::replay_authorizations;
use larry_runtime::runtime::restore;
use sim_kit::{Offer, Sim, dispatch, read_work, reader, reopen, replay, trace};
use support::{
    TestRuntime, assert_state, cancel, clock, command_id, operation, proposal, run, runtime_with,
    settings, submit, succeeded, tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Builds the standard `FileRead`/`FileRead`/100 runtime over `owner`.
fn mk_crash(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    reader(owner, 100)
}

/// E19: an unknown outcome at crash time reconciles after restore.
#[test]
fn sim_crash_unknown_reconciles_after_restore() -> Outcome {
    replay(0_u64..8, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([unknown(operation(1))]))?;
        let strat = ReconciliationStrategy::Queryable;
        dispatch(&mut runtime, 1, 1, &[Scope::FileRead], 5, strat)?;
        assert_state(runtime.projection(), run(1), RunState::Reconciling)?;
        let journal = runtime.into_journal();
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(1))]),
            clock()?,
            journal,
        )?;
        assert_state(restored.projection(), run(1), RunState::Reconciling)?;
        assert_eq!(
            restored.tick()?.reconciled(),
            &[operation(1)],
            "restore reconciles"
        );
        assert_state(restored.projection(), run(1), RunState::Completed)?;
        Ok(trace(restored.journal()))
    })
}

/// E02: a published success receipt survives the crash without duplication.
#[test]
fn sim_crash_success_receipt_not_duplicated() -> Outcome {
    replay(8_u64..16, |_| -> Sim<ContentDigest> {
        let owner = ScriptedOwner::scripted([succeeded(operation(2))]);
        let mut runtime = mk_crash(owner)?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 2, 2, &[Scope::FileRead], 5, strat)?;
        let journal = runtime.into_journal();
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(2))]),
            clock()?,
            journal,
        )?;
        assert_eq!(restored.tick()?.receipts().len(), 0, "no second receipt");
        assert_eq!(
            restored.projection().receipts().len(),
            1,
            "one receipt kept"
        );
        assert_state(restored.projection(), run(2), RunState::Completed)?;
        Ok(trace(restored.journal()))
    })
}

/// E72: crashing before any effect restores to an empty, idle runtime.
#[test]
fn sim_crash_empty_journal_restores_idle() -> Outcome {
    replay(16_u64..24, |_| -> Sim<usize> {
        let runtime = mk_crash(ScriptedOwner::new())?;
        let journal = runtime.into_journal();
        assert!(journal.is_empty(), "nothing recorded before the crash");
        let mut restored = reopen(journal, 100)?;
        assert!(restored.tick()?.is_idle(), "restored runtime is idle");
        assert!(restored.journal().is_empty(), "restore records nothing");
        Ok(restored.journal().len())
    })
}

/// E55/E72: restoring the same journal twice folds the same projection.
#[test]
fn sim_crash_same_journal_restores_twice_identically() -> Outcome {
    replay(24_u64..32, |_| -> Sim<(ContentDigest, usize)> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(3))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 3, 3, &[Scope::FileRead], 5, strat)?;
        let journal = runtime.into_journal();
        let first = reopen(journal.clone(), 100)?;
        let second = reopen(journal, 100)?;
        assert_eq!(
            first.projection().receipts(),
            second.projection().receipts(),
            "same receipts"
        );
        let (first_digest, first_count) =
            (trace(first.journal()), first.projection().receipts().len());
        assert_eq!(first_count, 1, "one receipt folded");
        assert_eq!(first_count, 1, "one receipt folded on replay");
        Ok((first_digest, first_count))
    })
}

/// E72: a new run submitted after restore completes and reserves budget.
#[test]
fn sim_crash_new_run_after_restore_completes() -> Outcome {
    replay(32_u64..40, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(4))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 4, 4, &[Scope::FileRead], 10, strat)?;
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(5))]),
            clock()?,
            runtime.into_journal(),
        )?;
        let tick = dispatch(&mut restored, 5, 5, &[Scope::FileRead], 20, strat)?;
        assert_eq!(tick.receipts().len(), 1, "post-restore run receipts");
        assert_state(restored.projection(), run(5), RunState::Completed)?;
        assert_eq!(restored.remaining_budget(), 70, "both reservations held");
        Ok(trace(restored.journal()))
    })
}

/// E19: a reconciling run interrupted mid-reconcile finishes after restore.
#[test]
fn sim_crash_mid_reconcile_finishes_after_restore() -> Outcome {
    replay(40_u64..48, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([unknown(operation(6))]))?;
        let strat = ReconciliationStrategy::Queryable;
        dispatch(&mut runtime, 6, 6, &[Scope::FileRead], 1, strat)?;
        runtime.tick()?;
        assert_state(runtime.projection(), run(6), RunState::Reconciling)?;
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(6))]),
            clock()?,
            runtime.into_journal(),
        )?;
        assert_eq!(
            restored.tick()?.receipts().len(),
            1,
            "restore finishes reconcile"
        );
        assert_state(restored.projection(), run(6), RunState::Completed)?;
        Ok(trace(restored.journal()))
    })
}

/// E72: the open reservation still holds budget after a crash.
#[test]
fn sim_crash_open_reservation_holds_budget() -> Outcome {
    replay(48_u64..56, |_| -> Sim<u64> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([unknown(operation(7))]))?;
        let strat = ReconciliationStrategy::Queryable;
        dispatch(&mut runtime, 7, 7, &[Scope::FileRead], 30, strat)?;
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_state(restored.projection(), run(7), RunState::Reconciling)?;
        Ok(restored.remaining_budget())
    })
}

/// E72: a denied run still folds to Failed after a crash.
#[test]
fn sim_crash_denied_run_stays_failed() -> Outcome {
    replay(56_u64..64, |_| -> Sim<ContentDigest> {
        let allowed = &[Scope::FileRead];
        let mut runtime = runtime_with(ScriptedOwner::new(), allowed, allowed, 10)?;
        let strat = ReconciliationStrategy::Queryable;
        dispatch(&mut runtime, 8, 8, &[Scope::FileRead], 11, strat)?;
        let restored = reopen(runtime.into_journal(), 10)?;
        assert_state(restored.projection(), run(8), RunState::Failed)?;
        assert_eq!(restored.remaining_budget(), 10, "denial reserves nothing");
        Ok(trace(restored.journal()))
    })
}

/// E72: a cancelled run still folds to Cancelled after a crash.
#[test]
fn sim_crash_cancelled_run_stays_cancelled() -> Outcome {
    replay(64_u64..72, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::new())?;
        let strat = ReconciliationStrategy::RequiresHuman;
        let body = proposal(run(9), tenant(1), operation(9), &[], 0, strat);
        runtime.submit(cancel(command_id(9), &body)?)?;
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_state(restored.projection(), run(9), RunState::Cancelled)?;
        assert_eq!(
            restored.projection().receipts().len(),
            0,
            "cancel has no receipt"
        );
        Ok(trace(restored.journal()))
    })
}

/// E72: restoring under a smaller but sufficient capacity keeps history.
#[test]
fn sim_crash_restore_under_smaller_sufficient_capacity() -> Outcome {
    replay(72_u64..80, |_| -> Sim<usize> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(10))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 10, 10, &[Scope::FileRead], 1, strat)?;
        let len = runtime.journal().len();
        let mut shrunk = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        shrunk.journal_capacity = len.saturating_add(4);
        let restored: TestRuntime = restore(
            shrunk,
            ScriptedOwner::new(),
            clock()?,
            runtime.into_journal(),
        )?;
        assert_state(restored.projection(), run(10), RunState::Completed)?;
        Ok(restored.journal().len())
    })
}

/// E28/E72: a pre-crash command resubmitted after restore is a duplicate.
#[test]
fn sim_crash_resubmit_after_restore_is_duplicate() -> Outcome {
    replay(80_u64..88, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(11))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        let body = read_work(11, 1, strat);
        runtime.offer(11, &body)?;
        runtime.tick()?;
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(11))]),
            clock()?,
            runtime.into_journal(),
        )?;
        restored.offer(11, &body)?;
        let tick = restored.tick()?;
        assert_eq!(
            tick.duplicates(),
            &[command_id(11)],
            "resubmit is a duplicate"
        );
        assert!(tick.admitted().is_empty(), "duplicate admits nothing");
        assert_eq!(
            restored.owner().dispatched().len(),
            0,
            "duplicate dispatches none"
        );
        assert_eq!(
            restored.remaining_budget(),
            99,
            "duplicate reserves nothing"
        );
        Ok(trace(restored.journal()))
    })
}

/// E28/E72: a conflicting retry after restore is refused, never folded twice.
#[test]
fn sim_crash_conflicting_retry_after_restore_refused() -> Outcome {
    replay(88_u64..96, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(12))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 12, 12, &[Scope::FileRead], 1, strat)?;
        let mut restored = reopen(runtime.into_journal(), 100)?;
        let changed = read_work(12, 9, strat);
        match restored.submit(submit(command_id(12), &changed)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => assert!(
                matches!(restored.tick(), Err(RuntimeError::Command(_))),
                "conflicting retry refused at tick"
            ),
            Err(other) => return Err(other.into()),
        }
        Ok(trace(restored.journal()))
    })
}

/// E55/E72: restored authorizations still re-derive bit-identically.
#[test]
fn sim_crash_restored_authorizations_replay_identically() -> Outcome {
    replay(96_u64..104, |_| -> Sim<usize> {
        let own = [succeeded(operation(13)), succeeded(operation(14))];
        let mut runtime = mk_crash(ScriptedOwner::scripted(own))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        for (number, effect) in [(13_u8, 13_u8), (14, 14)] {
            dispatch(&mut runtime, number, effect, &[Scope::FileRead], 3, strat)?;
        }
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 100)?;
        let report = replay_authorizations(restored.projection());
        assert!(
            report.is_bit_identical(),
            "restore keeps pinned authorizations"
        );
        assert_eq!(report.divergences(), 0, "no authorization diverges");
        Ok(report.findings().len())
    })
}

/// E02: two receipts recorded before the crash are both present after it.
#[test]
fn sim_crash_two_receipts_survive() -> Outcome {
    replay(104_u64..112, |_| -> Sim<usize> {
        let own = [succeeded(operation(15)), succeeded(operation(16))];
        let mut runtime = mk_crash(ScriptedOwner::scripted(own))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        for (number, effect) in [(15_u8, 15_u8), (16, 16)] {
            dispatch(&mut runtime, number, effect, &[Scope::FileRead], 2, strat)?;
        }
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_state(restored.projection(), run(15), RunState::Completed)?;
        assert_state(restored.projection(), run(16), RunState::Completed)?;
        Ok(restored.projection().receipts().len())
    })
}

/// E72: an empty script after restore leaves the run reconciling, twice alike.
#[test]
fn sim_crash_empty_owner_script_stays_reconciling() -> Outcome {
    replay(112_u64..120, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([unknown(operation(17))]))?;
        let strat = ReconciliationStrategy::Queryable;
        dispatch(&mut runtime, 17, 17, &[Scope::FileRead], 1, strat)?;
        let mut restored = reopen(runtime.into_journal(), 100)?;
        restored.tick()?;
        assert_state(restored.projection(), run(17), RunState::Reconciling)?;
        Ok(trace(restored.journal()))
    })
}

/// E71/E72: a fence refusal before the crash replays the same trace after it.
#[test]
fn sim_crash_fenced_dispatch_replays_same_trace() -> Outcome {
    replay(120_u64..128, |seed| -> Sim<(usize, ContentDigest)> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(18))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        let body = read_work(18, 1, strat);
        runtime.offer(18, &body)?;
        assert!(
            runtime.fence_mut().revoke(operation(18)),
            "withdrawal reports change"
        );
        let tick = runtime.tick()?;
        assert_eq!(tick.fenced().len(), 1, "one mutating reach fenced");
        let before = trace(runtime.journal());
        let restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(18))]),
            clock()?,
            runtime.into_journal(),
        )?;
        assert_eq!(trace(restored.journal()), before, "restore keeps the trace");
        let (first_fenced, first_chain) = (tick.fenced().len(), before);
        assert_eq!(first_fenced, 1, "seed {seed}: one refusal");
        Ok((first_fenced, first_chain))
    })
}

/// E72: journal length is preserved across the crash boundary.
#[test]
fn sim_crash_journal_length_preserved() -> Outcome {
    replay(128_u64..136, |seed| -> Sim<(usize, usize)> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(19))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 19, 19, &[Scope::FileRead], 1, strat)?;
        let before = runtime.journal().len();
        let restored = reopen(runtime.into_journal(), 100)?;
        let (first_before, first_after) = (before, restored.journal().len());
        assert_eq!(first_before, first_after, "seed {seed}: length kept");
        Ok((first_before, first_after))
    })
}

/// E19/E72: two crashes in a row keep the completed run completed.
#[test]
fn sim_crash_two_crashes_keep_completed_run() -> Outcome {
    replay(136_u64..144, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([succeeded(operation(20))]))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        dispatch(&mut runtime, 20, 20, &[Scope::FileRead], 1, strat)?;
        let once = reopen(runtime.into_journal(), 100)?;
        let twice = reopen(once.into_journal(), 100)?;
        assert_state(twice.projection(), run(20), RunState::Completed)?;
        assert_eq!(
            twice.projection().receipts().len(),
            1,
            "one receipt after two crashes"
        );
        Ok(trace(twice.journal()))
    })
}

/// E72: restoring with the default capacity keeps a full small history.
#[test]
fn sim_crash_default_capacity_restore_keeps_history() -> Outcome {
    replay(144_u64..152, |_| -> Sim<usize> {
        let own = [succeeded(operation(21)), succeeded(operation(22))];
        let mut runtime = mk_crash(ScriptedOwner::scripted(own))?;
        let strat = ReconciliationStrategy::IdempotentRetrySafe;
        for (number, effect) in [(21_u8, 21_u8), (22, 22)] {
            dispatch(&mut runtime, number, effect, &[Scope::FileRead], 1, strat)?;
        }
        runtime.tick()?;
        let len = runtime.journal().len();
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_eq!(restored.journal().len(), len, "restore keeps every entry");
        Ok(len)
    })
}

/// E72: an unknown-permanent run still folds to OutcomeUnknown after a crash.
#[test]
fn sim_crash_unknown_permanent_stays_unknown() -> Outcome {
    replay(152_u64..160, |_| -> Sim<ContentDigest> {
        let mut runtime = mk_crash(ScriptedOwner::scripted([unknown(operation(23))]))?;
        let strat = ReconciliationStrategy::RequiresHuman;
        dispatch(&mut runtime, 23, 23, &[Scope::FileRead], 1, strat)?;
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_state(restored.projection(), run(23), RunState::OutcomeUnknown)?;
        Ok(trace(restored.journal()))
    })
}
