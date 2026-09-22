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
    hostile_command(id, envelope_tenant, CommandKind::Submit, payload)
}

/// Places arbitrary caller bytes in a valid command envelope.
///
/// # Errors
///
/// Returns a typed contract error if the envelope is structurally invalid.
fn hostile_command(
    id: u8,
    envelope_tenant: u8,
    kind: CommandKind,
    payload: &[u8],
) -> Result<Command, Box<dyn std::error::Error>> {
    Ok(Command::new(
        command_id(id),
        tenant(envelope_tenant),
        Vec::new(),
        CommandContent::new(kind, Box::from(payload)),
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
    assert!(runtime.journal().is_empty());
    assert!(runtime.owner().dispatched().is_empty());
    assert!(runtime.owner().reconciled().is_empty());

    let corrected = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(1), &corrected)?)?;
    let retried = runtime.tick()?;
    assert_eq!(retried.admitted(), &[command_id(1)]);
    Ok(())
}

/// An established run's tenant is authoritative over later command envelopes.
#[test]
fn e63_foreign_cancel_is_refused_before_journal_or_state_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let body = proposal(
        run(4),
        tenant(1),
        operation(4),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut runtime = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    runtime.submit(submit(command_id(4), &body)?)?;
    runtime.tick()?;
    let before_journal = runtime.journal().len();
    let before_state = runtime.projection().run_state(run(4));
    let before_dispatches = runtime.owner().dispatched().len();
    let payload = lgwks_std::json::to_vec(&body)?;
    runtime.submit(hostile_command(5, 2, CommandKind::Cancel, &payload)?)?;

    let result = runtime.tick();

    assert!(matches!(
        result,
        Err(RuntimeError::RunTenantMismatch {
            run: rejected,
            expected,
            got,
        }) if rejected == run(4) && expected == tenant(1) && got == tenant(2)
    ));
    assert_eq!(runtime.journal().len(), before_journal);
    assert_eq!(runtime.projection().run_state(run(4)), before_state);
    assert_eq!(runtime.owner().dispatched().len(), before_dispatches);
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
