//! Deterministic E28 fault schedules through the public runtime interface.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rocco_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use rocco_contracts::error::CommandError;
use rocco_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId};
use rocco_runtime::JournalStorage;
use rocco_runtime::clock::FixedClock;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::journal::{Journal, JournalEntry};
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::ProjectionBackend;
use rocco_runtime::runtime::{Runtime, restore};

use super::{Store, TestResult, TestRuntime, command, raw, reopen, settings};

/// Runs a deterministic fault schedule and returns the complete trace digest.
fn simulate(seed: u64) -> TestResult<ContentDigest> {
    let store = Arc::new(Store::default());
    let op = OperationId::from_bytes(raw(seed));
    let work = command(seed, 1, CommandKind::Submit, seed & 7)?;
    let steer = command(seed, 1, CommandKind::Steer, 0)?;
    let mut runtime = reopen(&store, seed, vec![EffectOutcome::Unknown { operation: op }])?;
    assert_eq!(
        runtime.admit(work.clone())?,
        CommandAcceptance::Initial,
        "first durable admission"
    );
    drop(runtime);
    let mut runtime = reopen(&store, seed, vec![EffectOutcome::Unknown { operation: op }])?;
    assert_eq!(
        runtime.admit(work.clone())?,
        CommandAcceptance::DuplicateReplay,
        "retry after lost ack"
    );
    let conflict = command(seed, 1, CommandKind::Submit, 50)?;
    assert!(
        matches!(
            runtime.admit(conflict),
            Err(RuntimeError::Command(CommandError::ConflictingRetry { .. }))
        ),
        "conflicting retry must be typed"
    );
    runtime.tick()?;
    assert_eq!(
        runtime.owner().dispatched(),
        &[op],
        "accepted work executes once"
    );
    assert_eq!(
        runtime.admit(steer.clone())?,
        CommandAcceptance::Initial,
        "correction is durably accepted"
    );
    for _ in 0..(seed & 3) {
        assert!(
            runtime.tick()?.promoted().is_empty(),
            "seeded delay cannot activate correction"
        );
    }
    let held = runtime.tick()?;
    assert_eq!(
        held.deferred(),
        &[steer.id()],
        "unresolved effect holds correction"
    );
    assert_eq!(
        runtime.correction(work.tenant(), RunId::from_bytes(raw(seed)))?,
        None,
        "pending is not active"
    );
    drop(runtime);
    let mut runtime = reopen(
        &store,
        seed,
        vec![EffectOutcome::Succeeded { operation: op }],
    )?;
    assert_eq!(
        runtime.admit(steer.clone())?,
        CommandAcceptance::DuplicateReplay,
        "pending identity survives recovery"
    );
    let reconciled = runtime.tick()?;
    assert_eq!(
        reconciled.deferred(),
        &[steer.id()],
        "admission precedes reconciliation"
    );
    assert!(reconciled.promoted().is_empty(), "no promotion mid-effect");
    let promoted = runtime.tick()?;
    assert_eq!(
        promoted.promoted(),
        &[steer.id()],
        "next declared boundary promotes"
    );
    assert_eq!(
        runtime.correction(work.tenant(), RunId::from_bytes(raw(seed)))?,
        Some(&steer),
        "actual correction content is active"
    );
    assert!(
        runtime.owner().dispatched().is_empty(),
        "recovery reconciles rather than redispatches"
    );
    let entries = lgwks_std::json::to_vec(&runtime.journal().entries())?;
    let mut restored = reopen(&store, seed, Vec::new())?;
    assert!(
        restored.tick()?.promoted().is_empty(),
        "promotion is not repeated on restart"
    );
    assert_eq!(
        restored.correction(work.tenant(), RunId::from_bytes(raw(seed)))?,
        Some(&steer),
        "promotion survives restart"
    );
    Ok(ContentDigest::over(&[&entries]))
}

/// One seed determines identities, cost, policy revision and crash/reply schedule.
#[test]
fn sim_e28_thousand_seed_replay() -> TestResult {
    for seed in 1..=1024 {
        assert_eq!(
            simulate(seed)?,
            simulate(seed)?,
            "same seed must yield same full trace: {seed}"
        );
    }
    Ok(())
}

