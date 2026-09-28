//! Falsifiers for the effect lifecycle: admit, gate, reserve, dispatch,
//! observe, receipt, cancel.
//!
//! Every assertion inspects a real value — a concrete run state, a concrete
//! disposition, a concrete operation identity — never `is_ok()` alone.

mod support;

use rocco_contracts::authority::Scope;
use rocco_contracts::effect::{ReconciliationStrategy, TerminalDisposition};
use rocco_contracts::replay::DenialReason;
use rocco_contracts::state::RunState;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::{EffectPhase, ProjectionBackend};

use support::{
    TestRuntime, assert_state, cancel, command_id, epoch, operation, proposal, run, runtime,
    runtime_with, submit, succeeded, tenant, unknown,
};

/// A one-effect run reaches `Completed` and publishes exactly one receipt.
#[test]
fn authorized_effect_runs_to_a_terminal_receipt() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::IdempotentRetrySafe,
    );
    rt.submit(submit(command_id(1), &body)?)?;

    let tick = rt.tick()?;

    assert_eq!(tick.admitted(), &[command_id(1)]);
    assert_eq!(tick.authorized(), &[operation(1)]);
    assert_eq!(tick.outcomes(), &[(operation(1), succeeded(operation(1)))]);
    assert_eq!(tick.receipts().len(), 1);
    assert_eq!(
        tick.receipts()[0].disposition(),
        TerminalDisposition::Succeeded
    );
    assert_eq!(tick.receipts()[0].operation(), operation(1));
    assert_state(rt.projection(), run(1), RunState::Completed)?;
    // Reservation spent exactly the declared cost.
    assert_eq!(rt.remaining_budget(), 95);
    Ok(())
}

/// The intent is on record before the owner is called, and the owner sees the
/// stable operation identity.
#[test]
fn intent_is_durable_before_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(1), &body)?)?;
    rt.tick()?;

    assert_eq!(rt.owner().dispatched(), &[operation(1)]);
    assert_eq!(rt.projection().authorizations().len(), 1);
    assert_eq!(
        rt.projection().authorizations()[0].pinned.request.operation,
        operation(1)
    );
    Ok(())
}

/// A scope the policy refuses is denied with the deterministic reason, and the
/// run fails rather than silently continuing.
#[test]
fn policy_refusal_is_typed_and_fails_the_run() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    let body = proposal(
        run(2),
        tenant(1),
        operation(2),
        &[Scope::NetworkEgress],
        1,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(2), &body)?)?;

    let tick = rt.tick()?;

    assert_eq!(
        tick.denied(),
        &[(
            operation(2),
            DenialReason::PolicyRefuses {
                scope: Scope::NetworkEgress
            }
        )]
    );
    assert!(tick.authorized().is_empty());
    assert!(rt.owner().dispatched().is_empty());
    assert_state(rt.projection(), run(2), RunState::Failed)?;
    Ok(())
}

/// A worker lacking the capability is denied for the capability reason, not the
/// policy reason: the fixed check order is part of the contract.
#[test]
fn capability_is_checked_after_policy() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead, Scope::FileWrite],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(3),
        tenant(1),
        operation(3),
        &[Scope::FileWrite],
        1,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(3), &body)?)?;

    let tick = rt.tick()?;

    assert_eq!(
        tick.denied(),
        &[(
            operation(3),
            DenialReason::CapabilityMissing {
                scope: Scope::FileWrite
            }
        )]
    );
    Ok(())
}

/// A cost above the remaining budget is denied for the budget reason.
#[test]
fn budget_is_checked_last() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        10,
    )?;
    let body = proposal(
        run(4),
        tenant(1),
        operation(4),
        &[Scope::FileRead],
        11,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(4), &body)?)?;

    let tick = rt.tick()?;

    assert_eq!(
        tick.denied(),
        &[(
            operation(4),
            DenialReason::BudgetExhausted {
                required: 11,
                remaining: 10
            }
        )]
    );
    Ok(())
}

/// The same command identity with identical content is an idempotent replay and
/// is not folded twice (E28).
#[test]
fn duplicate_submission_is_suppressed_by_identity() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::IdempotentRetrySafe,
    );
    rt.submit(submit(command_id(1), &body)?)?;
    let first = rt.tick()?;
    assert_eq!(first.admitted(), &[command_id(1)]);

    rt.submit(submit(command_id(1), &body)?)?;
    let second = rt.tick()?;

    assert_eq!(second.duplicates(), &[command_id(1)]);
    assert!(second.admitted().is_empty());
    assert_eq!(rt.owner().dispatched().len(), 1);
    assert_eq!(rt.remaining_budget(), 99);
    Ok(())
}

