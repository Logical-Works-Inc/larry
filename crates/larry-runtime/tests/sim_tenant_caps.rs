//! larry#333: one tenant can no longer use up a cap every tenant shares.
//!
//! Each of the run table, the pending set (`MAX_READY_WORK`) and the pending
//! questions gives one tenant at most `capacity / max_tenants`, never less than
//! one. Past its share a tenant is refused with a typed error naming that cap,
//! and another tenant still admits, runs and asks. A terminal run whose receipt
//! is sealed leaves the run table, so its slot frees for the next run. Each
//! drive is a seeded simulation run twice per seed, so a divergence surfaces as
//! a failing seed rather than a silent pass.

mod sim_kit;
mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use larry_contracts::authority::Scope;
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::ProjectionBackend;
use larry_runtime::protocol::{ProtocolCapability, ProtocolOffer, ProtocolVersionRange};
use larry_runtime::runtime::{
    Question, QuestionError, Runtime, RuntimeSettings, Subject, TickItem,
};

use sim_kit::{Sim, replay};
use support::{
    TestRuntime, clock, command_id, epoch, operation, proposal, run, settings, submit, succeeded,
    tenant, unknown,
};

/// Two tenants share every cap. The run table holds eight, so each holds four;
/// four pending questions give each two; the compiled pending set holds 256, so
/// each holds 128.
const RUN_CAPACITY: usize = 8;
const MAX_TENANTS: usize = 2;
const RUN_SHARE: usize = 4;
const MAX_QUESTIONS: usize = 4;
const QUESTION_SHARE: usize = 2;
const READY_WORK: usize = larry_runtime::MAX_READY_WORK;
const READY_SHARE: usize = 128;
/// The share a sealed run must free in the eviction drive (four tenants).
const SEALED_TENANTS: usize = 4;
const SEALED_SHARE: usize = 2;

const _: () = assert!(
    READY_WORK == 256,
    "READY_SHARE divides the compiled pending bound"
);

/// The counts one seeded drive records, replayed per seed.
type Tally = (usize, usize);

/// Settings where two tenants share every cap.
fn capped() -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let mut settings = settings(&[Scope::FileRead], &[Scope::FileRead], 100_000)?;
    settings.run_capacity = RUN_CAPACITY;
    settings.max_tenants = MAX_TENANTS;
    settings.max_pending_questions = MAX_QUESTIONS;
    Ok(settings)
}

/// One `FileRead` work proposal for `run`'s `op` under `tenant`.
fn read(tenant: TenantId, run: RunId, op: OperationId) -> WorkProposal {
    proposal(
        run,
        tenant,
        op,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    )
}

/// A client with a human at it, authenticated as `principal`.
fn human(tenant: TenantId, principal: WorkerId) -> ProtocolOffer {
    ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(1, 1),
        BTreeSet::from([
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ]),
        false,
        false,
    )
    .authenticated(principal)
}

/// A client that only submits commands, headless.
fn client(tenant: TenantId) -> ProtocolOffer {
    ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(1, 1),
        BTreeSet::from([ProtocolCapability::SubmitCommand]),
        true,
        false,
    )
}

/// The worker a question names.
fn worker(byte: u8) -> WorkerId {
    WorkerId::from_bytes([byte.saturating_add(1); 16])
}

/// An `Ask` command about `run`'s `op`, from `requester` to `respondent`.
fn ask(
    id: CommandId,
    tenant: TenantId,
    run: RunId,
    op: OperationId,
    requester: WorkerId,
    respondent: WorkerId,
) -> Result<Command, Box<dyn std::error::Error>> {
    let subject = Subject::new(
        run,
        op,
        ContentDigest::over(&[b"larry#333/effect"]),
        ContentDigest::over(&[b"larry#333/schema"]),
    );
    let expires_at = epoch()?
        .checked_add(Duration::from_secs(600))
        .ok_or("question expiry overflow")?;
    let question = Question::new(subject, requester, respondent, expires_at, BTreeSet::new());
    let bytes = lgwks_std::json::to_vec(&question)?.into_boxed_slice();
    Ok(Command::new(
        id,
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Ask, bytes),
    )?)
}

