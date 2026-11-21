//! larry#329: one tenant's refused command or effect is a typed entry in the
//! tick report, and the tick still reconciles and cancels for every tenant.
//! Only journal and clock refusals end a tick.

mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::ProjectionBackend;
use larry_runtime::runtime::{TickItem, restore};

use sim_kit::Sim;
use support::{
    TestRuntime, assert_state, cancel, clock, command_id, operation, proposal, run, runtime_with,
    settings, submit, succeeded, tenant, unknown,
};

/// A failure the test never expects: the tick ended where it must go on.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// The runtime with `FileRead` allowed, a budget of `budget`, and the outcomes
/// `owner` reports in order.
fn runtime(owner: ScriptedOwner, budget: u64) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    runtime_with(owner, &[Scope::FileRead], &[Scope::FileRead], budget)
}

/// A `FileRead` proposal for `run` and `operation`, under `who`.
fn work(
    run_byte: u8,
    who: u8,
    op: u8,
    cost: u64,
    strategy: ReconciliationStrategy,
) -> larry_runtime::payload::WorkProposal {
    proposal(
        run(run_byte),
        tenant(who),
        operation(op),
        &[Scope::FileRead],
        cost,
        strategy,
    )
}

/// A command with no scope and no cost, for a cancellation or a correction.
fn idle(run_byte: u8, who: u8, op: u8) -> larry_runtime::payload::WorkProposal {
    proposal(
        run(run_byte),
        tenant(who),
        operation(op),
        &[],
        0,
        ReconciliationStrategy::RequiresHuman,
    )
}

/// Whether `report` holds a refusal for `item` that `judge` accepts.
fn refused(
    report: &larry_runtime::runtime::TickReport,
    item: TickItem,
    judge: impl Fn(&RuntimeError) -> bool,
) -> bool {
    report
        .refused()
        .iter()
        .any(|entry| entry.0 == item && judge(&entry.1))
}

/// larry#329 wedge 2: a second tenant's command for a run id the first tenant
/// has open is a typed denial, and the first tenant's effect still reconciles
/// in the same tick.
#[test]
fn a_foreign_command_is_a_typed_denial_and_reconciliation_goes_on() -> Outcome {
    let mut rt = runtime(ScriptedOwner::scripted([unknown(operation(1))]), 100)?;
    rt.submit(submit(
        command_id(1),
        &work(1, 1, 1, 1, ReconciliationStrategy::Queryable),
    )?)?;
    rt.tick()?;
    assert_state(rt.projection(), run(1), RunState::Reconciling)?;

    // Tenant 2 names tenant 1's run. The owner answers the open effect next.
    let trespass = submit(command_id(2), &idle(1, 2, 2))?;
    rt.submit(trespass)?;
    let report = rt.tick()?;
    assert!(
        refused(&report, TickItem::Command(command_id(2)), |error| matches!(
            error,
            RuntimeError::RunTenantMismatch { got, .. } if *got == tenant(2)
        )),
        "the foreign command is a typed denial: {report:?}"
    );
    assert_eq!(
        report.reconciled(),
        &[operation(1)],
        "the refusal skips no reconciliation for the run's owner"
    );
    assert_eq!(report.refused().len(), 1, "exactly one denial recorded");
    Ok(())
}

/// larry#329 wedge 2 after a restore: the pending run's tenant is rebuilt from
/// the journal, so a foreign command is still a denial and the restored effect
/// still reconciles.
#[test]
fn a_foreign_command_after_restore_is_a_typed_denial_and_the_tick_goes_on() -> Outcome {
    let mut rt = runtime(ScriptedOwner::scripted([unknown(operation(3))]), 100)?;
    rt.submit(submit(
        command_id(3),
        &work(3, 1, 3, 1, ReconciliationStrategy::Queryable),
    )?)?;
    rt.tick()?;
    assert_state(rt.projection(), run(3), RunState::Reconciling)?;

    let mut restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::scripted([succeeded(operation(3))]),
        clock()?,
        rt.into_journal(),
    )?;
    restored.submit(submit(command_id(4), &idle(3, 2, 4))?)?;
    let report = restored.tick()?;
    assert!(
        refused(&report, TickItem::Command(command_id(4)), |error| matches!(
            error,
            RuntimeError::RunTenantMismatch { expected, got, .. }
                if *expected == tenant(1) && *got == tenant(2)
        )),
        "the restored run keeps its tenant: {report:?}"
    );
    assert_eq!(
        report.reconciled(),
        &[operation(3)],
        "the restored effect reconciles in the same tick"
    );
    Ok(())
}

/// larry#329 wedge 3: a second `Cancel` of a run already `Cancelling` is a
/// typed denial, and cancellation completion still runs for the run.
#[test]
fn a_second_cancel_is_a_typed_denial_and_completion_goes_on() -> Outcome {
    let mut rt = runtime(
        ScriptedOwner::scripted([unknown(operation(5)), unknown(operation(5))]),
        100,
    )?;
    rt.submit(submit(
        command_id(5),
        &work(5, 1, 5, 1, ReconciliationStrategy::Queryable),
    )?)?;
    rt.tick()?;
    rt.submit(cancel(command_id(6), &idle(5, 1, 5))?)?;
    rt.tick()?;
    assert_state(rt.projection(), run(5), RunState::Cancelling)?;

    rt.submit(cancel(command_id(7), &idle(5, 1, 5))?)?;
    let report = rt.tick()?;
    assert!(
        refused(&report, TickItem::Command(command_id(7)), |error| matches!(
            error,
            RuntimeError::Transition(_)
        )),
        "the second cancel is a typed denial: {report:?}"
    );
    assert_state(rt.projection(), run(5), RunState::Cancelling)?;
    assert_eq!(report.refused().len(), 1, "exactly one denial recorded");
    Ok(())
}

