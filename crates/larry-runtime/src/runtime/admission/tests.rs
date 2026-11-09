//! E28 (larry#100, specification R44, D25): one accepted identity across a
//! crash after durable admission, a conflicting retry refused, and a
//! correction promoted only at a safe boundary.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::EventKind;
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_contracts::state::RunState;
use lgwks_bot::clock::Clock;

use crate::MAX_READY_WORK;
use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::owner::fixture::ScriptedOwner;
use crate::payload::{CommandAcceptedPayload, WorkProposal};
use crate::projection::{FoldProjection, ProjectionBackend};
use crate::runtime::{Runtime, RuntimeSettings, restore};

type Outcome = Result<(), Box<dyn std::error::Error>>;
type Rt = Runtime<FoldProjection, ScriptedOwner>;

/// The instant every runtime here starts at.
const START_SECS: u64 = 1_700_000_000;

fn ids(byte: u8, seed: u64) -> [u8; 16] {
    let mut bytes = [byte; 16];
    for (slot, mix) in bytes.iter_mut().skip(8).zip(seed.to_be_bytes()) {
        *slot ^= mix;
    }
    bytes
}

fn settings() -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let until = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(START_SECS + 3_600))
        .ok_or("fixture time overflow")?;
    Ok(RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, [Scope::FileRead].into_iter().collect()),
        CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
        ValiditySnapshot::new(until),
        BudgetUnit::Requests,
        100,
    ))
}

fn clock() -> Clock {
    Clock::virtual_at(Duration::from_secs(START_SECS))
}

fn memory(owner: ScriptedOwner) -> Result<Rt, Box<dyn std::error::Error>> {
    Ok(Runtime::new(settings()?, owner, clock())?)
}

fn durable(owner: ScriptedOwner, path: &Path) -> Result<Rt, Box<dyn std::error::Error>> {
    let settings = settings()?;
    let journal = Journal::open(path, settings.journal_capacity)?;
    Ok(restore(settings, owner, clock(), journal)?)
}

fn journal_file(name: &str, seed: u64) -> Result<PathBuf, std::io::Error> {
    let path = std::env::temp_dir().join(format!(
        "larry-e28-{}-{name}-{seed}.journal",
        std::process::id()
    ));
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(path),
    }
}

/// A unit of work: one `FileRead` effect costing one request.
fn work_body(run: RunId, tenant: TenantId, operation: OperationId) -> WorkProposal {
    WorkProposal::new(
        run,
        tenant,
        operation,
        [Scope::FileRead].into_iter().collect(),
        1,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    )
}

/// The body of a correction or a cancellation: no scope, no cost.
fn idle_body(run: RunId, tenant: TenantId, operation: OperationId) -> WorkProposal {
    WorkProposal::new(
        run,
        tenant,
        operation,
        BTreeSet::new(),
        0,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    )
}

fn command(
    id: CommandId,
    kind: CommandKind,
    body: &WorkProposal,
) -> Result<Command, Box<dyn std::error::Error>> {
    let payload = lgwks_std::json::to_vec(body)?.into_boxed_slice();
    Ok(Command::new(
        id,
        body.tenant,
        Vec::new(),
        CommandContent::new(kind, payload),
    )?)
}

/// The commands the journal records an acceptance for, in order.
fn accepted(journal: &Journal) -> Result<Vec<CommandId>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for entry in journal.entries() {
        if let JournalEntry::Event(ref event) = *entry
            && event.kind() == EventKind::CommandAccepted
        {
            let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
            out.push(payload.command);
        }
    }
    Ok(out)
}

/// Admission without a display, the form every case but the display one uses.
trait Ack {
    fn ack(&mut self, command: Command) -> Result<CommandAcceptance, RuntimeError>;
}

impl Ack for Rt {
    fn ack(&mut self, command: Command) -> Result<CommandAcceptance, RuntimeError> {
        self.admit(command, None)
    }
}

fn pending(runtime: &Rt) -> Vec<CommandId> {
    runtime
        .admitted
        .iter()
        .map(|admitted| admitted.command.id())
        .collect()
}

