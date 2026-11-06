//! Falsifiers for the invariant the whole runtime rests on: the journal is
//! authoritative, and replaying it reproduces the same state and the same
//! authorization decisions, bit for bit (D04, R71, D36, E55).
//!
//! These are the tests that would fail if someone reintroduced a `HashMap`
//! iteration, a wall-clock read inside a decision, or a projection that
//! quietly became the source of truth.

mod support;

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::{Event, EventKind};
use larry_contracts::ids::{ContentDigest, OperationId, RunId};
use larry_contracts::replay::replay_log;
use larry_contracts::state::{RunState, TransitionEvent};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::{Journal, JournalEntry};
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::TransitionPayload;
use larry_runtime::projection::{
    EffectPhase, FoldProjection, ProjectionBackend, replay_authorizations,
};
use larry_runtime::runtime::{Runtime, restore};

use support::{
    TestRuntime, assert_state, command_id, epoch, operation, proposal, run, runtime_with, settings,
    submit, succeeded, tenant, unknown,
};

/// A stable identity for one journal entry: its run, sequence, kind tag and
/// payload digest. Two journals with equal identity lists are the same
/// history, without needing the entries to serialize identically.
fn fingerprint(journal: &Journal) -> Vec<(RunId, u64, EventKind, ContentDigest)> {
    journal
        .entries()
        .iter()
        .filter_map(|entry| match *entry {
            JournalEntry::Event(ref event) => Some((
                event.run(),
                event.sequence().value(),
                event.kind(),
                event.payload_digest(),
            )),
            _ => None,
        })
        .collect()
}

/// A runtime over `owner` that permits and is capable of `FileRead` with a budget of 100.
fn file_read_runtime(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    runtime_with(owner, &[Scope::FileRead], &[Scope::FileRead], 100)
}

/// The same command sequence, driven twice, produces the same history.
#[test]
fn identical_inputs_produce_identical_journals() -> Result<(), Box<dyn std::error::Error>> {
    fn drive() -> Result<Journal, Box<dyn std::error::Error>> {
        let mut rt = file_read_runtime(ScriptedOwner::scripted([succeeded(operation(1))]))?;
        let body = proposal(
            run(1),
            tenant(1),
            operation(1),
            &[Scope::FileRead],
            5,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        rt.submit(submit(command_id(1), &body)?)?;
        rt.tick()?;
        rt.tick()?;
        Ok(rt.into_journal())
    }

    let first = drive()?;
    let second = drive()?;

    let first_fingerprint = fingerprint(&first);
    let second_fingerprint = fingerprint(&second);
    assert!(
        !first_fingerprint.is_empty(),
        "the driven scenario recorded no events"
    );
    assert_eq!(first_fingerprint, second_fingerprint);
    Ok(())
}

/// Future strategy names must not turn into authorized dispatch or success.
#[test]
fn an_unknown_wire_strategy_is_refused_before_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    use larry_contracts::command::{Command, CommandContent, CommandKind};

    let mut runtime = file_read_runtime(ScriptedOwner::scripted([succeeded(operation(1))]))?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut wire = lgwks_std::json::to_value(&body)?;
    let strategy = wire
        .get_mut("strategy")
        .ok_or("serialized proposal has no strategy")?;
    *strategy = "UnsupportedFutureStrategy".into();
    let command = Command::new(
        command_id(1),
        tenant(1),
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            lgwks_std::json::to_vec(&wire)?.into_boxed_slice(),
        ),
    )?;
    runtime.submit(command)?;
    let result = runtime.tick();
    assert!(
        matches!(result, Err(RuntimeError::Codec(_))),
        "unknown strategy must be a typed codec refusal: {result:?}"
    );
    assert_eq!(runtime.owner().dispatched(), &[]);
    assert_eq!(runtime.projection().receipts(), &[]);
    assert_eq!(runtime.projection().authorizations(), &[]);
    Ok(())
}

/// Rebuilding a projection from the journal alone reproduces the live
/// projection exactly. This is the claim that makes recovery a re-fold.
#[test]
fn replay_reproduces_the_projection_exactly() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1)), succeeded(operation(2))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    for (n, op) in [(1u8, 1u8), (2, 2)] {
        let body = proposal(
            run(n),
            tenant(1),
            operation(op),
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        rt.submit(submit(command_id(n), &body)?)?;
    }
    rt.tick()?;

    let mut rebuilt = FoldProjection::default();
    larry_runtime::runtime::replay(&mut rebuilt, rt.journal())?;

    assert_eq!(rebuilt.run_count(), rt.projection().run_count());
    for n in [1u8, 2] {
        assert_eq!(
            rebuilt.run_state(run(n)),
            rt.projection().run_state(run(n)),
            "run {n} state differed after a re-fold"
        );
    }
    assert_eq!(rebuilt.receipts(), rt.projection().receipts());
    assert_eq!(rebuilt.authorizations().len(), 2);
    assert_state(&rebuilt, run(1), RunState::Completed)?;
    assert_state(&rebuilt, run(2), RunState::Completed)?;
    Ok(())
}

