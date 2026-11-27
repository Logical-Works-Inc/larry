//! Seeded coverage sims for admission, the owner reach, reconciliation and the
//! journal's fault paths.
//!
//! Every bound is filled to its edge and one past it: the global pending
//! set, the per-tenant share, the ready queue, the per-run effect count and
//! the journal itself. Faults a real store produces (a refused sync, a full
//! file) are injected through a read-only directory, and hostile identities
//! and payloads through hand-crafted journals. Sources: E28 (durable
//! admission), E71 (fenced reaches), larry#182 (sync ordering), larry#209
//! (latched journal), larry#212 (every bound refused by name), larry#333
//! (tenant shares), Cell 05 (bounded queues) and CWE-20, CWE-400
//! (unbounded allocation), CWE-665 (improper initialization).

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::{Event, EventKind, EventSequence};
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::ProjectionBackend;
use larry_runtime::protocol::{
    MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, ProtocolCapability, ProtocolOffer,
    ProtocolVersionRange,
};
use larry_runtime::runtime::{Question, Runtime, Subject, restore};
use support::{
    TestRuntime, assert_state, clock, command_id, epoch, operation, proposal, run, runtime,
    settings, submit, tenant,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// A submit-only offer for `tenant`.
fn submit_offer(tenant: TenantId) -> ProtocolOffer {
    ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
        [ProtocolCapability::SubmitCommand].into_iter().collect(),
        false,
        false,
    )
}

/// A `Steer` command carrying `body`.
fn steer(id: CommandId, body: &WorkProposal) -> Result<Command, Box<dyn std::error::Error>> {
    Ok(Command::new(
        id,
        body.tenant(),
        Vec::new(),
        CommandContent::new(
            CommandKind::Steer,
            lgwks_std::json::to_vec(body)?.into_boxed_slice(),
        ),
    )?)
}

// Source: larry#333 (a tenant at its share waits behind the others instead
// of holding the queue head) and D25. Corrections a run is not settled for
// hold the pending set; a tenant at its share waits behind the others, and
// every queued command is passed over once per pass.
#[test]
fn sim_ready_queue_waits_behind_a_full_share() -> Outcome {
    let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    capped.max_tenants = 2;
    let mut runtime: TestRuntime = Runtime::new(capped, ScriptedOwner::new(), clock()?)?;
    let tenant = tenant(1);
    let run = run(1);
    let no_scopes: &[Scope] = &[];
    // One open effect keeps the run unsettled, so every correction waits.
    let work = proposal(
        run,
        tenant,
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit_protocol(&submit_offer(tenant), submit(command_id(1), &work)?, None)?;
    let first = runtime.tick()?;
    assert_eq!(
        first.admitted(),
        &[command_id(1)],
        "the opener dispatches: {:?}",
        first.admitted()
    );
    assert!(
        first.denied().is_empty() && first.refused().is_empty(),
        "the opener authorizes cleanly: denied={:?} refused={:?}",
        first.denied(),
        first.refused()
    );
    assert_eq!(
        runtime.projection().effect_count(run),
        1,
        "one open effect keeps the run unsettled"
    );
    for index in 0..128_u32 {
        let mut op_bytes = [7_u8; 16];
        op_bytes[..4].copy_from_slice(&index.to_be_bytes());
        let mut id_bytes = [8_u8; 16];
        id_bytes[..4].copy_from_slice(&index.to_be_bytes());
        let idle = proposal(
            run,
            tenant,
            OperationId::from_bytes(op_bytes),
            no_scopes,
            0,
            ReconciliationStrategy::Queryable,
        );
        runtime.submit_protocol(
            &submit_offer(tenant),
            steer(CommandId::from_bytes(id_bytes), &idle)?,
            None,
        )?;
    }
    // Two more of the same tenant queue behind the full share.
    let idle = proposal(
        run,
        tenant,
        operation(8),
        no_scopes,
        0,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(8), &idle)?)?;
    let idle = proposal(
        run,
        tenant,
        operation(9),
        no_scopes,
        0,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(9), &idle)?)?;
    let tick = runtime.tick()?;
    assert!(
        tick.admitted().is_empty(),
        "nothing passes a full share: {:?}",
        tick.admitted()
    );
    Ok(())
}

