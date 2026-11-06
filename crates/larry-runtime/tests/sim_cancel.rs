//! Seeded cancellation simulations (E30, R72).
//! Every test drives the real `Runtime` twice from one seed through the
//! public cancel surface and asserts the same counts plus the caller-visible
//! cancelled state. Cancellation never fabricates a receipt.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::ProjectionBackend;

use larry_runtime::runtime::TickReport;

use seed::step;
use sim_kit::{Offer, Sim, dispatch, read_work, reader, reopen, replay};
use support::{
    TestRuntime, assert_state, cancel, command_id, operation, proposal, run, succeeded, tenant,
    unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn halt_seed(seed: &mut u64) -> u64 {
    step(seed, 15_919_425_858_406_647_679, 13_957_476_399_865_751, 3)
}

/// Submits one cancellation and ticks once, returning the tick report.
fn nix(
    runtime: &mut TestRuntime,
    command: u8,
    run_op: u8,
) -> Result<TickReport, Box<dyn std::error::Error>> {
    let body = proposal(
        run(run_op),
        tenant(1),
        operation(run_op),
        &[],
        0,
        ReconciliationStrategy::RequiresHuman,
    );
    runtime.submit(cancel(command_id(command), &body)?)?;
    Ok(runtime.tick()?)
}

/// E30: an open Queryable run reconciles, then the cancel completes it.
#[test]
fn sim_cancel_open_run_reconciles_then_completes() -> Outcome {
    replay(0_u64..8, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([unknown(operation(1)), succeeded(operation(1))]),
            100,
        )?;
        dispatch(
            &mut runtime,
            1,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_state(runtime.projection(), run(1), RunState::Reconciling)?;
        assert_eq!(
            nix(&mut runtime, 2, 1)?.reconciled(),
            &[operation(1)],
            "cancel follows reconcile"
        );
        assert_state(runtime.projection(), run(1), RunState::Cancelled)?;
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30: cancelling a completed run keeps its receipt and stays completed.
#[test]
fn sim_cancel_completed_run_keeps_receipt() -> Outcome {
    replay(8_u64..16, |_| -> Sim<(RunState, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(3))]), 100)?;
        dispatch(
            &mut runtime,
            3,
            3,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        nix(&mut runtime, 4, 3)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            1,
            "completed receipt survives cancel"
        );
        let state = runtime
            .projection()
            .run_state(run(3))
            .ok_or("cancelled run is tracked")?;
        let (first_state, first_len) = (state, runtime.journal().len());

        Ok((first_state, first_len))
    })
}

/// E30: a cancel submitted twice dedupes the second sighting.
#[test]
fn sim_cancel_resubmit_dedupes() -> Outcome {
    replay(16_u64..24, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        nix(&mut runtime, 5, 5)?;
        assert_state(runtime.projection(), run(5), RunState::Cancelled)?;
        let body = proposal(
            run(5),
            tenant(1),
            operation(5),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(cancel(command_id(5), &body)?)?;
        assert_eq!(
            runtime.tick()?.duplicates(),
            &[command_id(5)],
            "cancel resubmit dedupes"
        );
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30: submitting work after an untracked cancel is refused as terminal.
#[test]
fn sim_cancel_work_after_untracked_cancel_refused() -> Outcome {
    replay(24_u64..32, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(6))]), 100)?;
        nix(&mut runtime, 6, 6)?;
        assert_state(runtime.projection(), run(6), RunState::Cancelled)?;
        let late = read_work(6, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(7, &late)?;
        assert!(
            matches!(runtime.tick(), Err(RuntimeError::Transition(_))),
            "work after cancel refused"
        );
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E63: a foreign cancel of an open run is refused and the run stays open.
#[test]
fn sim_cancel_foreign_cancel_of_open_run_refused() -> Outcome {
    use larry_contracts::command::{Command, CommandContent, CommandKind};
    replay(32_u64..40, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(8))]), 100)?;
        dispatch(
            &mut runtime,
            8,
            8,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_state(runtime.projection(), run(8), RunState::Reconciling)?;
        let foreign = read_work(8, 1, ReconciliationStrategy::Queryable);
        let command = Command::new(
            command_id(9),
            tenant(2),
            Vec::new(),
            CommandContent::new(
                CommandKind::Cancel,
                lgwks_std::json::to_vec(&foreign)?.into_boxed_slice(),
            ),
        )?;
        runtime.submit(command)?;
        assert!(
            matches!(runtime.tick(), Err(RuntimeError::RunTenantMismatch { .. })),
            "foreign cancel refused"
        );
        assert_state(runtime.projection(), run(8), RunState::Reconciling)?;
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30: a cancel naming a different operation still cancels the run.
#[test]
fn sim_cancel_operation_mismatch_still_cancels_run() -> Outcome {
    replay(40_u64..48, |_| -> Sim<(RunState, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(10))]), 100)?;
        dispatch(
            &mut runtime,
            10,
            10,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let gone = proposal(
            run(10),
            tenant(1),
            operation(11),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(cancel(command_id(11), &gone)?)?;
        runtime.tick()?;
        let state = runtime
            .projection()
            .run_state(run(10))
            .ok_or("cancelled run is tracked")?;
        let (first_state, first_len) = (state, runtime.journal().len());

        Ok((first_state, first_len))
    })
}