/// R44/E28: for every seed, an acknowledged admission survives the runtime's
/// death before any tick, is reduced exactly once after restore, a retry is a
/// duplicate, and a conflicting retry is refused with nothing written.
#[test]
fn sim_an_acknowledged_admission_survives_a_crash_and_is_reduced_once() -> Outcome {
    for seed in 0..32_u64 {
        let path = journal_file("ack", seed)?;
        let (run, tenant) = (
            RunId::from_bytes(ids(1, seed)),
            TenantId::from_bytes(ids(2, seed)),
        );
        let body = work_body(run, tenant, OperationId::from_bytes(ids(3, seed)));
        let id = CommandId::from_bytes(ids(4, seed));
        let first = command(id, CommandKind::Submit, &body)?;
        let mut runtime = durable(ScriptedOwner::new(), &path)?;
        assert_eq!(runtime.ack(first.clone())?, CommandAcceptance::Initial);
        let len = runtime.journal().len();
        // The control: a command only queued in memory is not acknowledged and
        // does not survive, so the survival below is admission's sync.
        let queued = CommandId::from_bytes(ids(5, seed));
        runtime.submit(command(queued, CommandKind::Steer, &body)?)?;
        drop(runtime);

        let mut restored = durable(ScriptedOwner::new(), &path)?;
        assert_eq!(
            restored.journal().len(),
            len,
            "seed {seed}: the ack is on disk"
        );
        assert!(
            restored.journal().command(queued).is_none(),
            "seed {seed}: no queue entry"
        );
        assert_eq!(
            pending(&restored),
            [id],
            "seed {seed}: pending again after restore"
        );
        assert_eq!(restored.ack(first)?, CommandAcceptance::DuplicateReplay);
        assert_eq!(
            restored.tick()?.admitted(),
            &[id],
            "seed {seed}: reduced once"
        );
        assert!(pending(&restored).is_empty());

        let mut changed = body.clone();
        changed.cost = 2;
        let before = restored.journal().len();
        let conflict = restored.ack(command(id, CommandKind::Submit, &changed)?);
        assert!(
            matches!(conflict, Err(RuntimeError::Command(_))),
            "seed {seed}: a conflicting retry is refused: {conflict:?}"
        );
        assert_eq!(
            restored.journal().len(),
            before,
            "seed {seed}: nothing written"
        );
        assert!(
            restored.tick()?.admitted().is_empty(),
            "seed {seed}: no second run"
        );
        assert_eq!(accepted(restored.journal())?, [id]);
        drop(restored);
        let reopened = durable(ScriptedOwner::new(), &path)?;
        assert!(
            pending(&reopened).is_empty(),
            "seed {seed}: the reduction is on disk"
        );
        drop(reopened);
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// E28: a correction submitted while its run's effect is unresolved stays
/// pending through reconciliation, a later command of that run waits behind it,
/// another run passes it, and it is promoted on the next boundary.
#[test]
fn a_correction_waits_for_its_runs_effect_to_settle() -> Outcome {
    let (run, other, tenant) = (
        RunId::from_bytes([1; 16]),
        RunId::from_bytes([2; 16]),
        TenantId::from_bytes([3; 16]),
    );
    let work = OperationId::from_bytes([4; 16]);
    let owner = ScriptedOwner::scripted([
        EffectOutcome::Unknown { operation: work },
        EffectOutcome::Succeeded { operation: work },
    ]);
    let mut runtime = memory(owner)?;
    let submit = CommandId::from_bytes([5; 16]);
    runtime.ack(command(
        submit,
        CommandKind::Submit,
        &work_body(run, tenant, work),
    )?)?;
    assert_eq!(runtime.tick()?.admitted(), &[submit]);

    let steer = CommandId::from_bytes([6; 16]);
    let cancel = CommandId::from_bytes([7; 16]);
    let elsewhere = CommandId::from_bytes([8; 16]);
    let idle = OperationId::from_bytes([9; 16]);
    runtime.ack(command(
        steer,
        CommandKind::Steer,
        &idle_body(run, tenant, idle),
    )?)?;
    runtime.ack(command(
        cancel,
        CommandKind::Cancel,
        &idle_body(run, tenant, idle),
    )?)?;
    runtime.ack(command(
        elsewhere,
        CommandKind::Steer,
        &idle_body(other, tenant, idle),
    )?)?;

    let held = runtime.tick()?;
    assert_eq!(
        held.admitted(),
        &[elsewhere],
        "another run passes the held correction"
    );
    assert_eq!(
        held.reconciled(),
        &[work],
        "the effect settles in this tick's stage 3"
    );
    assert_eq!(
        pending(&runtime),
        [steer, cancel],
        "the correction held, the cancel behind it"
    );

    let promoted = runtime.tick()?;
    assert_eq!(
        promoted.admitted(),
        &[steer, cancel],
        "promoted at the next boundary, in order"
    );
    assert_eq!(
        accepted(runtime.journal())?,
        [submit, elsewhere, steer, cancel]
    );
    assert_eq!(
        runtime.projection().run_state(run),
        Some(RunState::Completed)
    );
    Ok(())
}

/// E28: a permanently unknown effect is not a definite disposition, so the
/// correction stays pending however many ticks pass.
#[test]
fn a_permanent_unknown_keeps_a_correction_pending() -> Outcome {
    let (run, tenant) = (RunId::from_bytes([1; 16]), TenantId::from_bytes([2; 16]));
    let work = OperationId::from_bytes([3; 16]);
    let owner = ScriptedOwner::scripted([EffectOutcome::Unknown { operation: work }]);
    let mut runtime = memory(owner)?;
    let mut body = work_body(run, tenant, work);
    body.strategy = ReconciliationStrategy::RequiresHuman;
    runtime.ack(command(
        CommandId::from_bytes([4; 16]),
        CommandKind::Submit,
        &body,
    )?)?;
    runtime.tick()?;
    let steer = CommandId::from_bytes([5; 16]);
    let idle = OperationId::from_bytes([6; 16]);
    runtime.ack(command(
        steer,
        CommandKind::Steer,
        &idle_body(run, tenant, idle),
    )?)?;
    for _ in 0..4 {
        assert!(
            runtime.tick()?.admitted().is_empty(),
            "the correction is never promoted"
        );
    }
    assert_eq!(pending(&runtime), [steer]);
    assert_eq!(
        runtime.projection().run_state(run),
        Some(RunState::OutcomeUnknown)
    );
    Ok(())
}

/// D42/E28: a second tenant cannot admit onto a run whose first command is
/// still pending, and its refusal writes nothing.
#[test]
fn a_foreign_tenant_cannot_admit_onto_a_pending_run() -> Outcome {
    let run = RunId::from_bytes([1; 16]);
    let (owner_tenant, foreign) = (TenantId::from_bytes([2; 16]), TenantId::from_bytes([3; 16]));
    let work = OperationId::from_bytes([4; 16]);
    let mut runtime = memory(ScriptedOwner::new())?;
    let first = work_body(run, owner_tenant, work);
    runtime.ack(command(
        CommandId::from_bytes([5; 16]),
        CommandKind::Submit,
        &first,
    )?)?;
    let before = runtime.journal().len();
    let steer = idle_body(run, foreign, OperationId::from_bytes([6; 16]));
    let refused = runtime.ack(command(
        CommandId::from_bytes([7; 16]),
        CommandKind::Steer,
        &steer,
    )?);
    assert!(
        matches!(refused, Err(RuntimeError::RunTenantMismatch { got, .. }) if got == foreign),
        "the foreign tenant is refused: {refused:?}"
    );
    assert_eq!(runtime.journal().len(), before, "the refusal wrote nothing");
    assert_eq!(pending(&runtime).len(), 1);
    Ok(())
}

/// E28/E47: a duplicate in the ready queue is reported and skipped, and the
/// commands queued after it are still reduced in the same tick.
#[test]
fn a_duplicate_in_the_ready_queue_does_not_stop_the_tick() -> Outcome {
    let tenant = TenantId::from_bytes([1; 16]);
    let mut runtime = memory(ScriptedOwner::new())?;
    let first = command(
        CommandId::from_bytes([2; 16]),
        CommandKind::Steer,
        &idle_body(
            RunId::from_bytes([3; 16]),
            tenant,
            OperationId::from_bytes([4; 16]),
        ),
    )?;
    runtime.submit(first.clone())?;
    assert_eq!(runtime.tick()?.admitted(), &[first.id()]);
    let next = command(
        CommandId::from_bytes([5; 16]),
        CommandKind::Steer,
        &idle_body(
            RunId::from_bytes([6; 16]),
            tenant,
            OperationId::from_bytes([7; 16]),
        ),
    )?;
    runtime.submit(first.clone())?;
    runtime.submit(next.clone())?;
    let tick = runtime.tick()?;
    assert_eq!(tick.duplicates(), &[first.id()], "the retry is a duplicate");
    assert_eq!(
        tick.admitted(),
        &[next.id()],
        "the queue behind it still drains"
    );
    Ok(())
}

/// E28: a correction queued in memory is held while its run's effect is open,
/// joins the pending set, and the next run's queued command still passes it in
/// the same tick.
#[test]
fn a_held_queued_correction_lets_the_next_run_pass() -> Outcome {
    let (run, other, tenant) = (
        RunId::from_bytes([1; 16]),
        RunId::from_bytes([2; 16]),
        TenantId::from_bytes([3; 16]),
    );
    let work = OperationId::from_bytes([4; 16]);
    let mut runtime = memory(ScriptedOwner::new())?;
    runtime.submit(command(
        CommandId::from_bytes([5; 16]),
        CommandKind::Submit,
        &work_body(run, tenant, work),
    )?)?;
    runtime.tick()?;
    let idle = OperationId::from_bytes([6; 16]);
    let held = command(
        CommandId::from_bytes([7; 16]),
        CommandKind::Steer,
        &idle_body(run, tenant, idle),
    )?;
    let passing = command(
        CommandId::from_bytes([8; 16]),
        CommandKind::Steer,
        &idle_body(other, tenant, idle),
    )?;
    runtime.submit(held.clone())?;
    runtime.submit(passing.clone())?;
    assert_eq!(
        runtime.tick()?.admitted(),
        &[passing.id()],
        "the next run passes"
    );
    assert_eq!(pending(&runtime), [held.id()], "the correction is pending");
    Ok(())
}

/// Cell 05: the pending set holds at most `MAX_READY_WORK` commands; the next
/// is refused with nothing written, and a tick drains room for it.
#[test]
fn the_pending_set_refuses_past_its_bound_and_recovers() -> Outcome {
    let tenant = TenantId::from_bytes([1; 16]);
    let mut runtime = memory(ScriptedOwner::new())?;
    let steer = |index: u32| -> Result<Command, Box<dyn std::error::Error>> {
        let mut bytes = [9_u8; 16];
        bytes[..4].copy_from_slice(&index.to_be_bytes());
        let body = idle_body(
            RunId::from_bytes(bytes),
            tenant,
            OperationId::from_bytes(bytes),
        );
        command(CommandId::from_bytes(bytes), CommandKind::Steer, &body)
    };
    let bound = u32::try_from(MAX_READY_WORK)?;
    for index in 0..bound {
        runtime.ack(steer(index)?)?;
    }
    let before = runtime.journal().len();
    let refused = runtime.ack(steer(bound)?);
    assert!(
        matches!(refused, Err(RuntimeError::ReadyWorkFull { capacity }) if capacity == MAX_READY_WORK),
        "past the bound is refused: {refused:?}"
    );
    assert_eq!(runtime.journal().len(), before, "the refusal wrote nothing");
    assert!(
        !runtime.tick()?.admitted().is_empty(),
        "a tick drains the set"
    );
    assert_eq!(runtime.ack(steer(bound)?)?, CommandAcceptance::Initial);
    Ok(())
}