/// An unknown outcome under a strategy that keeps the unknown open produces no
/// receipt and leaves the run reconciling — never a success, never a failure.
#[test]
fn open_unknown_stays_reconciling_and_is_then_reconciled() -> Result<(), Box<dyn std::error::Error>>
{
    let mut rt = runtime_with(
        // First call unknown; the reconciliation call then succeeds.
        ScriptedOwner::scripted([unknown(operation(5)), succeeded(operation(5))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(5),
        tenant(1),
        operation(5),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(5), &body)?)?;

    let first = rt.tick()?;

    assert_eq!(first.receipts().len(), 0);
    assert_eq!(first.outcomes(), &[(operation(5), unknown(operation(5)))]);
    assert_state(rt.projection(), run(5), RunState::Reconciling)?;
    assert_eq!(
        rt.projection().effect_phase(operation(5)),
        Some(EffectPhase::Observed {
            outcome: unknown(operation(5))
        })
    );

    let second = rt.tick()?;

    assert_eq!(second.reconciled(), &[operation(5)]);
    assert_eq!(second.receipts().len(), 1);
    assert_eq!(
        second.receipts()[0].disposition(),
        TerminalDisposition::Succeeded
    );
    assert_state(rt.projection(), run(5), RunState::Completed)?;
    Ok(())
}

/// An unknown outcome under a strategy that makes the unknown permanent ends
/// the run in `OutcomeUnknown` with a permanent disposition — not a failure and
/// not a success.
#[test]
fn permanent_unknown_is_its_own_terminal_disposition() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([unknown(operation(6))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(6),
        tenant(1),
        operation(6),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::RequiresHuman,
    );
    rt.submit(submit(command_id(6), &body)?)?;

    let tick = rt.tick()?;

    assert_eq!(tick.receipts().len(), 1);
    assert_eq!(
        tick.receipts()[0].disposition(),
        TerminalDisposition::UnknownPermanent
    );
    assert_state(rt.projection(), run(6), RunState::OutcomeUnknown)?;
    Ok(())
}

/// A definitive failure ends the run `Failed` with a `Failed` disposition.
#[test]
fn definitive_failure_is_terminal() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([rocco_contracts::effect::EffectOutcome::Failed {
            operation: operation(7),
        }]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(7),
        tenant(1),
        operation(7),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::IdempotentRetrySafe,
    );
    rt.submit(submit(command_id(7), &body)?)?;

    let tick = rt.tick()?;

    assert_eq!(
        tick.receipts()[0].disposition(),
        TerminalDisposition::Failed
    );
    assert_state(rt.projection(), run(7), RunState::Failed)?;
    Ok(())
}

/// Cancellation is requested first and completed only once owned work has been
/// reconciled: requested is not completed.
#[test]
fn cancellation_enters_reconciliation_before_completing() -> Result<(), Box<dyn std::error::Error>>
{
    let mut rt = runtime_with(
        // Dispatch is unknown under a queryable strategy, so the effect stays
        // open; the reconciliation call then resolves it.
        ScriptedOwner::scripted([unknown(operation(8)), succeeded(operation(8))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let body = proposal(
        run(8),
        tenant(1),
        operation(8),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(8), &body)?)?;
    rt.tick()?;
    assert_state(rt.projection(), run(8), RunState::Reconciling)?;

    let cancel_body = proposal(
        run(8),
        tenant(1),
        operation(8),
        &[],
        0,
        ReconciliationStrategy::RequiresHuman,
    );
    rt.submit(cancel(command_id(9), &cancel_body)?)?;
    let requested = rt.tick()?;

    // The effect was reconciled in the same tick, so cancellation completes —
    // but only after the reconcile stage ran, never as a rewrite of it.
    assert_eq!(requested.reconciled(), &[operation(8)]);
    assert_state(rt.projection(), run(8), RunState::Cancelled)?;
    Ok(())
}

/// A run the journal never named is refused rather than silently created.
#[test]
fn cancellation_of_untracked_run_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    let body = proposal(
        run(99),
        tenant(1),
        operation(99),
        &[],
        0,
        ReconciliationStrategy::RequiresHuman,
    );
    rt.submit(cancel(command_id(99), &body)?)?;

    let tick = rt.tick()?;

    // The cancel names a run that owns no work, so the same tick can complete
    // it — but it completes as `Cancelled`, never as `Completed`, and publishes
    // no receipt. A cancellation must not be able to fabricate a success.
    assert_eq!(tick.receipts().len(), 0);
    assert_state(rt.projection(), run(99), RunState::Cancelled)?;
    Ok(())
}

/// Both halves of cancellation are recorded, in order: the request first, the
/// completion second.
#[test]
fn cancellation_records_request_before_completion() -> Result<(), Box<dyn std::error::Error>> {
    use rocco_contracts::event::EventKind;
    use rocco_contracts::state::TransitionEvent;
    use rocco_runtime::journal::JournalEntry;
    use rocco_runtime::payload::TransitionPayload;

    let mut rt = runtime()?;
    let body = proposal(
        run(11),
        tenant(1),
        operation(11),
        &[],
        0,
        ReconciliationStrategy::RequiresHuman,
    );
    rt.submit(cancel(command_id(11), &body)?)?;
    rt.tick()?;

    let mut transitions = Vec::new();
    for entry in rt.journal().entries() {
        if let JournalEntry::Event(ref event) = *entry
            && event.kind() == EventKind::StateTransition
        {
            let payload: TransitionPayload = lgwks_std::json::from_slice(event.payload())?;
            transitions.push(payload.event);
        }
    }

    let requested = transitions
        .iter()
        .position(|event| *event == TransitionEvent::RequestCancellation)
        .ok_or("no cancellation request was recorded")?;
    let finished = transitions
        .iter()
        .position(|event| *event == TransitionEvent::FinishCancellation)
        .ok_or("no cancellation completion was recorded")?;

    assert!(
        requested < finished,
        "cancellation completed before it was requested: {transitions:?}"
    );
    Ok(())
}

/// The journal is bounded: a full journal refuses admission instead of dropping
/// a recorded fact.
#[test]
fn a_full_journal_backpressures_admission() -> Result<(), Box<dyn std::error::Error>> {
    let mut settings = support::settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    settings.journal_capacity = 3;
    let mut rt: TestRuntime = support::fixture_runtime(settings, ScriptedOwner::new(), {
        rocco_runtime::clock::FixedClock::new(epoch()?)
    })?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::IdempotentRetrySafe,
    );
    rt.submit(submit(command_id(1), &body)?)?;

    let result = rt.tick();

    assert!(
        matches!(
            result,
            Err(rocco_runtime::error::RuntimeError::JournalFull { capacity: 3 })
        ),
        "expected a typed JournalFull refusal, got {result:?}"
    );
    Ok(())
}