/// Tenant isolation includes accepted-but-not-yet-projected run ownership.
#[test]
fn sim_e28_two_tenants_same_resource() -> TestResult {
    let store = Arc::new(Store::default());
    let mut runtime = reopen(&store, 7, Vec::new())?;
    let work = command(7, 1, CommandKind::Submit, 1)?;
    runtime.admit(work)?;
    let before = runtime.journal().len();
    assert!(
        matches!(
            runtime.admit(command(7, 2, CommandKind::Submit, 1)?),
            Err(RuntimeError::Command(CommandError::TenantMismatch { .. }))
        ),
        "foreign retry is not a dedup hit"
    );
    assert!(
        matches!(
            runtime.admit(command(7, 2, CommandKind::Steer, 0)?),
            Err(RuntimeError::RunTenantMismatch { .. })
        ),
        "foreign correction refused before projection exists"
    );
    assert_eq!(
        runtime.journal().len(),
        before,
        "foreign commands mutate nothing"
    );
    let second = Arc::new(Store::default());
    let mut other = reopen(&second, 7, Vec::new())?;
    other.admit(command(7, 2, CommandKind::Submit, 1)?)?;
    runtime.tick()?;
    other.tick()?;
    assert_eq!(
        runtime.projection().run_tenant(RunId::from_bytes(raw(7))),
        Some(TenantId::from_bytes(raw(1))),
        "first owner preserved"
    );
    assert_eq!(
        other.projection().run_tenant(RunId::from_bytes(raw(7))),
        Some(TenantId::from_bytes(raw(2))),
        "independent authenticated journal scope"
    );
    assert!(
        matches!(
            runtime.correction(TenantId::from_bytes(raw(2)), RunId::from_bytes(raw(7))),
            Err(RuntimeError::RunTenantMismatch { .. })
        ),
        "foreign read refused"
    );
    Ok(())
}

/// A committed-but-unacknowledged append forces recovery, never a blind retry.
#[test]
fn sim_e28_storage_failure_requires_recovery() -> TestResult {
    let store = Arc::new(Store {
        entries: Mutex::new(Vec::new()),
        lose_ack: true,
    });
    let mut runtime = reopen(&store, 1, Vec::new())?;
    let work = command(1, 1, CommandKind::Submit, 1)?;
    assert!(
        matches!(
            runtime.admit(work.clone()),
            Err(RuntimeError::JournalStorage(_))
        ),
        "storage error remains typed"
    );
    assert!(
        matches!(
            runtime.admit(work.clone()),
            Err(RuntimeError::JournalRecoveryRequired)
        ),
        "uncertain writer is fenced"
    );
    assert_eq!(
        runtime.journal().len(),
        0,
        "unconfirmed append is not acknowledged"
    );
    let mut restored = reopen(&store, 1, Vec::new())?;
    assert_eq!(
        restored.admit(work)?,
        CommandAcceptance::DuplicateReplay,
        "committed identity recovered despite lost ack"
    );
    Ok(())
}

/// Saturation refuses before acceptance and capacity recovers after draining.
#[test]
fn sim_e28_queue_saturation_and_recovery() -> TestResult {
    let store = Arc::new(Store::default());
    let mut runtime = reopen(&store, 1, Vec::new())?;
    for seed in 1..=256 {
        runtime.admit(command(seed, 1, CommandKind::Steer, 0)?)?;
    }
    let overflow = command(257, 1, CommandKind::Steer, 0)?;
    assert!(
        matches!(
            runtime.admit(overflow.clone()),
            Err(RuntimeError::ReadyWorkFull { capacity: 256 })
        ),
        "pending bound is enforced"
    );
    assert_eq!(
        runtime.journal().len(),
        256,
        "overflow was not durably accepted"
    );
    runtime.tick()?;
    assert_eq!(
        runtime.admit(overflow)?,
        CommandAcceptance::Initial,
        "draining restores admission capacity"
    );
    Ok(())
}

/// Volatile fixtures cannot accidentally acknowledge durability.
#[test]
fn sim_e28_missing_owner_and_malformed_input() -> TestResult {
    let mut runtime: TestRuntime = Runtime::new(
        settings(1),
        ScriptedOwner::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    )?;
    assert!(
        matches!(
            runtime.admit(command(1, 1, CommandKind::Submit, 1)?),
            Err(RuntimeError::DurabilityUnavailable)
        ),
        "no durable owner means no durable ack"
    );
    let store = Arc::new(Store::default());
    let mut runtime = reopen(&store, 1, Vec::new())?;
    let malformed = Command::new(
        CommandId::from_bytes(raw(1)),
        TenantId::from_bytes(raw(1)),
        Vec::new(),
        CommandContent::new(CommandKind::Submit, Box::from(b"{".as_slice())),
    )?;
    assert!(
        matches!(runtime.admit(malformed), Err(RuntimeError::Codec(_))),
        "malformed admission is typed"
    );
    assert!(
        runtime.journal().is_empty(),
        "invalid input never entered the journal"
    );
    Ok(())
}