/// larry#329 wedge 3: a `Submit` on a non-draft run is a typed denial, and a
/// second tenant's effect queued behind it still dispatches in the same tick.
#[test]
fn a_submit_on_a_non_draft_run_does_not_stop_another_tenants_effect() -> Outcome {
    let mut rt = runtime(
        ScriptedOwner::scripted([unknown(operation(8)), succeeded(operation(9))]),
        100,
    )?;
    rt.submit(submit(
        command_id(8),
        &work(8, 1, 8, 1, ReconciliationStrategy::Queryable),
    )?)?;
    rt.tick()?;
    assert_state(rt.projection(), run(8), RunState::Reconciling)?;

    rt.submit(submit(
        command_id(9),
        &work(8, 1, 9, 1, ReconciliationStrategy::Queryable),
    )?)?;
    rt.submit(submit(
        command_id(10),
        &work(10, 2, 10, 1, ReconciliationStrategy::IdempotentRetrySafe),
    )?)?;
    let report = rt.tick()?;
    assert!(
        refused(&report, TickItem::Command(command_id(9)), |error| matches!(
            error,
            RuntimeError::Transition(_)
        )),
        "the non-draft submit is a typed denial: {report:?}"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(8), operation(10)],
        "the second tenant's effect still dispatches"
    );
    assert!(
        report.receipts().iter().any(|receipt| {
            receipt.operation() == operation(10) && receipt.tenant() == tenant(2)
        })
    );
    Ok(())
}

/// larry#329: one thousand seeds of two tenants sharing a tick. Tenant A's bad
/// command is denied every time, tenant B's effect dispatches every time, and
/// the same seed replays the same counts.
#[test]
fn sim_a_tenants_bad_command_never_stops_another_tenants_effect() -> Outcome {
    sim_kit::replay(0_u64..1_000, |seed| -> Sim<(usize, usize, bool)> {
        let mut rt = runtime(ScriptedOwner::new(), 100)?;
        rt.submit(submit(
            command_id(1),
            &work(1, 1, 1, 1, ReconciliationStrategy::Queryable),
        )?)?;
        rt.tick()?;
        // A repeats a submission for its own non-draft run; B submits its own
        // run behind it in the same ready queue.
        rt.submit(submit(
            command_id(2),
            &work(1, 1, 2, 1, ReconciliationStrategy::Queryable),
        )?)?;
        rt.submit(submit(
            command_id(3),
            &work(3, 2, 3, 1, ReconciliationStrategy::IdempotentRetrySafe),
        )?)?;
        let report = rt.tick()?;
        let denied = report.refused().iter().any(|entry| {
            entry.0 == TickItem::Command(command_id(2))
                && matches!(&entry.1, RuntimeError::Transition(_))
        });
        let dispatched = rt.owner().dispatched().contains(&operation(3));
        let run_a_moved = rt.projection().run_state(run(1)) != Some(RunState::Reconciling);
        assert!(denied, "seed {seed}: A's command is denied typed");
        assert!(dispatched, "seed {seed}: B's effect dispatches");
        assert!(!run_a_moved, "seed {seed}: A's bad command moves no run");
        assert_eq!(
            report.refused().len(),
            1,
            "seed {seed}: one denial, never a cascade"
        );
        Ok((
            report.refused().len(),
            rt.owner().dispatched().len(),
            denied,
        ))
    })
}

/// A tick whose only activity was a recorded denial is not idle: the operator
/// must read the refusal.
#[test]
fn a_refusal_only_tick_is_not_idle() -> Outcome {
    let mut rt = runtime(ScriptedOwner::scripted([succeeded(operation(21))]), 100)?;
    rt.submit(submit(
        command_id(21),
        &work(21, 1, 21, 1, ReconciliationStrategy::Queryable),
    )?)?;
    assert_eq!(rt.tick()?.receipts().len(), 1, "the first run completes");

    rt.submit(submit(command_id(22), &idle(21, 2, 22))?)?;
    let report = rt.tick()?;
    assert_eq!(report.refused().len(), 1, "one recorded denial");
    assert!(report.admitted().is_empty(), "nothing was admitted");
    assert!(
        !report.is_idle(),
        "a tick that recorded a denial is not idle"
    );
    Ok(())
}

/// The report names each denial exactly: a malformed command is a codec
/// refusal against its command, and a second tenant's effect behind it in the
/// same ready queue still dispatches.
#[test]
fn a_per_item_refusal_is_a_typed_entry_and_the_tick_goes_on() -> Outcome {
    use larry_contracts::command::{Command, CommandContent, CommandKind};

    let mut rt = runtime(ScriptedOwner::scripted([succeeded(operation(12))]), 100)?;
    let malformed = Command::new(
        command_id(11),
        tenant(1),
        Vec::new(),
        CommandContent::new(CommandKind::Submit, Box::from(b"{".as_slice())),
    )?;
    rt.submit(malformed)?;
    rt.submit(submit(
        command_id(12),
        &work(12, 2, 12, 1, ReconciliationStrategy::IdempotentRetrySafe),
    )?)?;
    let report = rt.tick()?;
    assert!(
        refused(
            &report,
            TickItem::Command(command_id(11)),
            |error| matches!(error, RuntimeError::Codec(_))
        ),
        "the malformed command is named in the report: {report:?}"
    );
    assert!(
        report
            .receipts()
            .iter()
            .any(|receipt| receipt.operation() == operation(12) && receipt.tenant() == tenant(2)),
        "the effect behind the refusal still runs: {report:?}"
    );
    assert!(
        rt.journal().command(command_id(11)).is_none(),
        "the refused command was never admitted"
    );
    Ok(())
}