// Source: Cell 05 (one run holds at most 64 effects) and CWE-400. Sixty-four
// effects fill the run; the 65th is refused naming the run and its capacity,
// and nothing is recorded for an effect that never exists.
#[test]
fn sim_run_at_effect_capacity_refuses_one_more() -> Outcome {
    use larry_contracts::replay::{
        BudgetSnapshot, CapabilitySnapshot, EffectRequest, PinnedInputs, PolicySnapshot,
        ValiditySnapshot,
    };
    use larry_runtime::payload::IntentPayload;
    let tenant = tenant(5);
    let run = run(5);
    // Sixty-four reservations for one run, recorded directly: sequential
    // submits cannot do this, because the second submit finds its run past
    // `Start` and is refused by the algebra before any reservation.
    let mut journal = Journal::new(128)?;
    for index in 0..64_u32 {
        let mut op_bytes = [5_u8; 16];
        op_bytes[..4].copy_from_slice(&index.to_be_bytes());
        let operation = OperationId::from_bytes(op_bytes);
        let pinned = PinnedInputs::new(
            EffectRequest::new(
                operation,
                [Scope::FileRead].into_iter().collect(),
                1,
                BudgetUnit::Requests,
                epoch()?,
            ),
            PolicySnapshot::new(1, [Scope::FileRead].into_iter().collect()),
            BudgetSnapshot::new(100, BudgetUnit::Requests),
            CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
            ValiditySnapshot::new(
                epoch()?
                    .checked_add(Duration::from_secs(3_600))
                    .ok_or("time")?,
            ),
        );
        let intent = larry_contracts::effect::EffectIntent::new(
            larry_contracts::ids::EffectId::from_bytes(op_bytes),
            operation,
            run,
            tenant,
            ReconciliationStrategy::Queryable,
            larry_contracts::authority::SecurityEpoch::INITIAL,
        );
        journal.record(Event::new(
            tenant,
            run,
            EventSequence::new(u64::from(index)),
            Vec::new(),
            EventKind::EffectIntentRecorded,
            lgwks_std::json::to_vec(&IntentPayload::new(
                intent,
                larry_contracts::replay::ReplayRecord::new(
                    pinned,
                    larry_contracts::replay::Decision::Authorized,
                ),
            ))?
            .into_boxed_slice(),
        )?)?;
    }
    let mut runtime: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        clock()?,
        journal,
    )?;
    assert_eq!(
        runtime.projection().effect_count(run),
        64,
        "sixty-four effects are reserved"
    );
    let extra = proposal(
        run,
        tenant,
        operation(65),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(65), &extra)?)?;
    let tick = runtime.tick()?;
    assert!(
        tick.refused().iter().any(|entry| matches!(
            entry.1,
            RuntimeError::EffectLimitReached { capacity: 64, .. }
        )),
        "the 65th effect is refused: {:?}",
        tick.refused()
    );
    assert_eq!(
        runtime.projection().effect_count(run),
        64,
        "nothing is recorded past the bound"
    );
    Ok(())
}

// Source: E28 (a correction waits for a safe boundary) and D25. A steer
// whose run is unsettled holds, every later command of that run waits behind
// it while the tick walks past the end, and other runs still pass.
#[test]
fn sim_correction_holds_later_commands_until_settled() -> Outcome {
    let mut runtime = runtime()?;
    let tenant = tenant(6);
    let run = run(6);
    let other = RunId::from_bytes([60; 16]);
    let work = proposal(
        run,
        tenant,
        operation(6),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(60), &work)?)?;
    runtime.tick()?;
    // The effect is open (the owner answers unknown), so the correction holds.
    let no_scopes: &[Scope] = &[];
    let steer_body = proposal(
        run,
        tenant,
        operation(61),
        no_scopes,
        0,
        ReconciliationStrategy::Queryable,
    );
    let later_body = proposal(
        run,
        tenant,
        operation(62),
        no_scopes,
        0,
        ReconciliationStrategy::Queryable,
    );
    let passing_body = proposal(
        other,
        tenant,
        operation(63),
        no_scopes,
        0,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(steer(command_id(61), &steer_body)?)?;
    runtime.submit(submit(command_id(62), &later_body)?)?;
    runtime.submit(steer(command_id(63), &passing_body)?)?;
    let tick = runtime.tick()?;
    assert!(
        tick.admitted().contains(&command_id(63)),
        "another run passes the held correction: {:?}",
        tick.admitted()
    );
    assert!(
        !tick.admitted().contains(&command_id(61)),
        "the correction is held: {:?}",
        tick.admitted()
    );
    assert!(
        !tick.admitted().contains(&command_id(62)),
        "later commands of the held run wait: {:?}",
        tick.admitted()
    );
    Ok(())
}