/// A `Steer` command carrying `body`, so it enters the pending set and waits.
fn steer(id: CommandId, body: &WorkProposal) -> Result<Command, Box<dyn std::error::Error>> {
    let bytes = lgwks_std::json::to_vec(body)?.into_boxed_slice();
    Ok(Command::new(
        id,
        body.tenant(),
        Vec::new(),
        CommandContent::new(CommandKind::Steer, bytes),
    )?)
}

/// A command identity from an index and a tag, distinct for every index.
fn wide(index: u32, tag: u8) -> [u8; 16] {
    let mut bytes = [tag; 16];
    bytes[..4].copy_from_slice(&index.to_be_bytes());
    bytes[12] = tag.wrapping_add(1);
    bytes
}

/// Tenant `first` holds its whole run share with four open runs, so the table
/// is full for it and empty for `second`.
fn fill_run_share(runtime: &mut TestRuntime, first: TenantId) -> Sim<()> {
    for index in 1_u8..=4 {
        let body = read(first, run(index), operation(index));
        runtime.submit(submit(command_id(index), &body)?)?;
    }
    runtime.tick()?;
    for index in 1_u8..=4 {
        assert_eq!(
            runtime.projection().run_state(run(index)),
            Some(RunState::Reconciling),
            "tenant 1's run {index} stays open"
        );
    }
    Ok(())
}

/// Tenant `first`'s fifth run is refused naming its share, and `second`'s run
/// is admitted, dispatched and sealed. Returns how many refusals the tick
/// recorded.
fn refuse_fifth_run(runtime: &mut TestRuntime, first: TenantId, second: TenantId) -> Sim<usize> {
    runtime.submit(submit(command_id(5), &read(first, run(5), operation(10)))?)?;
    runtime.submit(submit(command_id(6), &read(second, run(6), operation(5)))?)?;
    let tick = runtime.tick()?;
    assert!(
        tick.refused().iter().any(|entry| {
            entry.0 == TickItem::Command(command_id(5))
                && matches!(&entry.1, RuntimeError::RunLimitReached { capacity }
                    if *capacity == RUN_SHARE)
        }),
        "tenant 1 past its run share is refused naming it: {tick:?}"
    );
    assert_eq!(
        runtime.projection().run_state(run(6)),
        Some(RunState::Completed),
        "tenant 2 still runs its run"
    );
    assert!(
        runtime
            .projection()
            .receipts()
            .iter()
            .any(|receipt| receipt.run() == run(6)),
        "tenant 2's receipt is sealed"
    );
    Ok(tick.refused().len())
}

/// Tenant `first` holds its whole question share; its next question is refused
/// naming the share, and `second`'s question is admitted.
fn fill_question_share(runtime: &mut TestRuntime, first: TenantId, second: TenantId) -> Sim<()> {
    for (index, target) in [(0_u32, (run(1), operation(1))), (1, (run(2), operation(2)))] {
        let item = ask(
            CommandId::from_bytes(wide(index, 0x21)),
            first,
            target.0,
            target.1,
            worker(1),
            worker(2),
        )?;
        runtime.submit_protocol(&human(first, worker(2)), item, None)?;
    }
    let third = ask(
        CommandId::from_bytes(wide(2, 0x21)),
        first,
        run(3),
        operation(3),
        worker(1),
        worker(2),
    )?;
    match runtime.submit_protocol(&human(first, worker(2)), third, None) {
        Err(RuntimeError::Question(QuestionError::PendingFull { capacity })) => {
            assert_eq!(
                capacity, QUESTION_SHARE,
                "the refusal names tenant 1's share"
            );
        }
        other => return Err(format!("the third question must be refused: {other:?}").into()),
    }
    let second_question = ask(
        CommandId::from_bytes(wide(3, 0x31)),
        second,
        run(7),
        operation(7),
        worker(3),
        worker(4),
    )?;
    runtime.submit_protocol(&human(second, worker(4)), second_question, None)?;
    Ok(())
}

