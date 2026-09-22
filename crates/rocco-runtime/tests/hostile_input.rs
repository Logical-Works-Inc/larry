//! E63 hostile-input falsifiers at Rocco's public command boundary.
//!
//! These cases exercise untrusted bytes and untrusted declarations through
//! `Runtime::submit` and `Runtime::tick`. They do not model a canonical owner:
//! the scripted owner exists solely to prove that refused input caused zero
//! dispatches, reconciliations, outcomes, or terminal receipts.

mod support;

use rocco_contracts::authority::Scope;
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::replay::DenialReason;
use rocco_contracts::state::RunState;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::ProjectionBackend;

use support::{
    assert_state, command_id, operation, proposal, run, runtime_with, submit, succeeded, tenant,
};

/// Places arbitrary caller bytes in a valid submit envelope.
///
/// # Errors
///
/// Returns a typed contract error if the envelope is structurally invalid.
fn hostile_submit(
    id: u8,
    envelope_tenant: u8,
    payload: &[u8],
) -> Result<Command, Box<dyn std::error::Error>> {
    Ok(Command::new(
        command_id(id),
        tenant(envelope_tenant),
        Vec::new(),
        CommandContent::new(CommandKind::Submit, Box::from(payload)),
    )?)
}

/// Malformed input cannot be decoded into work and never reaches an owner.
#[test]
fn e63_malformed_work_payload_is_refused_before_authorization_or_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    runtime.submit(hostile_submit(1, 1, br#"{"run":"not-a-complete-payload"#)?)?;

    let result = runtime.tick();

    assert!(
        matches!(result, Err(RuntimeError::Codec(_))),
        "a malformed proposal must surface the typed codec refusal"
    );
    assert!(runtime.projection().intents().is_empty());
    assert!(runtime.projection().receipts().is_empty());
    assert!(runtime.owner().dispatched().is_empty());
    assert!(runtime.owner().reconciled().is_empty());
    Ok(())
}

/// A forged privilege declaration is denied by the pure gate and cannot
/// consume the fixture owner's effect boundary.
#[test]
fn e63_forged_scope_is_denied_with_zero_owner_effects() -> Result<(), Box<dyn std::error::Error>> {
    let body = proposal(
        run(2),
        tenant(2),
        operation(2),
        &[Scope::FileWrite],
        5,
        ReconciliationStrategy::Queryable,
    );
    let mut runtime = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(2))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    runtime.submit(submit(command_id(2), &body)?)?;

    let tick = runtime.tick()?;

    assert_eq!(tick.authorized(), &[]);
    assert_eq!(
        tick.denied(),
        &[(
            operation(2),
            DenialReason::PolicyRefuses {
                scope: Scope::FileWrite
            }
        )]
    );
    assert!(tick.outcomes().is_empty());
    assert!(tick.receipts().is_empty());
    assert!(runtime.projection().intents().is_empty());
    assert!(runtime.projection().receipts().is_empty());
    assert!(runtime.owner().dispatched().is_empty());
    assert!(runtime.owner().reconciled().is_empty());
    assert_state(runtime.projection(), run(2), RunState::Failed)?;
    Ok(())
}

/// A forged cost cannot reserve more authority than the runtime's budget
/// snapshot grants, even when the scope declaration is otherwise allowed.
#[test]
fn e63_forged_budget_is_denied_with_zero_owner_effects() -> Result<(), Box<dyn std::error::Error>> {
    let body = proposal(
        run(3),
        tenant(3),
        operation(3),
        &[Scope::FileRead],
        101,
        ReconciliationStrategy::Queryable,
    );
    let mut runtime = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(3))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    runtime.submit(submit(command_id(3), &body)?)?;

    let tick = runtime.tick()?;

    assert_eq!(tick.authorized(), &[]);
    assert_eq!(
        tick.denied(),
        &[(
            operation(3),
            DenialReason::BudgetExhausted {
                required: 101,
                remaining: 100
            }
        )]
    );
    assert!(tick.outcomes().is_empty());
    assert!(tick.receipts().is_empty());
    assert!(runtime.projection().intents().is_empty());
    assert!(runtime.projection().receipts().is_empty());
    assert!(runtime.owner().dispatched().is_empty());
    assert!(runtime.owner().reconciled().is_empty());
    assert_state(runtime.projection(), run(3), RunState::Failed)?;
    Ok(())
}
