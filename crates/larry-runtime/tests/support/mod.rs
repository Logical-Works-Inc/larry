//! Shared fixtures for the runtime's public-interface falsifiers.
//!
//! These build *real* `larry_contracts` values and drive the runtime through
//! its public surface only. They are not owner evidence: nothing here reaches
//! Moo, Sentinel, Nova or a network, and the owner is always the scripted
//! fixture double in `larry_runtime::owner::fixture`.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_contracts::state::RunState;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{Runtime, RuntimeSettings};
use lgwks_bot::clock::Clock;

/// The alias the tests use for the concrete runtime under test.
pub type TestRuntime = Runtime<FoldProjection, ScriptedOwner>;

thread_local! {
    /// The seed every fixture identity and the epoch are drawn under. Zero is
    /// the unseeded world the non-simulation falsifiers run in.
    static SEED: Cell<u64> = const { Cell::new(0) };
}

/// Puts the previous seed back when a seeded scope ends, early returns included.
struct Restore(u64);

impl Drop for Restore {
    fn drop(&mut self) {
        SEED.with(|cell| cell.set(self.0));
    }
}

/// Runs `body` with every identity fixture and [`epoch`] drawn from `seed`.
///
/// The scope is synchronous on the caller's thread, so no other test and no
/// other thread observes the seed.
pub fn seeded<T>(seed: u64, body: impl FnOnce() -> T) -> T {
    let _restore = Restore(SEED.with(|cell| cell.replace(seed)));
    body()
}

/// Sixteen identity bytes for `byte` under the current seed: `byte` leads, so
/// identities keep their relative order under every seed, and the seed is
/// folded into the tail, so each seed draws its own identity universe. Seed
/// zero yields `[byte; 16]`.
fn id_bytes(byte: u8) -> [u8; 16] {
    let mut bytes = [byte; 16];
    let salt = SEED.with(Cell::get).to_be_bytes();
    for (slot, mix) in bytes.iter_mut().skip(8).zip(salt) {
        *slot ^= mix;
    }
    bytes
}

/// The pinned instant every test starts from, shifted by the seed in seconds.
pub fn epoch() -> Result<SystemTime, Box<dyn std::error::Error>> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .and_then(|base| base.checked_add(Duration::from_secs(SEED.with(Cell::get))))
        .ok_or_else(|| "fixture epoch is outside the platform time range".into())
}

/// The estate's virtual clock placed at [`epoch`]: the runtime reads it as elapsed time since
/// the Unix epoch, and nothing moves it but an explicit `advance` (larry#74).
pub fn clock() -> Result<Clock, Box<dyn std::error::Error>> {
    Ok(Clock::virtual_at(
        epoch()?.duration_since(SystemTime::UNIX_EPOCH)?,
    ))
}

/// A run identifier from one byte.
pub fn run(byte: u8) -> RunId {
    RunId::from_bytes(id_bytes(byte))
}

/// A tenant identifier from one byte.
pub fn tenant(byte: u8) -> TenantId {
    TenantId::from_bytes(id_bytes(byte))
}

/// An operation identifier from one byte.
pub fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes(id_bytes(byte))
}

/// A command identifier from one byte.
pub fn command_id(byte: u8) -> CommandId {
    CommandId::from_bytes(id_bytes(byte))
}

/// The pinned validity window the tests use: valid for an hour from [`epoch`].
pub fn validity() -> Result<ValiditySnapshot, Box<dyn std::error::Error>> {
    Ok(ValiditySnapshot::new(
        epoch()?
            .checked_add(Duration::from_secs(3_600))
            .ok_or("fixture validity overflow")?,
    ))
}

/// Settings allowing `allowed` under a policy that permits it, with `capable`
/// as the worker's capabilities and `budget` requests reserved.
pub fn settings(
    allowed: &[Scope],
    capable: &[Scope],
    budget: u64,
) -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    Ok(RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, allowed.iter().copied().collect()),
        CapabilitySnapshot::new(capable.iter().copied().collect()),
        validity()?,
        BudgetUnit::Requests,
        budget,
    ))
}