/// Tenant `first` fills its whole share of the pending set; its next command is
/// refused naming the share, and `second`'s command is admitted. `already` is
/// how many commands `first` already holds. Returns how many commands it took.
fn fill_ready_share(
    runtime: &mut TestRuntime,
    first: TenantId,
    second: TenantId,
    already: usize,
) -> Sim<usize> {
    let filler = read(first, run(8), operation(8));
    let mut filled = 0_usize;
    loop {
        let id = CommandId::from_bytes(wide(u32::try_from(filled)?, 0x41));
        let before = runtime.journal().len();
        match runtime.submit_protocol(&client(first), steer(id, &filler)?, None) {
            Ok(_) => filled = filled.saturating_add(1),
            Err(RuntimeError::ReadyWorkFull { capacity }) => {
                assert_eq!(capacity, READY_SHARE, "the refusal names tenant 1's share");
                assert_eq!(
                    runtime.journal().len(),
                    before,
                    "the refusal writes nothing"
                );
                break;
            }
            Err(other) => {
                return Err(
                    format!("tenant 1 past its pending share must be refused: {other:?}").into(),
                );
            }
        }
        assert!(
            filled <= READY_SHARE,
            "the pending share never bounded tenant 1"
        );
    }
    // The boundary is exact: the tenant holds its whole share, no more and no
    // fewer, when the refusal comes.
    assert_eq!(
        filled.saturating_add(already),
        READY_SHARE,
        "tenant 1 held exactly its share when refused"
    );
    runtime.submit_protocol(
        &client(second),
        steer(
            CommandId::from_bytes(wide(0, 0x42)),
            &read(second, run(9), operation(9)),
        )?,
        None,
    )?;
    Ok(filled)
}

/// Tenant A fills its share of the run table, the pending set and the pending
/// questions, and tenant B still admits a run, runs it to a sealed receipt, and
/// asks a question, for every seed.
#[test]
fn sim_a_tenant_at_its_share_refuses_and_another_tenant_still_admits_runs_and_asks() -> Sim<()> {
    replay(0_u64..8, |_| -> Sim<Tally> {
        let first = tenant(1);
        let second = tenant(2);
        let owner = ScriptedOwner::scripted([
            unknown(operation(1)),
            unknown(operation(2)),
            unknown(operation(3)),
            unknown(operation(4)),
            succeeded(operation(5)),
        ]);
        let mut runtime: TestRuntime = Runtime::new(capped()?, owner, clock()?)?;
        fill_run_share(&mut runtime, first)?;
        let refused = refuse_fifth_run(&mut runtime, first, second)?;
        fill_question_share(&mut runtime, first, second)?;
        // The two questions above are already pending for tenant 1.
        fill_ready_share(&mut runtime, first, second, 2)?;
        Ok((refused, runtime.journal().len()))
    })
}

/// A terminal run whose receipt is sealed leaves the run table, so a tenant
/// that completed its whole share admits another run in that slot, for every
/// seed; the completed runs still read `Completed` from the projection.
#[test]
fn sim_a_sealed_terminal_run_leaves_the_table_and_frees_its_slot() -> Sim<()> {
    replay(8_u64..16, |_| -> Sim<usize> {
        let mut settings = settings(&[Scope::FileRead], &[Scope::FileRead], 100_000)?;
        settings.run_capacity = RUN_CAPACITY;
        settings.max_tenants = SEALED_TENANTS;
        let owner = ScriptedOwner::scripted([
            succeeded(operation(11)),
            succeeded(operation(12)),
            succeeded(operation(13)),
            succeeded(operation(14)),
        ]);
        let mut runtime: TestRuntime = Runtime::new(settings, owner, clock()?)?;
        let first = tenant(1);
        // The share is two runs; the third and the fourth fit only because
        // each completed run before them is sealed and freed.
        assert_eq!(SEALED_SHARE, 2, "the share this drive fills");
        for index in 1_u8..=3 {
            let body = read(first, run(index), operation(index.saturating_add(10)));
            runtime.submit(submit(command_id(index), &body)?)?;
            let tick = runtime.tick()?;
            assert!(
                tick.refused().is_empty(),
                "run {index} is admitted into a freed slot: {tick:?}"
            );
            assert_eq!(
                runtime.projection().run_state(run(index)),
                Some(RunState::Completed),
                "run {index} completes and seals its receipt"
            );
        }
        for index in 1_u8..=3 {
            assert_eq!(
                runtime.projection().run_state(run(index)),
                Some(RunState::Completed),
                "a freed run keeps its recorded state"
            );
        }
        assert_eq!(
            runtime
                .projection()
                .receipts()
                .iter()
                .filter(|receipt| receipt.tenant() == first)
                .count(),
            3,
            "every completed run keeps its receipt"
        );
        Ok(runtime.journal().len())
    })
}