/// E30: cancelling a failed run keeps it failed with no new receipt.
#[test]
fn sim_cancel_failed_run_stays_failed() -> Outcome {
    replay(48_u64..56, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 5)?;
        dispatch(
            &mut runtime,
            12,
            12,
            &[Scope::FileRead],
            6,
            ReconciliationStrategy::Queryable,
        )?;
        assert_state(runtime.projection(), run(12), RunState::Failed)?;
        nix(&mut runtime, 13, 12)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            0,
            "cancel fabricates no receipt"
        );
        assert_state(runtime.projection(), run(12), RunState::Failed)?;
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30: cancelling a completed run keeps it completed with its receipt.
#[test]
fn sim_cancel_same_tenant_open_run_cancels() -> Outcome {
    replay(56_u64..64, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(14))]), 100)?;
        dispatch(
            &mut runtime,
            14,
            14,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        nix(&mut runtime, 15, 14)?;
        assert_state(runtime.projection(), run(14), RunState::Completed)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            1,
            "completed receipt survives cancel"
        );
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30: a command identity of zero cancels exactly like any other identity.
#[test]
fn sim_cancel_zero_identity_cancels() -> Outcome {
    replay(64_u64..72, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        nix(&mut runtime, 0, 0)?;
        assert_state(runtime.projection(), run(0), RunState::Cancelled)?;
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E72/E30: a cancelled run restored from the journal stays cancelled. The
/// query answers in the cancel tick, so the effect reconciles with its own
/// receipt before the cancellation completes (larry#119).
#[test]
fn sim_cancel_restored_stays_cancelled() -> Outcome {
    replay(72_u64..80, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([unknown(operation(16)), succeeded(operation(16))]),
            100,
        )?;
        dispatch(
            &mut runtime,
            16,
            16,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        nix(&mut runtime, 17, 16)?;
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_state(restored.projection(), run(16), RunState::Cancelled)?;
        assert_eq!(
            restored.projection().receipts().len(),
            1,
            "the reconciled effect keeps its receipt and cancel adds none"
        );
        Ok((
            restored.projection().receipts().len(),
            restored.journal().len(),
        ))
    })
}

/// E30: cancelling costs no budget and reserves nothing.
#[test]
fn sim_cancel_costs_no_budget() -> Outcome {
    replay(80_u64..88, |seed| -> Sim<(u64, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        nix(&mut runtime, 17, 17)?;
        assert_eq!(runtime.remaining_budget(), 100, "cancel reserves nothing");
        let (first_budget, first_len) = (runtime.remaining_budget(), runtime.journal().len());
        assert_eq!(first_budget, 100, "seed {seed}: cancel costs nothing");
        Ok((first_budget, first_len))
    })
}

/// E28/E30: a duplicate submit before the cancel still cancels once.
#[test]
fn sim_cancel_after_duplicate_submit_cancels_once() -> Outcome {
    replay(88_u64..96, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(18))]), 100)?;
        let body = read_work(18, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(18, &body)?;
        runtime.offer(18, &body)?;
        nix(&mut runtime, 19, 18)?;
        assert_state(runtime.projection(), run(18), RunState::Completed)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            1,
            "completed receipt survives cancel"
        );
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30: the cancel tick timing still converges on Cancelled. A cancel landing
/// in the dispatch tick holds `Cancelling` until the next tick reconciles the
/// unknown; it never completes over an effect with no receipt (larry#119).
#[test]
fn sim_cancel_timing_decides_reconcile_before_cancel() -> Outcome {
    replay(96_u64..104, |seed| -> Sim<(usize, usize)> {
        let mut random = seed.saturating_add(21);
        let early = halt_seed(&mut random) & 1 == 0;
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([unknown(operation(20)), succeeded(operation(20))]),
            100,
        )?;
        let body = read_work(20, 1, ReconciliationStrategy::Queryable);
        runtime.offer(20, &body)?;
        if !early {
            runtime.tick()?;
        }
        nix(&mut runtime, 21, 20)?;
        if early {
            assert_state(runtime.projection(), run(20), RunState::Cancelling)?;
            runtime.tick()?;
        }
        assert_state(runtime.projection(), run(20), RunState::Cancelled)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            1,
            "seed {seed}: the effect reconciled before the cancel completed"
        );
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30/E56: an unknown RequiresHuman run stays OutcomeUnknown under cancel.
#[test]
fn sim_cancel_requires_human_unknown_cancels() -> Outcome {
    replay(104_u64..112, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(22))]), 100)?;
        dispatch(
            &mut runtime,
            22,
            22,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::RequiresHuman,
        )?;
        assert_state(runtime.projection(), run(22), RunState::OutcomeUnknown)?;
        nix(&mut runtime, 23, 22)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            1,
            "the permanent receipt stands"
        );
        assert_state(runtime.projection(), run(22), RunState::OutcomeUnknown)?;
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}

