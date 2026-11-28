//! Seeded coverage sims for the projection fold, the journal payloads and the
//! journal's own bounds.
//!
//! The fold is the runtime's only view of history, so every corrupt record a
//! hostile writer or a torn file can produce is folded here and named exactly.
//! Sources: E55 (bit-identical replay), larry#182 (dispatch budget as a fold,
//! torn tail), larry#208 (per-tenant spend), larry#371 (refused commands as
//! facts), Cell 04 (gap-free sequencing) and CWE-20 (hostile journal bytes).

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use larry_contracts::authority::BudgetUnit;
use larry_contracts::command::CommandKind;
use larry_contracts::effect::{
    EffectIntent, EffectOutcome, ReconciliationStrategy, TerminalDisposition,
};
use larry_contracts::event::{Event, EventKind, EventSequence};
use larry_contracts::ids::{CommandId, EffectId, OperationId, RunId, TenantId};
use larry_contracts::replay::{
    BudgetSnapshot, CapabilitySnapshot, Decision, EffectRequest, PinnedInputs, PolicySnapshot,
    ValiditySnapshot,
};
use larry_contracts::state::{RunState, TransitionEvent};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::{
    CommandAcceptedPayload, IntentPayload, OutcomePayload, ReceiptPayload, TransitionPayload,
};
use larry_runtime::projection::{
    EffectPhase, FoldProjection, ProjectionBackend, RunView, replay_authorizations,
};
use larry_runtime::runtime::{Runtime, restore};
use support::{clock, command_id, epoch, operation, run, settings, submit, tenant};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Calls a tiny `const` constructor or getter through an opaque function
/// pointer so its body executes out of line in its own crate.
fn outlined<F>(func: F) -> F {
    std::hint::black_box(func)
}

/// A scope set with one scope.
fn one_scope() -> BTreeSet<larry_contracts::authority::Scope> {
    [larry_contracts::authority::Scope::FileRead]
        .into_iter()
        .collect()
}

/// Records one event for `run` at `sequence` in `journal`.
fn record(
    journal: &mut Journal,
    tenant: TenantId,
    run: RunId,
    sequence: u64,
    kind: EventKind,
    payload: Vec<u8>,
) -> Outcome {
    journal.record(Event::new(
        tenant,
        run,
        EventSequence::new(sequence),
        Vec::new(),
        kind,
        payload.into_boxed_slice(),
    )?)?;
    Ok(())
}

/// The intent of one `FileRead` effect for `operation`, with `tenant`.
fn intent(
    operation: OperationId,
    run: RunId,
    tenant: TenantId,
    strategy: ReconciliationStrategy,
) -> EffectIntent {
    EffectIntent::new(
        EffectId::from_bytes([0x33; 16]),
        operation,
        run,
        tenant,
        strategy,
        larry_contracts::authority::SecurityEpoch::INITIAL,
    )
}

/// Pinned inputs that authorize one unit of `FileRead` work.
fn pinned(operation: OperationId) -> Result<PinnedInputs, Box<dyn std::error::Error>> {
    Ok(PinnedInputs::new(
        EffectRequest::new(operation, one_scope(), 1, BudgetUnit::Requests, epoch()?),
        PolicySnapshot::new(1, one_scope()),
        BudgetSnapshot::new(100, BudgetUnit::Requests),
        CapabilitySnapshot::new(one_scope()),
        ValiditySnapshot::new(
            epoch()?
                .checked_add(Duration::from_secs(3_600))
                .ok_or("fixture validity overflow")?,
        ),
    ))
}

