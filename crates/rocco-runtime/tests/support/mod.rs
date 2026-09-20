//! Shared fixtures for the runtime's public-interface falsifiers.
//!
//! These build *real* `rocco_contracts` values and drive the runtime through
//! its public surface only. They are not owner evidence: nothing here reaches
//! Moo, Sentinel, Nova or a network, and the owner is always the scripted
//! fixture double in `rocco_runtime::owner::fixture`.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use rocco_contracts::authority::{BudgetUnit, Scope};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use rocco_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use rocco_contracts::state::RunState;
use rocco_runtime::clock::FixedClock;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::{FoldProjection, ProjectionBackend};
use rocco_runtime::runtime::{Runtime, RuntimeSettings};

/// The alias the tests use for the concrete runtime under test.
pub type TestRuntime = Runtime<FoldProjection, ScriptedOwner, FixedClock>;

/// The pinned instant every test starts from.
pub fn epoch() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
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
pub fn validity() -> ValiditySnapshot {
    ValiditySnapshot {
        valid_until: epoch() + Duration::from_secs(3_600),
    }
}

/// Settings allowing `allowed` under a policy that permits it, with `capable`
/// as the worker's capabilities and `budget` requests reserved.
pub fn settings(allowed: &[Scope], capable: &[Scope], budget: u64) -> RuntimeSettings {
    RuntimeSettings::with_defaults(
        PolicySnapshot {
            revision: 1,
            allowed_scopes: allowed.iter().copied().collect(),
        },
        CapabilitySnapshot {
            capabilities: capable.iter().copied().collect(),
        },
        validity(),
        BudgetUnit::Requests,
        budget,
    )
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
) -> Result<TestRuntime, RuntimeError> {
    Runtime::new(
        settings(allowed, capable, budget),
        owner,
        FixedClock::new(epoch()),
    )
}

/// A runtime whose owner has nothing scripted and therefore reports unknown
/// forever, with `allowed`/`capable` permitting `FileRead` and a budget of 100.
///
/// # Errors
///
/// As [`runtime_with`].
pub fn runtime() -> Result<TestRuntime, RuntimeError> {
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
    WorkProposal {
        run,
        tenant,
        operation,
        required_scopes: scopes.iter().copied().collect::<BTreeSet<_>>(),
        cost,
        unit: BudgetUnit::Requests,
        strategy,
    }
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
