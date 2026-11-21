//! larry#329: one tenant's refused command or effect is a typed entry in the
//! tick report, and the tick still reconciles and cancels for every tenant.
//! Only journal and clock refusals end a tick.

mod sim_kit;
mod support;

/// The atomic-compare-and-dispatch owner double the display refinement needs.
#[path = "support/atomic.rs"]
mod atomic;

use std::collections::BTreeMap;
use std::time::Duration;

use atomic::{AtomicOwner, holder};
use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope};
use larry_contracts::binding::{BindingError, BindingField};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, ReconciliationStrategy, TerminalReceipt};
use larry_contracts::event::{Event, EventKind};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::ReplayRecord;
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::{Journal, JournalEntry};
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::CommandRefusal;
use larry_runtime::projection::{EffectPhase, FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{Runtime, TickItem, restore};
use lgwks_std::json::serde::Deserialize;

use sim_kit::Sim;
use support::{
    TestRuntime, assert_state, cancel, clock, command_id, epoch, operation, proposal, run,
    runtime_with, settings, submit, succeeded, tenant, unknown,
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

/// larry#371 gap 1: a pending command whose acceptance record fails typed is
/// reduced once per tick, and the tick moves on to the commands behind it.
///
/// The projection holds one run, so a second run's acceptance can never fold
/// into it: `record_command_accepted` fails with the projection's own
/// `RunLimitReached`, which is no journal refusal and so ends nothing. The
/// command is re-inserted at its place, and the loop must step past it rather
/// than reduce the same slot until the tick's budget runs out.
#[test]
fn sim_a_command_whose_accept_fails_is_reduced_once_and_the_tick_goes_on() -> Outcome {
    sim_kit::replay(0..64, |_seed| -> Sim<(usize, bool, bool)> {
        let mut bounded = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        bounded.run_capacity = 1;
        bounded.max_runs_per_tick = 4;
        let mut rt: TestRuntime = Runtime::new(bounded, ScriptedOwner::new(), clock()?)?;

        rt.submit(submit(
            command_id(1),
            &work(1, 1, 1, 1, ReconciliationStrategy::Queryable),
        )?)?;
        assert!(rt.tick()?.refused().is_empty(), "the first run is admitted");

        let stuck = command_id(2);
        rt.submit(submit(
            stuck,
            &work(2, 1, 2, 1, ReconciliationStrategy::Queryable),
        )?)?;
        let failed = rt.tick()?;
        assert_eq!(
            failed.refused().len(),
            1,
            "the stuck command fails once: {failed:?}"
        );

        let stop = command_id(3);
        rt.submit(cancel(stop, &idle(1, 1, 3))?)?;
        let report = rt.tick()?;
        let once = report.refused().len() == 1;
        assert!(
            once,
            "the stuck command is reduced once, never until the budget runs out: {report:?}"
        );
        assert!(
            report.refused().iter().all(|entry| {
                entry.0 == TickItem::Command(stuck)
                    && matches!(&entry.1, RuntimeError::RunLimitReached { capacity: 1 })
            }),
            "one typed refusal for the stuck command: {report:?}"
        );
        let ran = report.admitted().contains(&stop);
        assert!(ran, "the command behind it still runs: {report:?}");
        Ok((report.refused().len(), ran, once))
    })
}

/// A submit that must be displayed: the atomic owner resolves a target, so an
/// unbound submission is refused.
type ShownRuntime = Runtime<FoldProjection, AtomicOwner>;

/// One host grant covering `who`'s `FileRead` work, held by [`holder`].
fn covering_grant(who: u8) -> Result<Grant, Box<dyn std::error::Error>> {
    Ok(Grant::new(
        tenant(who),
        holder(),
        [Scope::FileRead].into_iter().collect(),
        Budget::new(5, BudgetUnit::Requests),
        epoch()?
            .checked_add(Duration::from_secs(1_000))
            .ok_or("grant expiry overflow")?,
        0,
    )?)
}

/// The runtime the display journeys drive: the first effect's reply is lost,
/// so its run stays open and a correction for that run holds later commands.
fn shown_runtime() -> Result<ShownRuntime, Box<dyn std::error::Error>> {
    let mut owner = AtomicOwner::new(0, 0);
    owner.lose_reply = true;
    Ok(Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        owner,
        clock()?,
    )?)
}

/// A correction for `run_byte`'s run, which holds that run's later commands.
fn steer(
    id: CommandId,
    run_byte: u8,
    who: u8,
    op: u8,
) -> Result<Command, Box<dyn std::error::Error>> {
    let payload = lgwks_std::json::to_vec(&idle(run_byte, who, op))?;
    Ok(Command::new(
        id,
        tenant(who),
        Vec::new(),
        CommandContent::new(CommandKind::Steer, payload.into_boxed_slice()),
    )?)
}

/// The refusal payload an operator reads back from a stored journal. The
/// reason is its stable name, so a restart says why the command never ran.
#[derive(Debug, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
struct RefusalFact {
    /// The command that never ran.
    command: CommandId,
    /// The typed reason.
    reason: String,
}

/// Whether `journal` holds a stale-epoch refusal fact for `command`.
fn refusal_fact(journal: &Journal, command: CommandId) -> bool {
    for entry in journal.entries() {
        let JournalEntry::Event(ref event) = *entry else {
            continue;
        };
        if let Ok(fact) = lgwks_std::json::from_slice::<RefusalFact>(event.payload())
            && fact.command == command
            && fact.reason == "StaleDisplayEpoch"
        {
            return true;
        }
    }
    false
}

/// larry#371 gap 2: a held `Submit` refused because its display's security
/// epoch went stale keeps its durable `CommandAccepted` fact, so the refusal
/// itself must outlive the process. A restart reads it from the journal alone.
#[test]
fn sim_a_stale_display_refusal_survives_a_restart_as_a_journal_fact() -> Outcome {
    sim_kit::replay(0..16, |_seed| -> Sim<(bool, usize)> {
        let mut rt = shown_runtime()?;
        let first = submit(
            command_id(1),
            &work(1, 1, 1, 1, ReconciliationStrategy::Queryable),
        )?;
        let shown = rt.display(&first, holder(), &[covering_grant(1)?])?;
        rt.submit_displayed(first, shown)?;
        rt.tick()?;

        rt.submit(steer(command_id(3), 1, 1, 3)?)?;
        let second = submit(
            command_id(2),
            &work(1, 1, 2, 1, ReconciliationStrategy::Queryable),
        )?;
        let second_id = second.id();
        let shown = rt.display(&second, holder(), &[covering_grant(1)?])?;
        rt.submit_displayed(second, shown)?;
        let held = rt.tick()?;
        assert!(
            held.authorized().is_empty(),
            "the correction holds the submission"
        );

        rt.fence_mut().advance_epoch()?;
        let refused = rt.tick()?;
        assert!(
            refused.refused().iter().any(|entry| {
                entry.0 == TickItem::Command(second_id)
                    && matches!(
                        &entry.1,
                        RuntimeError::Binding(BindingError::Changed {
                            field: BindingField::SecurityEpoch
                        })
                    )
            }),
            "the stale display is a typed refusal: {refused:?}"
        );

        let restored: ShownRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            AtomicOwner::new(0, 0),
            clock()?,
            rt.into_journal(),
        )?;
        let found = refusal_fact(restored.journal(), second_id);
        assert!(
            found,
            "a restart reads why the command never ran from the journal alone"
        );
        assert_eq!(
            restored.projection().refusals(),
            &[(second_id, CommandRefusal::StaleDisplayEpoch)],
            "the restored fold carries the typed refusal"
        );
        Ok((found, refused.refused().len()))
    })
}