// Source: larry#329 (one tenant's refusal never skips another tenant's work)
// and D42 (runs belong to one tenant). Two tenants' commands for one run in
// one restored journal are refused for the second tenant at promote time,
// while the first tenant's command still reduces.
#[test]
fn sim_cross_tenant_pendings_refused_at_promote() -> Outcome {
    let run = run(7);
    let tenant_a = tenant(7);
    let tenant_b = tenant(70);
    let body_a = proposal(
        run,
        tenant_a,
        operation(7),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let body_b = proposal(
        run,
        tenant_b,
        operation(70),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut journal = Journal::new(64)?;
    let accepted_a = submit(command_id(70), &body_a)?;
    let accepted_b = submit(command_id(71), &body_b)?;
    journal.accept_command(accepted_a)?;
    journal.accept_command(accepted_b)?;
    let mut runtime: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        clock()?,
        journal,
    )?;
    let tick = runtime.tick()?;
    assert!(
        tick.refused()
            .iter()
            .any(|entry| matches!(entry.1, RuntimeError::RunTenantMismatch { .. })),
        "the foreign command is refused at promote: {:?}",
        tick.refused()
    );
    assert!(
        !tick.refused().iter().any(|entry| matches!(
            &entry.0,
            larry_runtime::runtime::TickItem::Command(id) if *id == command_id(70)
        )),
        "the first tenant still reduces: {:?}",
        tick.refused()
    );
    Ok(())
}

// Source: E30 (a pending question holds its submit) and larry#332 (the stop
// control stays real). An answer hold stops work but never the stop control:
// the cancellation passes the hold and closes the run in the same tick.
#[test]
fn sim_answer_hold_passes_stop_control_but_holds_work() -> Outcome {
    let tenant = tenant(8);
    let requester = WorkerId::from_bytes([0xA8; 16]);
    let respondent = WorkerId::from_bytes([0xB8; 16]);
    let run = run(8);
    let operation = operation(8);
    let body = proposal(
        run,
        tenant,
        operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let digest = submit(command_id(80), &body)?.content().digest();
    let mut runtime = runtime()?;
    let expiry = epoch()?
        .checked_add(Duration::from_secs(3_600))
        .ok_or("time")?;
    let scopes: BTreeSet<Scope> = [Scope::FileRead].into_iter().collect();
    let offer = ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
        [
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ]
        .into_iter()
        .collect(),
        false,
        true,
    )
    .authenticated(respondent);
    let question = Question::new(
        Subject::new(run, operation, digest, ContentDigest::over(&[b"s"])),
        requester,
        respondent,
        expiry,
        scopes,
    );
    runtime.submit_protocol(
        &offer,
        Command::new(
            command_id(81),
            tenant,
            Vec::new(),
            CommandContent::new(
                CommandKind::Ask,
                lgwks_std::json::to_vec(&question)?.into_boxed_slice(),
            ),
        )?,
        None,
    )?;
    runtime.submit(submit(command_id(82), &body)?)?;
    runtime.submit(support::cancel(command_id(83), &body)?)?;
    let tick = runtime.tick()?;
    assert!(
        tick.admitted().contains(&command_id(81)),
        "the question is admitted: {:?}",
        tick.admitted()
    );
    assert!(
        !tick.admitted().contains(&command_id(82)),
        "the work waits on its answer: {:?}",
        tick.admitted()
    );
    assert!(
        tick.admitted().contains(&command_id(83)),
        "the stop control passes the hold: {:?}",
        tick.admitted()
    );
    Ok(())
}

// Source: CWE-20 (hostile bytes on the wire) and larry#100 (nothing is
// written on a refusal). A payload that is not JSON, a question or answer
// that does not decode, and an undecodable acceptance record are each a
// typed codec refusal, and the journal does not move.
#[test]
fn sim_garbage_payloads_are_refused_typed_and_write_nothing() -> Outcome {
    let tenant = tenant(9);
    let mut runtime = runtime()?;
    let before = runtime.journal().len();
    let garbage = Command::new(
        command_id(90),
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Submit, Box::from(&b"not json"[..])),
    )?;
    let refused = runtime.submit_protocol(&submit_offer(tenant), garbage, None);
    assert!(
        matches!(refused, Err(RuntimeError::Codec(_))),
        "a garbage submit is a codec refusal: {refused:?}"
    );
    let bad_ask = Command::new(
        command_id(91),
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Ask, Box::from(&b"not json"[..])),
    )?;
    let offer = ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
        [
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ]
        .into_iter()
        .collect(),
        false,
        true,
    )
    .authenticated(WorkerId::from_bytes([0xA9; 16]));
    let refused_ask = runtime.submit_protocol(&offer, bad_ask, None);
    assert!(
        matches!(refused_ask, Err(RuntimeError::Codec(_))),
        "a garbage question is a codec refusal: {refused_ask:?}"
    );
    let bad_answer = Command::new(
        command_id(92),
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Answer, Box::from(&b"not json"[..])),
    )?;
    let refused_answer = runtime.submit_protocol(&offer, bad_answer, None);
    assert!(
        matches!(refused_answer, Err(RuntimeError::Codec(_))),
        "a garbage answer is a codec refusal: {refused_answer:?}"
    );
    assert_eq!(
        runtime.journal().len(),
        before,
        "codec refusals write nothing"
    );

    // An undecodable acceptance record fails the restore with the codec
    // cause instead of inventing a pending command.
    let mut journal = Journal::new(64)?;
    journal.record(Event::new(
        tenant,
        run(9),
        EventSequence::new(0),
        Vec::new(),
        EventKind::CommandAccepted,
        Box::from(&b"not json"[..]),
    )?)?;
    let restored: Result<TestRuntime, RuntimeError> = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        clock()?,
        journal,
    );
    assert!(
        matches!(restored, Err(RuntimeError::Codec(_))),
        "a garbage acceptance fails the restore"
    );
    Ok(())
}