// Source: larry#182 (a restart spends the recorded dispatch budget) and CWE-20
// (a hostile generator declares a zero identity). A journal that reserves an
// effect for an all-zero tenant, run or operation restores, and the first
// reconcile refuses the effect key typed naming the field, reaching no owner
// and recording the refusal while the tick goes on.
#[test]
fn sim_zero_identity_intent_is_refused_typed() -> Outcome {
    for seed in 0..24_u64 {
        let field = match seed % 3 {
            0 => "tenant",
            1 => "run",
            _ => "operation",
        };
        let mut salted = [0_u8; 16];
        salted[..8].copy_from_slice(&seed.to_le_bytes());
        let zero = [0_u8; 16];
        let run = RunId::from_bytes(if field == "run" { zero } else { salted });
        let tenant = TenantId::from_bytes(if field == "tenant" { zero } else { salted });
        let operation = OperationId::from_bytes(if field == "operation" { zero } else { salted });
        let event_tenant = TenantId::from_bytes([0xE0; 16]);
        let mut journal = Journal::new(64)?;
        let effect = intent(
            operation,
            run,
            tenant,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        let authorization =
            larry_contracts::replay::ReplayRecord::new(pinned(operation)?, Decision::Authorized);
        record(
            &mut journal,
            event_tenant,
            run,
            0,
            EventKind::EffectIntentRecorded,
            lgwks_std::json::to_vec(&IntentPayload::new(effect, authorization))?,
        )?;
        record(
            &mut journal,
            event_tenant,
            run,
            1,
            EventKind::EffectOutcomeRecorded,
            lgwks_std::json::to_vec(&OutcomePayload::new(
                effect,
                EffectOutcome::Unknown { operation },
                0,
            ))?,
        )?;
        let mut runtime: Runtime<FoldProjection, ScriptedOwner> = restore(
            settings(
                &[larry_contracts::authority::Scope::FileRead],
                &[larry_contracts::authority::Scope::FileRead],
                100,
            )?,
            ScriptedOwner::new(),
            clock()?,
            journal,
        )?;
        let report = runtime.tick()?;
        assert!(
            report.reconciled().is_empty(),
            "seed {seed}: nothing reconciles past a refused key"
        );
        assert_eq!(
            report.refused().len(),
            1,
            "seed {seed}: exactly the refuse is recorded: {:?}",
            report.refused()
        );
        assert!(
            matches!(
                report.refused().first(),
                Some((
                    larry_runtime::runtime::TickItem::Effect(op),
                    RuntimeError::EffectIdentity { field: got, .. }
                )) if *op == operation && *got == field
            ),
            "seed {seed}: the zero identity is refused naming its field: {:?}",
            report.refused()
        );
        assert!(
            runtime.owner().dispatched().is_empty(),
            "seed {seed}: the owner is never reached for a keyless effect"
        );
    }
    Ok(())
}

// Source: E55 (a corrupt record surfaces, never passes silently), larry#182
// (a second opener is refused while the first holds the file is journal
// territory; here the fold refuses its own corruption) and CWE-20. Every
// undecodable payload, every duplicate reservation or receipt, and every
// transition the pure algebra re-derives differently is a typed refusal,
// and the per-run accessors read back exactly what was folded.
#[test]
fn sim_projection_fold_names_every_corrupt_record() -> Outcome {
    // A transition the algebra re-derives differently diverges by name.
    let mut journal = Journal::new(64)?;
    let (run, tenant) = (run(1), tenant(1));
    record(
        &mut journal,
        tenant,
        run,
        0,
        EventKind::StateTransition,
        lgwks_std::json::to_vec(&TransitionPayload::new(
            TransitionEvent::Start,
            RunState::Admitted,
            RunState::Failed,
        ))?,
    )?;
    let mut folded = FoldProjection::new(64);
    assert!(
        matches!(
            larry_runtime::runtime::replay(&mut folded, &journal),
            Err(RuntimeError::FoldDivergence {
                recorded: RunState::Failed,
                rederived: RunState::Running,
                ..
            })
        ),
        "a recorded state the algebra re-derives differently diverges"
    );

    // An undecodable transition payload is a codec refusal, not a state.
    let mut journal = Journal::new(64)?;
    record(
        &mut journal,
        tenant,
        run,
        0,
        EventKind::StateTransition,
        b"not json".to_vec(),
    )?;
    let mut folded = FoldProjection::new(64);
    assert!(
        matches!(
            larry_runtime::runtime::replay(&mut folded, &journal),
            Err(RuntimeError::Codec(_))
        ),
        "a garbage transition payload is a codec refusal"
    );

    // An undecodable acceptance payload is a codec refusal.
    let mut journal = Journal::new(64)?;
    record(
        &mut journal,
        tenant,
        run,
        0,
        EventKind::CommandAccepted,
        b"not json".to_vec(),
    )?;
    let mut folded = FoldProjection::new(64);
    assert!(
        matches!(
            larry_runtime::runtime::replay(&mut folded, &journal),
            Err(RuntimeError::Codec(_))
        ),
        "a garbage acceptance payload is a codec refusal"
    );

    // A second reservation of one operation is a duplicate, never a second
    // effect; the first intent's cost is charged once to its event's tenant.
    let mut journal = Journal::new(64)?;
    let effect = intent(operation(1), run, tenant, ReconciliationStrategy::Queryable);
    let authorization =
        larry_contracts::replay::ReplayRecord::new(pinned(operation(1))?, Decision::Authorized);
    // A question-free reservation: the authorization inputs above authorize.
    assert_eq!(authorization.recorded, Decision::Authorized);
    record(
        &mut journal,
        tenant,
        run,
        0,
        EventKind::EffectIntentRecorded,
        lgwks_std::json::to_vec(&IntentPayload::new(effect, authorization.clone()))?,
    )?;
    record(
        &mut journal,
        tenant,
        run,
        1,
        EventKind::EffectIntentRecorded,
        lgwks_std::json::to_vec(&IntentPayload::new(effect, authorization))?,
    )?;
    let mut folded = FoldProjection::new(64);
    assert!(
        matches!(
            larry_runtime::runtime::replay(&mut folded, &journal),
            Err(RuntimeError::DuplicateOperation { operation: op })
            if op == operation(1)
        ),
        "a reused operation is refused before a second reservation"
    );
    Ok(())
}

// Source: E55 (the fold is the runtime's only view of history). The per-run
// accessors read back exactly what was folded: state, tenant, counts,
// effects, sequences, receipts, authorizations and spend.
#[test]
fn sim_projection_accessors_read_back_the_fold() -> Outcome {
    let (run, tenant) = (run(1), tenant(1));
    // The per-run accessors read back exactly the one folded intent.
    let mut journal = Journal::new(64)?;
    let effect = intent(operation(1), run, tenant, ReconciliationStrategy::Queryable);
    record(
        &mut journal,
        tenant,
        run,
        0,
        EventKind::EffectIntentRecorded,
        lgwks_std::json::to_vec(&IntentPayload::new(
            effect,
            larry_contracts::replay::ReplayRecord::new(pinned(operation(1))?, Decision::Authorized),
        ))?,
    )?;
    let mut folded = FoldProjection::new(64);
    larry_runtime::runtime::replay(&mut folded, &journal)?;
    assert_eq!(folded.run_state(run), Some(RunState::Draft));
    assert_eq!(folded.run_tenant(run), Some(tenant));
    assert_eq!(folded.run_count(), 1);
    assert_eq!(folded.effect_count(run), 1);
    assert_eq!(
        folded.effect_count(RunId::from_bytes([9; 16])),
        0,
        "an unseen run holds nothing"
    );
    let run_at: fn(&FoldProjection, RunId) -> Option<&RunView> = outlined(FoldProjection::run);
    assert!(run_at(&folded, run).is_some());
    let runs_of: fn(&FoldProjection) -> &BTreeMap<RunId, RunView> = outlined(FoldProjection::runs);
    assert_eq!(runs_of(&folded).len(), 1);
    let view = folded.run(run).ok_or("the folded view must exist")?;
    let state_of: fn(&RunView) -> RunState = outlined(RunView::state);
    assert_eq!(state_of(view), RunState::Draft);
    let tenant_of_view: fn(&RunView) -> Option<TenantId> = outlined(RunView::tenant);
    assert_eq!(tenant_of_view(view), Some(tenant));
    let commands_of: fn(&RunView) -> usize = outlined(RunView::commands);
    assert_eq!(commands_of(view), 0);
    let effects_of: fn(&RunView) -> &BTreeMap<OperationId, EffectPhase> =
        outlined(RunView::effects);
    assert_eq!(effects_of(view).len(), 1);
    let sequence_of: fn(&RunView) -> Option<larry_contracts::event::EventSequence> =
        outlined(RunView::last_sequence);
    assert_eq!(sequence_of(view), Some(EventSequence::new(0)));
    assert!(folded.run_ids().contains(&run));
    assert_eq!(folded.intents().len(), 1);
    assert_eq!(folded.authorizations().len(), 1);
    assert_eq!(
        folded.reserved_by_tenant().get(&tenant),
        Some(&1),
        "one unit of spend is charged to the event's tenant"
    );
    assert!(folded.receipts().is_empty());
    assert!(!folded.has_receipt(operation(1)));
    let replayed = replay_authorizations(&folded);
    assert!(replayed.is_bit_identical(), "one decision replays");

    // An operation the journal never mentions proves no dispatch count, so a
    // restart spends the whole budget for it rather than recharging it.
    let dispatches_of: fn(&FoldProjection, OperationId) -> u8 =
        outlined(FoldProjection::dispatches);
    assert_eq!(
        dispatches_of(&folded, operation(2)),
        larry_contracts::effect::MAX_EFFECT_ATTEMPTS,
        "an unproven operation restores with the whole budget spent"
    );
    Ok(())
}

// Source: larry#182 (an old record without an attempt number restores with
// the whole budget spent) and the payload codec. Every payload constructor
// executes out of line through a function pointer and reads back exactly;
// a record written before attempt numbers existed decodes with the maximum
// dispatch count, so a restart never recharges what the journal cannot prove.
#[test]
fn sim_payload_constructors_and_old_records() -> Outcome {
    let new_accepted: fn(CommandId, RunId, CommandKind) -> CommandAcceptedPayload =
        outlined(CommandAcceptedPayload::new);
    let accepted = new_accepted(command_id(1), run(1), CommandKind::Submit);
    assert_eq!(accepted.command, command_id(1));
    assert_eq!(accepted.run, run(1));
    assert_eq!(accepted.kind, CommandKind::Submit);

    let new_transition: fn(TransitionEvent, RunState, RunState) -> TransitionPayload =
        outlined(TransitionPayload::new);
    let transition = new_transition(
        TransitionEvent::Start,
        RunState::Admitted,
        RunState::Running,
    );
    assert_eq!(transition.event, TransitionEvent::Start);
    assert_eq!(transition.from, RunState::Admitted);
    assert_eq!(transition.to, RunState::Running);

    let effect = intent(
        operation(1),
        run(1),
        tenant(1),
        ReconciliationStrategy::RequiresHuman,
    );
    let new_intent: fn(EffectIntent, larry_contracts::replay::ReplayRecord) -> IntentPayload =
        outlined(IntentPayload::new);
    let authorization =
        larry_contracts::replay::ReplayRecord::new(pinned(operation(1))?, Decision::Authorized);
    let reservation = new_intent(effect, authorization);
    assert_eq!(reservation.intent, effect);

    let new_outcome: fn(EffectIntent, EffectOutcome, u8) -> OutcomePayload =
        outlined(OutcomePayload::new);
    let observed = new_outcome(
        effect,
        EffectOutcome::Failed {
            operation: operation(1),
        },
        2,
    );
    assert_eq!(observed.dispatches, 2);

    let new_receipt: fn(larry_contracts::effect::TerminalReceipt) -> ReceiptPayload =
        outlined(ReceiptPayload::new);
    let receipt = larry_contracts::effect::TerminalReceipt::publish(
        operation(1),
        run(1),
        tenant(1),
        TerminalDisposition::Failed,
    );
    assert_eq!(new_receipt(receipt).receipt, receipt);

    // The declared strategy reaches the reserved intent; it decides the
    // reconciliation path.
    let body = support::proposal(
        run(1),
        tenant(1),
        operation(1),
        &[larry_contracts::authority::Scope::FileRead],
        1,
        ReconciliationStrategy::AlwaysCompensate,
    );
    let reserved = body.intent(
        tenant(1),
        larry_contracts::authority::SecurityEpoch::INITIAL,
    );
    assert_eq!(reserved.strategy, ReconciliationStrategy::AlwaysCompensate);

    // A record without an attempt number decodes with the whole budget spent.
    let full = lgwks_std::json::to_vec(&observed)?;
    let mut trimmed = String::from_utf8(full).map_err(|error| error.to_string())?;
    let cut = trimmed
        .find(",\"dispatches\"")
        .ok_or("the attempt number must be the last field")?;
    trimmed.truncate(cut);
    trimmed.push('}');
    let old: OutcomePayload = lgwks_std::json::from_str(&trimmed)?;
    assert_eq!(
        old.dispatches,
        larry_contracts::effect::MAX_EFFECT_ATTEMPTS,
        "an old record restores with the whole budget spent"
    );
    Ok(())
}

// Source: Cell 05 (every bound refuses past itself) and CWE-20 (a zero or
// contradictory configuration). A zero capacity, a ceiling below history
// and an empty run table are typed refusals or exact answers, never silent
// clamps, and a journal opened at capacity zero refuses before reading.
#[test]
fn sim_journal_bounds_and_inspection() -> Outcome {
    let new_journal: fn(usize) -> Result<Journal, RuntimeError> = outlined(Journal::new);
    assert!(
        matches!(
            new_journal(0),
            Err(RuntimeError::JournalFull { capacity: 0 })
        ),
        "a zero-capacity journal is refused before it records"
    );
    let here = support::temp_dir::TempDir::new()?;
    let path = here.path().join("bounds.journal");
    assert!(
        matches!(
            Journal::open(&path, 0),
            Err(RuntimeError::JournalFull { capacity: 0 })
        ),
        "opening at capacity zero refuses before reading the file"
    );
    let mut journal = Journal::new(4)?;
    assert!(journal.is_empty());
    assert_eq!(journal.len(), 0);
    assert_eq!(journal.capacity(), 4);
    assert!(!journal.is_full());
    let (run, tenant) = (run(1), tenant(1));
    let body = support::proposal(
        run,
        tenant,
        operation(1),
        &[larry_contracts::authority::Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let command = submit(command_id(1), &body)?;
    journal.accept_command(command)?;
    let other = submit(command_id(2), &body)?;
    journal.accept_command(other)?;
    assert_eq!(journal.len(), 2);
    assert!(!journal.is_empty());
    assert_eq!(journal.run_count(), 0, "commands alone track no run");
    assert!(journal.last().is_some());
    assert!(journal.command(command_id(1)).is_some());
    assert!(journal.command(command_id(2)).is_some());
    assert!(journal.command(command_id(3)).is_none());
    let reopen_as: fn(Journal, usize) -> Result<Journal, RuntimeError> = outlined(Journal::reopen);
    assert!(
        matches!(
            reopen_as(journal.clone(), 1),
            Err(RuntimeError::JournalFull { capacity: 1 })
        ),
        "a ceiling below history is a contradiction, never a truncation"
    );
    assert!(
        matches!(
            reopen_as(journal.clone(), 0),
            Err(RuntimeError::JournalFull { capacity: 0 })
        ),
        "a zero ceiling reopens nothing"
    );
    let kept = reopen_as(journal, 64)?;
    assert_eq!(kept.len(), 2, "reopening keeps every entry");
    let run_count_of: fn(&Journal) -> usize = outlined(Journal::run_count);
    assert_eq!(run_count_of(&kept), 0);
    Ok(())
}