/// A projection that refuses to fold run 2's acceptance, standing in for the
/// fold divergence a split between the journal and the projection surfaces.
#[derive(Debug, Default)]
struct Diverging(FoldProjection);

impl ProjectionBackend for Diverging {
    fn with_capacity(capacity: usize) -> Self {
        Self(FoldProjection::new(capacity))
    }

    fn rebuild(&mut self, journal: &Journal) -> Result<(), RuntimeError> {
        self.0.rebuild(journal)
    }

    fn apply(&mut self, event: &Event) -> Result<(), RuntimeError> {
        if event.kind() == EventKind::CommandAccepted && event.run() == run(2) {
            return Err(RuntimeError::FoldDivergence {
                run: event.run(),
                recorded: RunState::Reconciling,
                rederived: RunState::Draft,
            });
        }
        self.0.apply(event)
    }

    fn run_state(&self, run: RunId) -> Option<RunState> {
        self.0.run_state(run)
    }

    fn run_tenant(&self, run: RunId) -> Option<TenantId> {
        self.0.run_tenant(run)
    }

    fn run_count(&self) -> usize {
        self.0.run_count()
    }

    fn effect_phase(&self, operation: OperationId) -> Option<EffectPhase> {
        self.0.effect_phase(operation)
    }

    fn dispatches(&self, operation: OperationId) -> u8 {
        self.0.dispatches(operation)
    }

    fn effect_count(&self, run: RunId) -> usize {
        self.0.effect_count(run)
    }

    fn receipts(&self) -> &[TerminalReceipt] {
        self.0.receipts()
    }

    fn authorizations(&self) -> &[ReplayRecord] {
        self.0.authorizations()
    }

    fn reserved_by_tenant(&self) -> BTreeMap<TenantId, u64> {
        self.0.reserved_by_tenant()
    }

    fn intents(&self) -> &[EffectIntent] {
        self.0.intents()
    }

    fn run_ids(&self) -> Vec<RunId> {
        self.0.run_ids()
    }
}

/// larry#371 gap 3, against ADR 0073: `FoldDivergence` is one command's
/// refusal, never the tick's. The tick records it, the command stays pending,
/// and a second tenant's command queued behind it still dispatches.
#[test]
fn a_fold_divergence_is_a_recorded_refusal_and_the_tick_goes_on() -> Outcome {
    let mut rt: Runtime<Diverging, ScriptedOwner> = Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        clock()?,
    )?;
    rt.submit(submit(
        command_id(1),
        &work(1, 1, 1, 1, ReconciliationStrategy::IdempotentRetrySafe),
    )?)?;
    assert_eq!(rt.tick()?.admitted().len(), 1, "the first run reduces");

    rt.submit(submit(
        command_id(2),
        &work(2, 1, 2, 1, ReconciliationStrategy::IdempotentRetrySafe),
    )?)?;
    let diverged = rt.tick()?;
    assert!(
        diverged.refused().iter().any(|entry| {
            entry.0 == TickItem::Command(command_id(2))
                && matches!(&entry.1, RuntimeError::FoldDivergence { run: diverged_run, .. } if *diverged_run == run(2))
        }),
        "the divergence is a typed refusal for its command: {diverged:?}"
    );

    // The command behind the diverging one, from another tenant, still runs.
    rt.submit(submit(
        command_id(3),
        &work(3, 2, 3, 1, ReconciliationStrategy::IdempotentRetrySafe),
    )?)?;
    let report = rt.tick()?;
    assert_eq!(
        report.refused().len(),
        1,
        "one refusal, never the whole tick budget: {report:?}"
    );
    assert!(
        rt.owner().dispatched().contains(&operation(3)),
        "the second tenant's command still dispatches: {report:?}"
    );
    Ok(())
}