/// Every recorded authorization re-derives bit-identically from its own
/// pinned inputs (E55) — the direct answer to "does this journal replay?".
#[test]
fn recorded_authorizations_replay_bit_identically() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1)), succeeded(operation(2))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    for (n, cost) in [(1u8, 3u64), (2, 7)] {
        let body = proposal(
            run(n),
            tenant(1),
            operation(n),
            &[Scope::FileRead],
            cost,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        rt.submit(submit(command_id(n), &body)?)?;
    }
    rt.tick()?;

    let report = replay_authorizations(rt.projection());

    assert_eq!(report.findings().len(), 2);
    assert_eq!(report.divergences(), 0);
    assert!(report.is_bit_identical());
    Ok(())
}

/// The budget snapshot each decision was made under is the one that was in
/// force at the time, not the current one. Without that, the second
/// authorization would re-derive against a depleted budget and diverge.
#[test]
fn replay_holds_each_decision_against_its_own_budget_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1)), succeeded(operation(2))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    for n in [1u8, 2] {
        let body = proposal(
            run(n),
            tenant(1),
            operation(n),
            &[Scope::FileRead],
            40,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        rt.submit(submit(command_id(n), &body)?)?;
    }
    rt.tick()?;

    let records = rt.projection().authorizations();
    assert_eq!(records.len(), 2);
    // The first saw the full budget; the second saw 60 left. Neither sees the
    // final 20, which is what a naive "replay against current state" would do.
    assert_eq!(records[0].pinned.budget.remaining, 100);
    assert_eq!(records[1].pinned.budget.remaining, 60);
    assert_eq!(rt.remaining_budget(), 20);

    // Replaying against a *decision-changing* mutation must diverge
    // explicitly rather than pass silently. Draining the recorded budget below
    // the declared cost flips the decision from Authorized to BudgetExhausted.
    let mut mutated = records.to_vec();
    let mut first = mutated[0].clone();
    first.pinned.budget.remaining = 1;
    mutated[0] = first;
    let report = replay_log(&mutated);
    assert_eq!(report.divergences(), 1);
    assert!(!report.is_bit_identical());
    assert!(matches!(
        &report.findings()[0],
        larry_contracts::replay::ReplayFinding::Diverged {
            recorded: larry_contracts::replay::Decision::Authorized,
            rederived: larry_contracts::replay::Decision::Denied { .. },
            ..
        }
    ));
    Ok(())
}

/// A crash between the durable reservation and the recorded outcome leaves the
/// effect reading as reserved — "may have happened" — and restoration is
/// allowed to reconcile it, never to assume it never ran.
///
/// The crash is produced through the public API rather than injected: a journal
/// sized so the intent fits and the outcome does not is exactly the state a
/// process death in that window leaves behind.
#[test]
fn a_reserved_effect_survives_and_is_reconciled_not_reassumed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut limited = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    // Command, CommandAccepted, Admit, Start, EnterReconciling, Intent = 6
    // entries. The outcome event is the seventh and does not fit.
    limited.journal_capacity = 6;
    let mut rt: TestRuntime = Runtime::new(
        limited,
        ScriptedOwner::new(),
        larry_runtime::clock::FixedClock::new(epoch()?),
    )?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(1), &body)?)?;

    let crashed = rt.tick();
    assert!(
        matches!(crashed, Err(RuntimeError::JournalFull { capacity: 6 })),
        "expected the outcome write to be refused, got {crashed:?}"
    );

    // The journal the crash left behind ends at the reservation.
    let journal = rt.into_journal();
    let mut probe = FoldProjection::default();
    probe.rebuild(&journal)?;
    assert_eq!(
        probe.effect_phase(operation(1)),
        Some(EffectPhase::Reserved),
        "the crashed journal must leave the effect reserved, not resolved"
    );

    // Restoration treats the reservation as possibly-dispatched and reconciles.
    let mut restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::scripted([succeeded(operation(1))]),
        larry_runtime::clock::FixedClock::new(epoch()?),
        journal,
    )?;

    let tick = restored.tick()?;

    assert_eq!(tick.reconciled(), &[operation(1)]);
    assert_eq!(tick.receipts().len(), 1);
    assert_state(restored.projection(), run(1), RunState::Completed)?;
    assert!(
        restored.owner().dispatched().is_empty(),
        "restoration must not re-dispatch"
    );
    Ok(())
}

/// Folding a journal whose recorded transition disagrees with the pure algebra
/// is refused, not accepted. The journal is self-checking.
#[test]
fn a_divergent_transition_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let mut journal = Journal::new(64)?;
    let payload = TransitionPayload::new(
        TransitionEvent::Admit,
        RunState::Draft,
        // The algebra says Draft + Admit is Admitted; this claims otherwise.
        RunState::Completed,
    );
    let bytes = lgwks_std::json::to_vec(&payload)?;
    let event = Event::new(
        tenant(1),
        run(1),
        larry_contracts::event::EventSequence::FIRST,
        Vec::new(),
        EventKind::StateTransition,
        bytes.into_boxed_slice(),
    )?;
    journal.record(event)?;

    let mut projection = FoldProjection::default();
    let result = projection.rebuild(&journal);

    assert!(
        matches!(
            result,
            Err(RuntimeError::FoldDivergence {
                recorded: RunState::Completed,
                rederived: RunState::Admitted,
                ..
            })
        ),
        "expected a typed FoldDivergence, got {result:?}"
    );
    Ok(())
}

