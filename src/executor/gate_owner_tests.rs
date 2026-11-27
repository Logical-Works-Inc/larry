use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use larry_contracts::binding::{BindingError, BindingField};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, ReconciliationStrategy};
use larry_contracts::ids::{CommandId, EffectId, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::EffectOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::FoldProjection;
use larry_runtime::runtime::{Runtime, RuntimeSettings, TickItem};
use lgwks_bot::kernel::cap::{Auth, Cap};
use lgwks_bot::kernel::clock::Clock;
use lgwks_bot::kernel::gate::GrantSet;

use super::{ExecError, TaskOwner};

/// Builds a leaseless proof that covers the first capability but not the
/// second one: no expiry to advance, so the verdict turns on coverage alone.
fn proof_and_required(seed: u8) -> Result<(Auth, [Cap; 2]), Box<dyn std::error::Error>> {
    let granted = Cap::new(format!("larry.granted.{seed}"));
    let missing = Cap::new(format!("larry.missing.{seed}"));
    let proof = GrantSet::empty()
        .grant(granted.clone())
        .issue(std::slice::from_ref(&granted))?;
    Ok((proof, [granted, missing]))
}

/// Builds one intent with a deterministic tenant and operation identity.
fn intent(seed: u8, tenant: TenantId) -> EffectIntent {
    EffectIntent::new(
        EffectId::from_bytes([seed; 16]),
        OperationId::from_bytes([seed; 16]),
        RunId::from_bytes([seed; 16]),
        tenant,
        ReconciliationStrategy::Queryable,
        SecurityEpoch::INITIAL,
    )
}

/// The task body's outcome, shared with the gate through one slot.
type BodyOutcome = Result<super::Report, ExecError>;

/// The slot that stays empty unless the body runs.
type BodySlot = Rc<RefCell<Option<BodyOutcome>>>;

/// A task body that must never run: it marks `ran` and fails, so any reach
/// that executes it is observed, and the failure is the body's own.
fn unrun_body(ran: Rc<Cell<bool>>) -> impl FnOnce() -> BodyOutcome {
    move || {
        ran.set(true);
        Err(ExecError::Gate(RuntimeError::ClockOutOfRange {
            elapsed: Duration::ZERO,
        }))
    }
}

/// A task owner whose body must never run, with the flags that observe it:
/// `ran` is set only if the body executes, `slot` only if a result is recorded.
fn owner_with_unrun_body(
    tenant: TenantId,
    epoch: SecurityEpoch,
) -> (
    TaskOwner<'static, impl FnOnce() -> BodyOutcome>,
    Rc<Cell<bool>>,
    BodySlot,
) {
    let ran = Rc::new(Cell::new(false));
    let slot = Rc::new(RefCell::new(None));
    let owner = TaskOwner {
        repo: Path::new("."),
        tenant,
        epoch,
        task_bytes: b"task",
        run: Some(unrun_body(Rc::clone(&ran))),
        slot: Rc::clone(&slot),
    };
    (owner, ran, slot)
}

/// Asserts a refused reach left no trace: the body never ran and no result
/// was recorded.
fn assert_body_never_ran(ran: &Rc<Cell<bool>>, slot: &BodySlot) {
    assert!(!ran.get(), "a refused reach must not run the body");
    assert!(
        slot.borrow().is_none(),
        "a refused reach must not record a body result"
    );
}

#[test]
fn sim_task_owner_refuses_a_missing_required_capability() -> Result<(), Box<dyn std::error::Error>>
{
    for seed in 0_u8..32 {
        let (proof, required) = proof_and_required(seed)?;
        let tenant = TenantId::from_bytes([seed; 16]);
        let (mut owner, ran, slot) = owner_with_unrun_body(tenant, SecurityEpoch::INITIAL);
        assert_eq!(
            owner.dispatch_bound(
                &intent(seed, tenant),
                &proof,
                &required,
                SystemTime::UNIX_EPOCH,
            ),
            Err(BindingError::NoCoveringGrant),
            "seed {seed}: the task owner must require every effect capability"
        );
        assert_body_never_ran(&ran, &slot);
    }
    Ok(())
}

#[test]
fn an_unbound_task_owner_dispatch_is_typed_and_never_runs_the_body()
-> Result<(), Box<dyn std::error::Error>> {
    let (proof, [granted, _missing]) = proof_and_required(91)?;
    let required = [granted];
    let tenant = TenantId::from_bytes([91; 16]);
    let (mut owner, ran, slot) = owner_with_unrun_body(tenant, SecurityEpoch::INITIAL);
    assert_eq!(
        owner.dispatch(
            &intent(91, tenant),
            &proof,
            &required,
            SystemTime::UNIX_EPOCH,
        ),
        Err(BindingError::UnsupportedOwner)
    );
    assert_body_never_ran(&ran, &slot);
    Ok(())
}

#[test]
fn task_owner_refuses_a_foreign_intent_tenant_before_body() -> Result<(), Box<dyn std::error::Error>>
{
    let (proof, [granted, _missing]) = proof_and_required(92)?;
    let required = [granted];
    let owner_tenant = TenantId::from_bytes([92; 16]);
    let foreign_tenant = TenantId::from_bytes([93; 16]);
    let (mut owner, ran, slot) = owner_with_unrun_body(owner_tenant, SecurityEpoch::INITIAL);
    assert_eq!(
        owner.dispatch_bound(
            &intent(92, foreign_tenant),
            &proof,
            &required,
            SystemTime::UNIX_EPOCH,
        ),
        Err(BindingError::Changed {
            field: larry_contracts::binding::BindingField::Command,
        })
    );
    assert_body_never_ran(&ran, &slot);
    Ok(())
}

/// Drives a foreign-tenant command through a real runtime tick against a
/// `TaskOwner`: display admits it under its own grant, the bound reach refuses
/// it typed, and the body never runs.
///
/// Source: larry#461 finding 3. `cap::Auth` names no tenant or holder
/// (estate gap logicalworks-crates#1161), so Larry checks the intent tenant
/// against the owner tenant. A direct `dispatch_bound` call would also pass on
/// unpatched code paths that never consult the tenant; this sim proves the
/// refusal happens on the runtime's own reach, where the body lives.
#[test]
fn sim_a_foreign_tenant_is_refused_on_the_runtime_reach() -> Result<(), Box<dyn std::error::Error>>
{
    for seed in 0_u8..32 {
        foreign_tenant_refused_once(seed)?;
    }
    Ok(())
}

/// One seed of the runtime-reach tenant sim: display admits a foreign-tenant
/// command under its own grant, the tick's bound reach refuses it typed, and
/// the task body never runs and nothing is recorded.
fn foreign_tenant_refused_once(seed: u8) -> Result<(), Box<dyn std::error::Error>> {
    // Identity bytes are never all zero: the fence refuses a zero run,
    // tenant or operation as uninitialised before the owner is reached.
    let owner_byte = seed.wrapping_add(1);
    let foreign_byte = seed.wrapping_add(129);
    let owner_tenant = TenantId::from_bytes([owner_byte; 16]);
    let foreign_tenant = TenantId::from_bytes([foreign_byte; 16]);
    let epoch = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000_u64 + u64::from(seed)))
        .ok_or("fixture epoch overflow")?;
    let valid_until = epoch
        .checked_add(Duration::from_secs(3_600))
        .ok_or("fixture validity overflow")?;
    let scopes: BTreeSet<Scope> = [Scope::FileRead, Scope::ProcessSpawn]
        .iter()
        .copied()
        .collect();
    let settings = RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, scopes.clone()),
        CapabilitySnapshot::new(scopes.clone()),
        ValiditySnapshot::new(valid_until),
        BudgetUnit::Requests,
        100,
    );
    let owner_epoch = settings.security_epoch;
    let clock = Clock::virtual_at(epoch.duration_since(SystemTime::UNIX_EPOCH)?);
    let (owner, ran, slot) = owner_with_unrun_body(owner_tenant, owner_epoch);
    let mut runtime: Runtime<FoldProjection, _> = Runtime::new(settings, owner, clock)?;
    let operation = OperationId::from_bytes([owner_byte; 16]);
    let proposal = WorkProposal::new(
        RunId::from_bytes([owner_byte; 16]),
        foreign_tenant,
        operation,
        scopes.clone(),
        1,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    );
    let payload = lgwks_std::json::to_vec(&proposal)?;
    let command_id = CommandId::from_bytes([owner_byte; 16]);
    let command = Command::new(
        command_id,
        foreign_tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Submit, payload.into_boxed_slice()),
    )?;
    let holder = WorkerId::from_bytes([7; 16]);
    let grant = Grant::new(
        foreign_tenant,
        holder,
        scopes,
        Budget::new(1, BudgetUnit::Requests),
        valid_until,
        0,
    )?;
    let shown = runtime.display(&command, holder, &[grant])?;
    runtime.submit_displayed(command, shown)?;
    let report = runtime.tick()?;
    assert!(
        report.outcomes().is_empty(),
        "seed {seed}: a foreign tenant records no outcome"
    );
    let refused = report.refused();
    assert_eq!(
        refused.len(),
        1,
        "seed {seed}: a foreign tenant is exactly one typed refusal"
    );
    // The owner's typed refusal propagates through the reduce that reached
    // it, so the report keys it by the command it refused, not the effect.
    assert!(
        matches!(
            refused.first(),
            Some((TickItem::Command(refused_id), RuntimeError::Binding(
                BindingError::Changed {
                    field: BindingField::Command,
                }
            ))) if *refused_id == command_id
        ),
        "seed {seed}: the refusal names the tenant change, got {refused:?}"
    );
    assert_body_never_ran(&ran, &slot);
    Ok(())
}