// Source: E30 (the tick pins one instant) and the estate clock's saturation
// contract (lgwks_bot saturates rather than wraps). A clock at the estate
// ceiling still places a valid instant: saturation keeps admission working
// where a wrap would alias history.
#[test]
fn sim_clock_at_its_ceiling_still_admits() -> Outcome {
    let tenant = tenant(11);
    let run = run(11);
    let body = proposal(
        run,
        tenant,
        operation(11),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let settings = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    let mut runtime: TestRuntime = Runtime::new(
        settings,
        ScriptedOwner::new(),
        lgwks_bot::kernel::clock::Clock::virtual_at(
            lgwks_bot::kernel::clock::Clock::elapsed_ceiling(),
        ),
    )?;
    let before = runtime.journal().len();
    let admitted =
        runtime.submit_protocol(&submit_offer(tenant), submit(command_id(110), &body)?, None);
    assert!(
        matches!(admitted, Ok((CommandAcceptance::Initial, _))),
        "the ceiling places a valid instant: {admitted:?}"
    );
    assert!(
        runtime.journal().len() > before,
        "the admission is journaled"
    );
    Ok(())
}

// Source: larry#182 (the reconciliation budget is a fold) and Cell 05 (the
// journal never grows past its ceiling). A journal restored exactly full
// refuses the reconciliation record typed instead of dropping history.
#[test]
fn sim_full_journal_refuses_the_reconciliation_record() -> Outcome {
    use larry_contracts::replay::{
        BudgetSnapshot, CapabilitySnapshot, PinnedInputs, PolicySnapshot, ValiditySnapshot,
    };
    use larry_runtime::payload::{IntentPayload, OutcomePayload};
    let tenant = tenant(13);
    let run = run(13);
    let operation = operation(13);
    let validity = epoch()?
        .checked_add(Duration::from_secs(3_600))
        .ok_or("time")?;
    let pinned = PinnedInputs::new(
        larry_contracts::replay::EffectRequest::new(
            operation,
            [Scope::FileRead].into_iter().collect(),
            1,
            BudgetUnit::Requests,
            epoch()?,
        ),
        PolicySnapshot::new(1, [Scope::FileRead].into_iter().collect()),
        BudgetSnapshot::new(100, BudgetUnit::Requests),
        CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
        ValiditySnapshot::new(validity),
    );
    let effect = larry_contracts::effect::EffectIntent::new(
        larry_contracts::ids::EffectId::from_bytes([0x33; 16]),
        operation,
        run,
        tenant,
        ReconciliationStrategy::Queryable,
        larry_contracts::authority::SecurityEpoch::INITIAL,
    );
    let authorization = larry_contracts::replay::ReplayRecord::new(
        pinned,
        larry_contracts::replay::Decision::Authorized,
    );
    let mut journal = Journal::new(2)?;
    journal.record(Event::new(
        tenant,
        run,
        EventSequence::new(0),
        Vec::new(),
        EventKind::EffectIntentRecorded,
        lgwks_std::json::to_vec(&IntentPayload::new(effect, authorization))?.into_boxed_slice(),
    )?)?;
    journal.record(Event::new(
        tenant,
        run,
        EventSequence::new(1),
        Vec::new(),
        EventKind::EffectOutcomeRecorded,
        lgwks_std::json::to_vec(&OutcomePayload::new(
            effect,
            EffectOutcome::Unknown { operation },
            0,
        ))?
        .into_boxed_slice(),
    )?)?;
    let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    capped.journal_capacity = 2;
    let mut runtime: TestRuntime = restore(capped, ScriptedOwner::new(), clock()?, journal)?;
    let tick = runtime.tick();
    assert!(
        matches!(tick, Err(RuntimeError::JournalFull { capacity: 2 })),
        "a full journal refuses the next record: {tick:?}"
    );
    Ok(())
}

// Source: larry#333 (one tenant's rows never exhaust another's) and Cell 05.
// One run fills the table; the next run is refused naming the capacity, and
// a sealed run's slot frees for a later run of its own tenant.
#[test]
fn sim_run_table_refuses_past_its_capacity_by_name() -> Outcome {
    let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    capped.run_capacity = 1;
    let mut runtime: TestRuntime = Runtime::new(capped, ScriptedOwner::new(), clock()?)?;
    let tenant = tenant(14);
    let first = proposal(
        run(14),
        tenant,
        operation(14),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(140), &first)?)?;
    runtime.tick()?;
    let second = proposal(
        run(15),
        tenant,
        operation(15),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(141), &second)?)?;
    let refused = runtime.tick()?;
    assert!(
        refused
            .refused()
            .iter()
            .any(|entry| matches!(entry.1, RuntimeError::RunLimitReached { capacity: 1 })),
        "the table refuses past one: {:?}",
        refused.refused()
    );
    Ok(())
}