/// A runtime over the scripted owner `owner`.
///
/// # Errors
///
/// Cannot fail for the settings this helper builds, but the constructor is
/// fallible and the error is propagated rather than discarded.
pub fn runtime_with(
    owner: ScriptedOwner,
    allowed: &[Scope],
    capable: &[Scope],
    budget: u64,
) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    Ok(Runtime::new(
        settings(allowed, capable, budget)?,
        owner,
        clock()?,
    )?)
}

/// A runtime whose owner has nothing scripted and therefore reports unknown
/// forever, with `allowed`/`capable` permitting `FileRead` and a budget of 100.
///
/// # Errors
///
/// As [`runtime_with`].
pub fn runtime() -> Result<TestRuntime, Box<dyn std::error::Error>> {
    runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )
}

/// The proposal body for one unit of work.
pub fn proposal(
    run: RunId,
    tenant: TenantId,
    operation: OperationId,
    scopes: &[Scope],
    cost: u64,
    strategy: ReconciliationStrategy,
) -> WorkProposal {
    WorkProposal::new(
        run,
        tenant,
        operation,
        scopes.iter().copied().collect::<BTreeSet<_>>(),
        cost,
        BudgetUnit::Requests,
        strategy,
    )
}

/// A `Submit` command carrying `body`.
///
/// # Errors
///
/// Returns the contract's ordering error for an over-long causal-parent list,
/// which cannot happen here but is not silently discarded.
pub fn submit(id: CommandId, body: &WorkProposal) -> Result<Command, Box<dyn std::error::Error>> {
    let payload = lgwks_std::json::to_vec(body)?;
    Ok(Command::new(
        id,
        body.tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Submit, payload.into_boxed_slice()),
    )?)
}

/// A `Cancel` command carrying `body`.
///
/// # Errors
///
/// As [`submit`].
pub fn cancel(id: CommandId, body: &WorkProposal) -> Result<Command, Box<dyn std::error::Error>> {
    let payload = lgwks_std::json::to_vec(body)?;
    Ok(Command::new(
        id,
        body.tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Cancel, payload.into_boxed_slice()),
    )?)
}

/// A success outcome for `operation`.
pub fn succeeded(operation: OperationId) -> EffectOutcome {
    EffectOutcome::Succeeded { operation }
}

/// An ambiguous outcome for `operation`.
pub fn unknown(operation: OperationId) -> EffectOutcome {
    EffectOutcome::Unknown { operation }
}

/// Asserts the run reached exactly the expected state.
///
/// # Errors
///
/// Returns an error when the run is untracked or the state differs, naming
/// both values.
pub fn assert_state(
    projection: &impl ProjectionBackend,
    run: RunId,
    expected: RunState,
) -> Result<(), Box<dyn std::error::Error>> {
    let actual = projection
        .run_state(run)
        .ok_or_else(|| format!("run {run} is not tracked"))?;
    if actual != expected {
        return Err(format!("run {run}: expected {expected}, got {actual}").into());
    }
    Ok(())
}

/// Shared fixtures preserve command identity, cancellation and unknown outcomes.
#[test]
fn fixture_contracts() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    assert_eq!(unknown(body.operation).operation(), body.operation);
    assert_eq!(succeeded(body.operation).operation(), body.operation);
    let command = cancel(command_id(1), &body)?;
    assert_eq!(command.id(), command_id(1));
    assert_eq!(command.content().kind(), CommandKind::Cancel);
    runtime.submit(command)?;
    let tick = runtime.tick()?;
    assert_eq!(tick.admitted(), &[command_id(1)]);
    assert_state(runtime.projection(), run(1), RunState::Cancelled)?;
    assert!(
        assert_state(runtime.projection(), run(2), RunState::Cancelled).is_err(),
        "an untracked run is an error, never a match"
    );
    assert_eq!(tick.receipts(), &[]);
    assert_eq!(run(7), RunId::from_bytes([7; 16]), "seed zero is unseeded");
    let (seeded_run, seeded_epoch) = seeded(3, || {
        Ok::<_, Box<dyn std::error::Error>>((run(7), epoch()?))
    })?;
    assert_ne!(seeded_run, run(7), "a seed draws its own identities");
    assert_ne!(seeded_epoch, epoch()?, "a seed shifts the epoch");
    assert_eq!(
        run(7),
        RunId::from_bytes([7; 16]),
        "the seed is restored after its scope"
    );
    Ok(())
}