/// The projection is a view: discarding and rebuilding it loses nothing.
#[test]
fn the_projection_is_discardable() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = file_read_runtime(ScriptedOwner::scripted([
        unknown(operation(1)),
        succeeded(operation(1)),
    ]))?;
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

    let before = rt.journal().len();
    let state_before = rt.projection().run_state(run(1));

    // A fresh view over the same journal agrees with the live one.
    let mut fresh = FoldProjection::default();
    larry_runtime::runtime::replay(&mut fresh, rt.journal())?;

    assert_eq!(fresh.run_state(run(1)), state_before);
    assert_eq!(rt.journal().len(), before);
    assert_eq!(
        fresh.effect_phase(operation(1)),
        rt.projection().effect_phase(operation(1))
    );
    Ok(())
}

/// A helper kept honest: the crate's own `EffectOutcome` constructors agree
/// with the intent they describe.
#[test]
fn outcome_carries_the_operation_it_describes() {
    let intent = EffectIntent::new(
        larry_contracts::ids::EffectId::from_bytes(*operation(3).as_bytes()),
        operation(3),
        run(3),
        tenant(1),
        ReconciliationStrategy::Queryable,
        SecurityEpoch::INITIAL,
    );
    let outcome: EffectOutcome = succeeded(intent.operation);
    assert_eq!(outcome.operation(), OperationId::from_bytes([3; 16]));
}

/// The journal's order is total and checked, and that is what makes the fold's
/// order well-defined.
///
/// This is not a formality. A fold over an unordered collection would make the
/// projection depend on iteration order, and the ECS this runtime keeps a seam
/// for states outright that its query iteration order is not guaranteed —
/// with parallel iteration the order is not merely unspecified but genuinely
/// raced. Refusing an out-of-order append is what keeps the fold a pure
/// function of a totally ordered log rather than of a scheduler's whims.
#[test]
fn the_journal_refuses_an_out_of_order_append() -> Result<(), Box<dyn std::error::Error>> {
    use larry_contracts::error::SequenceError;
    use larry_contracts::event::EventSequence;

    let mut journal = Journal::new(64)?;
    let first = Event::new(
        tenant(1),
        run(1),
        EventSequence::FIRST,
        Vec::new(),
        EventKind::StateTransition,
        lgwks_std::json::to_vec(&TransitionPayload::new(
            TransitionEvent::Admit,
            RunState::Draft,
            RunState::Admitted,
        ))?
        .into_boxed_slice(),
    )?;
    journal.record(first)?;

    // Sequence 2 skips the gap-free successor, 1.
    let skipping = Event::new(
        tenant(1),
        run(1),
        EventSequence::new(2),
        Vec::new(),
        EventKind::StateTransition,
        lgwks_std::json::to_vec(&TransitionPayload::new(
            TransitionEvent::Start,
            RunState::Admitted,
            RunState::Running,
        ))?
        .into_boxed_slice(),
    )?;

    let result = journal.record(skipping);

    assert!(
        matches!(
            result,
            Err(RuntimeError::Sequence(SequenceError::Gap {
                expected: 1,
                got: 2
            }))
        ),
        "a gap in the sequence must be refused, got {result:?}"
    );
    assert_eq!(journal.len(), 1);
    Ok(())
}

/// Every journal the runtime can produce is restorable, because recovery is a
/// re-fold and nothing else (D04). A run whose recorded history reserved no
/// effect — a submission the reducer denied, a steer, or a cancel that arrived
/// before any work — is still a run the journal names, and restore must not
/// confuse "the journal never named this run" with "this run reserved
/// nothing".
#[test]
fn a_run_that_reserved_no_effect_does_not_block_restore() -> Result<(), Box<dyn std::error::Error>>
{
    // The policy permits nothing, so the reducer denies and the run is
    // tracked by its own transitions with no intent at all.
    let mut rt = runtime_with(ScriptedOwner::new(), &[], &[Scope::FileRead], 100)?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(1), &body)?)?;
    let tick = rt.tick()?;
    assert_eq!(tick.denied().len(), 1);
    assert_eq!(tick.authorized(), &[], "no effect was reserved");

    let journal = rt.into_journal();
    let mut probe = FoldProjection::default();
    probe.rebuild(&journal)?;
    assert_eq!(probe.run_ids(), vec![run(1)], "the journal names the run");
    assert!(probe.intents().is_empty(), "and reserved no effect for it");

    let restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        larry_runtime::clock::FixedClock::new(epoch()?),
        journal,
    )?;

    assert_eq!(
        restored.projection().run_ids(),
        vec![run(1)],
        "the restored runtime tracks every run the journal named"
    );
    Ok(())
}