/// Journal capacity and stale writers refuse without inventing durable success.
#[test]
fn sim_e28_bounded_recovery_and_stale_writer() -> TestResult {
    let store = Arc::new(Store::default());
    let mut first = reopen(&store, 4, Vec::new())?;
    let mut stale = reopen(&store, 4, Vec::new())?;
    first.admit(command(4, 1, CommandKind::Submit, 1)?)?;
    assert!(
        matches!(
            stale.admit(command(5, 1, CommandKind::Submit, 1)?),
            Err(RuntimeError::JournalStorage(_))
        ),
        "stale offset cannot overwrite accepted history"
    );
    let concrete = Arc::clone(&store);
    let storage: Arc<dyn JournalStorage> = concrete;
    let mut journal = Journal::with_storage(1, Vec::new(), storage)?;
    // A restored prefix cannot exceed its configured capacity.
    let entries = [
        JournalEntry::Command(command(1, 1, CommandKind::Submit, 1)?),
        JournalEntry::Command(command(2, 1, CommandKind::Submit, 1)?),
    ];
    assert!(
        matches!(
            Journal::with_storage(1, entries, Arc::new(Store::default())),
            Err(RuntimeError::JournalFull { capacity: 1 })
        ),
        "recovery cannot silently truncate overflow"
    );
    assert!(
        matches!(
            journal.accept_command(command(6, 1, CommandKind::Submit, 1)?),
            Err(RuntimeError::JournalStorage(_))
        ),
        "public journal also enforces storage offset"
    );
    Ok(())
}

/// Terminal uncertainty still cannot certify that a committed tool has stopped.
#[test]
fn sim_e28_permanent_unknown_is_not_a_safe_boundary() -> TestResult {
    let store = Arc::new(Store::default());
    let mut runtime = reopen(&store, 23, Vec::new())?;
    let original = command(23, 1, CommandKind::Submit, 1)?;
    let mut body: WorkProposal = lgwks_std::json::from_slice(original.content().payload())?;
    body.strategy = ReconciliationStrategy::RequiresHuman;
    let work = Command::new(
        original.id(),
        original.tenant(),
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            lgwks_std::json::to_vec(&body)?.into_boxed_slice(),
        ),
    )?;
    runtime.admit(work)?;
    runtime.tick()?;
    let correction = command(23, 1, CommandKind::Steer, 0)?;
    runtime.admit(correction.clone())?;
    assert_eq!(
        runtime.tick()?.deferred(),
        &[correction.id()],
        "permanent uncertainty cannot promote a correction"
    );
    assert_eq!(
        runtime.correction(original.tenant(), body.run)?,
        None,
        "no claim that correction applied during uncertain tool"
    );
    Ok(())
}

/// The byte ceiling is enforced on durable admission, volatile queues and replay.
#[test]
fn sim_e28_pending_byte_backpressure() -> TestResult {
    let store = Arc::new(Store::default());
    let mut runtime = reopen(&store, 1, Vec::new())?;
    let oversized = Command::new(
        CommandId::from_bytes(raw(1)),
        TenantId::from_bytes(raw(1)),
        Vec::new(),
        CommandContent::new(
            CommandKind::Steer,
            vec![b' '; rocco_runtime::MAX_READY_BYTES.saturating_add(1)].into_boxed_slice(),
        ),
    )?;
    assert!(
        matches!(
            runtime.admit(oversized.clone()),
            Err(RuntimeError::ReadyBytesFull {
                capacity: rocco_runtime::MAX_READY_BYTES
            })
        ),
        "durable admission refuses bytes before decoding or writing"
    );
    assert!(
        matches!(
            runtime.submit(oversized.clone()),
            Err(RuntimeError::ReadyBytesFull { .. })
        ),
        "volatile queue has the same byte ceiling"
    );
    assert!(
        runtime.journal().is_empty(),
        "oversized payload was not accepted"
    );
    let journal = Journal::with_storage(
        4096,
        [JournalEntry::Command(oversized)],
        Arc::new(Store::default()),
    )?;
    let recovered: Result<TestRuntime, _> = restore(
        settings(1),
        ScriptedOwner::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
        journal,
    );
    assert!(
        matches!(recovered, Err(RuntimeError::ReadyBytesFull { .. })),
        "recovery does not allocate an oversized pending queue"
    );
    runtime.admit(command(2, 1, CommandKind::Steer, 0)?)?;
    assert_eq!(
        runtime.tick()?.promoted().len(),
        1,
        "a bounded corrected input still works"
    );
    Ok(())
}