/// E30/INVARIANTS: an observed unknown with no receipt holds cancellation open.
/// A `Queryable` unknown is `Observed { Unknown }` and stays unresolved, with
/// no receipt, so the run must not reach `Cancelled` while it is open.
#[test]
fn sim_cancel_unresolved_observed_effect_holds_cancellation_open() -> Outcome {
    replay(128_u64..192, |_| -> Sim<(RunState, usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(28))]), 100)?;
        dispatch(
            &mut runtime,
            28,
            28,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_state(runtime.projection(), run(28), RunState::Reconciling)?;
        assert_eq!(
            runtime.projection().receipts().len(),
            0,
            "the unknown has no receipt"
        );
        nix(&mut runtime, 29, 28)?;
        let state = runtime
            .projection()
            .run_state(run(28))
            .ok_or("run is tracked")?;
        assert_ne!(
            state,
            RunState::Cancelled,
            "cancel must not complete while an unknown effect has no receipt"
        );
        let (receipts, len) = (
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        );
        let restored = reopen(runtime.into_journal(), 100)?;
        assert_state(restored.projection(), run(28), RunState::Cancelling)?;
        Ok((state, receipts, len))
    })
}

/// E30: three quiet ticks after a cancel add no receipt and change nothing.
#[test]
fn sim_cancel_quiet_ticks_change_nothing() -> Outcome {
    replay(112_u64..120, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        nix(&mut runtime, 23, 23)?;
        let len = runtime.journal().len();
        runtime.tick()?;
        runtime.tick()?;
        runtime.tick()?;
        assert_eq!(runtime.journal().len(), len, "quiet ticks record nothing");
        assert_eq!(runtime.projection().receipts().len(), 0, "no receipt ever");
        assert_state(runtime.projection(), run(23), RunState::Cancelled)?;
        Ok((len, runtime.journal().len()))
    })
}

/// E30: submit and cancel in the same tick resolve deterministically.
#[test]
fn sim_cancel_same_tick_submit_and_cancel() -> Outcome {
    replay(120_u64..128, |_| -> Sim<(RunState, usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(24))]), 100)?;
        let body = read_work(24, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(24, &body)?;
        let gone = proposal(
            run(24),
            tenant(1),
            operation(24),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(cancel(command_id(25), &gone)?)?;
        runtime.tick()?;
        let state = runtime
            .projection()
            .run_state(run(24))
            .ok_or("run is tracked")?;
        let (first_state, first_receipts, first_len) = (
            state,
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        );

        Ok((first_state, first_receipts, first_len))
    })
}

/// E30/R72: over an owner that never answers, every seeded cancel delay holds
/// the run in `Cancelling` with no receipt, and the hold is bounded: once the
/// reconciliation budget is spent a quiet tick records nothing (larry#119).
#[test]
fn sim_cancel_seeded_delays_hold_unresolved_open() -> Outcome {
    replay(128_u64..136, |seed| -> Sim<(usize, usize)> {
        let mut random = seed.saturating_add(33);
        let delay = usize::try_from(halt_seed(&mut random) & 3)?;
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([
                unknown(operation(26)),
                unknown(operation(26)),
                unknown(operation(26)),
                unknown(operation(26)),
            ]),
            100,
        )?;
        let body = read_work(26, 1, ReconciliationStrategy::Queryable);
        runtime.offer(26, &body)?;
        for _ in 0..delay {
            runtime.tick()?;
        }
        nix(&mut runtime, 27, 26)?;
        for _ in 0..=larry_contracts::effect::MAX_EFFECT_ATTEMPTS {
            runtime.tick()?;
        }
        assert_state(runtime.projection(), run(26), RunState::Cancelling)?;
        let held = runtime.journal().len();
        assert!(
            runtime.tick()?.is_idle(),
            "seed {seed}: a spent hold is quiet"
        );
        assert_eq!(
            runtime.journal().len(),
            held,
            "seed {seed}: a spent hold records nothing"
        );
        assert_eq!(
            runtime.projection().receipts().len(),
            0,
            "seed {seed}: an unanswered effect is never given a receipt"
        );
        Ok((
            runtime.projection().receipts().len(),
            runtime.journal().len(),
        ))
    })
}