// Source: D42 (a run id names one run) and larry#333 (eviction frees only a
// sealed terminal row). A resubmission to a completed run whose row was
// evicted is refused at the algebra, never by re-running the run.
#[test]
fn sim_resubmission_to_an_evicted_completed_run_is_refused() -> Outcome {
    let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    capped.run_capacity = 4;
    capped.max_tenants = 4;
    let owner = ScriptedOwner::scripted([
        EffectOutcome::Succeeded {
            operation: operation(16),
        },
        EffectOutcome::Succeeded {
            operation: operation(17),
        },
    ]);
    let mut runtime: TestRuntime = Runtime::new(capped, owner, clock()?)?;
    let tenant = tenant(16);
    let first = proposal(
        run(16),
        tenant,
        operation(16),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(160), &first)?)?;
    runtime.tick()?;
    assert_state(
        runtime.projection(),
        run(16),
        larry_contracts::state::RunState::Completed,
    )?;
    // A second run of the same tenant evicts the sealed first row.
    let second = proposal(
        run(17),
        tenant,
        operation(17),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(161), &second)?)?;
    runtime.tick()?;
    // The completed run takes no new work: the tick refuses the resubmission
    // at the algebra, never by re-running the run.
    let retry = proposal(
        run(16),
        tenant,
        operation(18),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(162), &retry)?)?;
    let refused = runtime.tick()?;
    assert!(
        refused.refused().iter().any(|entry| matches!(
            entry.0,
            larry_runtime::runtime::TickItem::Command(id) if id == command_id(162)
        )),
        "the resubmission is refused: {:?}",
        refused.refused()
    );
    assert!(
        refused
            .refused()
            .iter()
            .any(|entry| matches!(entry.1, RuntimeError::Transition(_))),
        "a completed run takes no new work: {:?}",
        refused.refused()
    );
    Ok(())
}