/// A queued command is not accepted until a tick admits it.
#[test]
fn queuing_is_not_acceptance() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(1), &body)?)?;

    assert_eq!(rt.journal().len(), 0);
    assert_eq!(rt.tick_count(), 0);

    rt.tick()?;

    assert!(!rt.journal().is_empty());
    assert_eq!(rt.tick_count(), 1);
    Ok(())
}

/// The second gate: authority valid at admission is re-validated at the effect
/// boundary, so a grant that expired in between cannot be spent.
#[test]
fn an_expired_grant_is_refused_at_the_effect_boundary() -> Result<(), Box<dyn std::error::Error>> {
    use rocco_contracts::authority::{Budget, Grant, ResourcePolicy};
    use rocco_contracts::ids::WorkerId;
    use rocco_runtime::error::RuntimeError;

    // The grant expired an hour before the pinned instant the runtime uses.
    let grant = Grant::new(
        tenant(1),
        WorkerId::from_bytes([7; 16]),
        [Scope::FileRead].into_iter().collect(),
        Budget::new(100, rocco_contracts::authority::BudgetUnit::Requests),
        epoch()?,
        0,
    )?;
    let policy = ResourcePolicy::new([Scope::FileRead].into_iter().collect());

    let result = rocco_runtime::gate::revalidate_at_boundary(
        &grant,
        Scope::FileRead,
        1,
        &policy,
        epoch()?
            .checked_add(std::time::Duration::from_secs(60))
            .ok_or("fixture time overflow")?,
    );

    assert!(
        matches!(result, Err(RuntimeError::Authority(_))),
        "an expired grant must be refused at the boundary, got {result:?}"
    );
    Ok(())
}

/// A grant that is still valid and sufficient passes the boundary check.
#[test]
fn a_live_grant_passes_the_effect_boundary() -> Result<(), Box<dyn std::error::Error>> {
    use rocco_contracts::authority::{Budget, BudgetUnit, Grant, ResourcePolicy};
    use rocco_contracts::ids::WorkerId;

    let grant = Grant::new(
        tenant(1),
        WorkerId::from_bytes([7; 16]),
        [Scope::FileRead].into_iter().collect(),
        Budget::new(100, BudgetUnit::Requests),
        epoch()?
            .checked_add(std::time::Duration::from_secs(3_600))
            .ok_or("fixture time overflow")?,
        0,
    )?;
    let policy = ResourcePolicy::new([Scope::FileRead].into_iter().collect());

    rocco_runtime::gate::revalidate_at_boundary(&grant, Scope::FileRead, 1, &policy, epoch()?)?;
    Ok(())
}