/// A tenant whose pending set is full has its queued command pass behind
/// another tenant's, so the other tenant's command still admits in that tick.
#[test]
fn sim_a_full_tenants_queued_command_never_holds_another_tenants() -> Sim<()> {
    replay(16_u64..24, |_| -> Sim<usize> {
        let holder = tenant(1);
        let behind = tenant(2);
        let owner = ScriptedOwner::scripted([
            unknown(operation(1)),
            unknown(operation(2)),
            unknown(operation(3)),
            unknown(operation(4)),
        ]);
        let mut runtime: TestRuntime = Runtime::new(capped()?, owner, clock()?)?;
        fill_run_share(&mut runtime, holder)?;
        // Tenant 1 fills its pending share with corrections its open run holds.
        let filler = read(holder, run(1), operation(1));
        for index in 0..u32::try_from(READY_SHARE)? {
            let id = CommandId::from_bytes(wide(index, 0x51));
            runtime.submit_protocol(&client(holder), steer(id, &filler)?, None)?;
        }
        // Tenant 1's command sits ahead of tenant 2's in the ready queue.
        let waiting = CommandId::from_bytes(wide(0, 0x52));
        runtime.submit(steer(waiting, &filler)?)?;
        let body = proposal(
            run(6),
            behind,
            operation(6),
            &[Scope::FileRead],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(submit(command_id(7), &body)?)?;
        let tick = runtime.tick()?;
        assert!(
            tick.admitted().contains(&command_id(7)),
            "tenant 2 admits behind a full tenant: {tick:?}"
        );
        assert!(
            !tick.admitted().contains(&waiting),
            "the full tenant's queued command waits: {tick:?}"
        );
        Ok(tick.admitted().len())
    })
}

/// A terminal run without a sealed receipt keeps its slot: only a receipt
/// frees the table row, and another run's receipt frees nothing.
#[test]
fn sim_a_terminal_run_without_a_receipt_keeps_its_slot() -> Sim<()> {
    replay(24_u64..32, |_| -> Sim<usize> {
        let mut settings = settings(&[Scope::FileRead], &[Scope::FileRead], 100_000)?;
        // Four tenants share four runs, so each holds one.
        settings.run_capacity = SEALED_TENANTS;
        settings.max_tenants = SEALED_TENANTS;
        let owner = ScriptedOwner::scripted([succeeded(operation(20))]);
        let mut runtime: TestRuntime = Runtime::new(settings, owner, clock()?)?;
        let denied_tenant = tenant(1);
        let sealed_tenant = tenant(2);
        // Tenant 2 completes one run and seals its receipt.
        let sealed = proposal(
            run(6),
            sealed_tenant,
            operation(20),
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        runtime.submit(submit(command_id(1), &sealed)?)?;
        // Tenant 1's run is denied by the policy: it fails with no receipt.
        let denied = proposal(
            run(1),
            denied_tenant,
            operation(1),
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.submit(submit(command_id(2), &denied)?)?;
        let tick = runtime.tick()?;
        assert!(
            tick.receipts()
                .iter()
                .any(|receipt| receipt.run() == run(6)),
            "tenant 2's receipt is sealed"
        );
        assert_eq!(
            runtime.projection().run_state(run(1)),
            Some(RunState::Failed),
            "tenant 1's run failed with no receipt"
        );
        // Tenant 1's only slot stays taken: a failed run with no receipt is
        // not the sealed terminal run that frees one.
        runtime.submit(submit(
            command_id(3),
            &read(denied_tenant, run(2), operation(2)),
        )?)?;
        let second_tick = runtime.tick()?;
        assert!(
            second_tick.refused().iter().any(|entry| {
                entry.0 == TickItem::Command(command_id(3))
                    && matches!(&entry.1, RuntimeError::RunLimitReached { capacity }
                        if *capacity == 1)
            }),
            "the slot is not freed without a receipt: {second_tick:?}"
        );
        Ok(second_tick.refused().len())
    })
}
