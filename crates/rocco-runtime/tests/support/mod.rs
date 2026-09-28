//! Shared fixtures for the runtime's public-interface falsifiers.
//!
//! These build *real* `rocco_contracts` values and drive the runtime through
//! its public surface only. They are not owner evidence: nothing here reaches
//! Moo, Sentinel, Nova or a network, and the owner is always the scripted
//! fixture double in `rocco_runtime::owner::fixture`.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use rocco_contracts::authority::{BudgetUnit, Scope};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use rocco_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use rocco_contracts::state::RunState;
use rocco_runtime::clock::FixedClock;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::{FoldProjection, ProjectionBackend};
use rocco_runtime::runtime::{Runtime, RuntimeSettings};

/// The alias the tests use for the concrete runtime under test.
pub type TestRuntime = Runtime<FoldProjection, ScriptedOwner, FixedClock>;

/// The pinned instant every test starts from.
pub fn epoch() -> Result<SystemTime, Box<dyn std::error::Error>> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .ok_or_else(|| "fixture epoch is outside the platform time range".into())
}

/// A run identifier from one byte.
pub fn run(byte: u8) -> RunId {
    RunId::from_bytes([byte; 16])
}

/// A tenant identifier from one byte.
pub fn tenant(byte: u8) -> TenantId {
    TenantId::from_bytes([byte; 16])
}

/// An operation identifier from one byte.
pub fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16])
}

/// A command identifier from one byte.
pub fn command_id(byte: u8) -> CommandId {
    CommandId::from_bytes([byte; 16])
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
    fixture_runtime(
        settings(allowed, capable, budget)?,
        owner,
        FixedClock::new(epoch()?),
    )
}

/// Test-owned durable journal owner: compare-and-append into memory. Durable
/// admission (E28) refuses a runtime without one.
#[derive(Debug, Default)]
pub struct MemoryStore {
    /// The committed prefix.
    entries: std::sync::Mutex<Vec<rocco_runtime::journal::JournalEntry>>,
}

impl rocco_runtime::JournalStorage for MemoryStore {
    fn append(
        &self,
        offset: usize,
        entry: &rocco_runtime::journal::JournalEntry,
    ) -> std::io::Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| std::io::Error::other("poisoned test store"))?;
        if offset != entries.len() {
            return Err(std::io::Error::other("stale offset refused"));
        }
        entries.push(entry.clone());
        Ok(())
    }
}

/// [`runtime_with`] over a durable journal owner, for callers of admission.
pub fn durable_runtime_with(
    owner: ScriptedOwner,
    allowed: &[Scope],
    capable: &[Scope],
    budget: u64,
) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    let storage: std::sync::Arc<dyn rocco_runtime::JournalStorage> =
        std::sync::Arc::new(MemoryStore::default());
    fixture_restore(
        settings(allowed, capable, budget)?,
        owner,
        FixedClock::new(epoch()?),
        rocco_runtime::journal::Journal::with_storage(4_096, Vec::new(), storage)?,
    )
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
    let command = cancel(command_id(1), &body)?;
    assert_eq!(command.id(), command_id(1));
    assert_eq!(command.content().kind(), CommandKind::Cancel);
    runtime.submit(command)?;
    let tick = runtime.tick()?;
    assert_eq!(tick.admitted(), &[command_id(1)]);
    assert_eq!(
        runtime.projection().run_state(run(1)),
        Some(RunState::Cancelled)
    );
    assert_eq!(tick.receipts(), &[]);
    let restored: TestRuntime = fixture_restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        FixedClock::new(epoch()?),
        runtime.into_journal(),
    )?;
    assert_eq!(
        restored.projection().run_state(run(1)),
        Some(RunState::Cancelled)
    );
    // The durable fixture journals an admitted command before any tick runs.
    let mut durable = durable_runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    durable.admit(submit(command_id(2), &body)?)?;
    assert_eq!(durable.journal().len(), 1);
    assert_eq!(durable.tick_count(), 0);
    Ok(())
}

/// Explicitly provisions the finite synthetic tenant family used by these tests.
/// Production constructors never derive authority from runtime settings.
pub fn fixture_authority(
    settings: &RuntimeSettings,
) -> Result<rocco_runtime::authority::fixture::FixtureAuthority, Box<dyn std::error::Error>> {
    let mut source = rocco_runtime::authority::fixture::FixtureAuthority::new(
        settings.security_epoch,
        settings.offline_dispatch_bound,
    );
    for byte in u8::MIN..=u8::MAX {
        source.trust(
            tenant(byte),
            settings.policy.clone(),
            settings.capabilities.clone(),
            settings.validity,
        )?;
    }
    Ok(source)
}

/// Constructs a runtime with explicitly synthetic source evidence.
pub fn fixture_runtime<P, O, C>(
    settings: RuntimeSettings,
    owner: O,
    clock: C,
) -> Result<Runtime<P, O, C>, Box<dyn std::error::Error>>
where
    P: ProjectionBackend + Default,
    O: rocco_runtime::owner::EffectOwner,
    C: rocco_runtime::clock::Clock,
{
    let authority = fixture_authority(&settings)?;
    Ok(Runtime::new(settings, owner, clock, authority)?)
}

/// Reopens a journal with an explicit independently supplied fixture source.
pub fn fixture_restore<P, O, C>(
    settings: RuntimeSettings,
    owner: O,
    clock: C,
    journal: rocco_runtime::journal::Journal,
) -> Result<Runtime<P, O, C>, Box<dyn std::error::Error>>
where
    P: ProjectionBackend + Default,
    O: rocco_runtime::owner::EffectOwner,
    C: rocco_runtime::clock::Clock,
{
    let authority = fixture_authority(&settings)?;
    Ok(rocco_runtime::runtime::restore(
        settings, owner, clock, journal, authority,
    )?)
}
