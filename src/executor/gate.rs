//! The admission gate (larry#164): a task's body runs only inside the
//! runtime's owner call, after the gate authorized it.
//!
//! Source: OWASP ASVS 4.0.3 chapter V4 (access control) — §4.1.1 rejects a
//! request whose privilege level is insufficient at the server, §4.1.3 denies
//! by default, and §4.2.1 puts the enforcement at the action's boundary rather
//! than the entry point; plus larry#164. The executor is the server here:
//! `LARRY_POLICY` is the operator's policy, each task declares the scopes it
//! needs (repository read and process spawn always; file write and model
//! invoke when it carries a brief), and `larry-runtime` gates the declared
//! work before the body is dispatched exactly once inside the bound owner
//! reach. A denial runs nothing — no clone, no command, no model call — and
//! the sealed report says so.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use larry_contracts::binding::{BindingError, EffectTarget, ResolvedEffect};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::{CapabilitySnapshot, DenialReason, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::EffectOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::FoldProjection;
use larry_runtime::runtime::{Runtime, RuntimeSettings};
use lgwks_bot::clock::Clock;

use super::{ExecError, Report, Settings, Task};

/// How long admission authority runs: admission and dispatch happen inside one
/// call, so the lease only has to outlive the tick.
const GATE_LEASE: Duration = Duration::from_secs(60);

/// The policy revision this build pins; a decision is only comparable inside
/// one revision.
const POLICY_REVISION: u64 = 1;

/// The request each task costs against its one-run budget.
const TASK_COST: u64 = 1;

/// The scopes the executor is capable of. The capability snapshot pins them,
/// so the policy alone decides what a task may do.
const EXECUTOR_CAPABILITIES: [Scope; 4] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
];

/// The policy when `LARRY_POLICY` is unset: the four scopes the executor uses.
fn default_policy() -> BTreeSet<Scope> {
    EXECUTOR_CAPABILITIES.iter().copied().collect()
}

/// Domain tags separating the identities one state directory and one task id
/// derive, so no two derivations collide and none collides with another
/// repository's.
const TENANT_TAG: &[u8] = b"larry/executor/tenant/v1\0";
/// The run's domain tag.
const RUN_TAG: &[u8] = b"larry/executor/run/v1\0";
/// The operation's domain tag.
const OPERATION_TAG: &[u8] = b"larry/executor/operation/v1\0";
/// The command's domain tag.
const COMMAND_TAG: &[u8] = b"larry/executor/command/v1\0";
/// The worker's domain tag.
const WORKER_TAG: &[u8] = b"larry/executor/worker/v1\0";

/// The prefix the estate capability names carry and a policy name omits.
const CAPABILITY_PREFIX: &str = "larry.";

/// The dotted name a scope carries in `LARRY_POLICY` and in a denial line: the
/// estate capability name `replay::capability` gives it (larry#77), without
/// its `larry.` prefix, so the scope-to-name mapping has one owner.
fn scope_name(scope: Scope) -> String {
    let capability = larry_contracts::replay::capability(scope);
    let name = capability.as_str();
    match name.strip_prefix(CAPABILITY_PREFIX) {
        Some(short) => short.to_owned(),
        None => name.to_owned(),
    }
}

/// The scope a policy name admits, or `None` for a name that is not one: the
/// executor's own scopes plus network egress, which a policy may name though
/// no executor task needs it.
fn named_scope(name: &str) -> Option<Scope> {
    EXECUTOR_CAPABILITIES
        .iter()
        .copied()
        .chain(core::iter::once(Scope::NetworkEgress))
        .find(|scope| scope_name(*scope) == name)
}

/// The `LARRY_POLICY` scopes `read` yields: unset is the four scopes the
/// executor uses, the empty string is no scope, names are trimmed and
/// comma-separated, and an unknown name is a typed refusal.
pub(super) fn policy(read: &impl Fn(&str) -> Option<String>) -> Result<BTreeSet<Scope>, ExecError> {
    let Some(text) = read("LARRY_POLICY") else {
        return Ok(default_policy());
    };
    let mut scopes = BTreeSet::new();
    for name in text.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        match named_scope(name) {
            Some(scope) => {
                scopes.insert(scope);
            }
            None => {
                return Err(ExecError::BadPolicy {
                    value: name.to_owned(),
                });
            }
        }
    }
    Ok(scopes)
}

/// The scopes one task requires: repository read and process spawn always,
/// and file write plus model invoke when the task carries a brief the model
/// writes.
fn required_scopes(task: &Task) -> BTreeSet<Scope> {
    let mut scopes = BTreeSet::new();
    scopes.insert(Scope::FileRead);
    scopes.insert(Scope::ProcessSpawn);
    if task.brief().is_some() {
        scopes.insert(Scope::FileWrite);
        scopes.insert(Scope::ModelInvoke);
    }
    scopes
}

/// The first 16 bytes of the BLAKE3 digest over `parts`: the bytes a stable,
/// reproducible identity is built from (as `digest_hex` hashes).
fn identity16(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = lgwks_std::hash::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    match hasher.finalize().as_bytes().first_chunk::<16>() {
        Some(chunk) => *chunk,
        None => [0_u8; 16],
    }
}

/// The first required scope, in `Scope` order, the policy does not admit.
fn first_missing(required: &BTreeSet<Scope>, policy: &BTreeSet<Scope>) -> Option<Scope> {
    required
        .iter()
        .copied()
        .find(|scope| !policy.contains(scope))
}

/// The one denial line a recorded authorization reason stands for, naming the
/// refused scope with its dotted name.
fn denial_line(reason: &DenialReason) -> String {
    match *reason {
        DenialReason::PolicyRefuses { scope } => {
            format!("gate: policy_refuses: {}", scope_name(scope))
        }
        DenialReason::CapabilityMissing { scope } => {
            format!("gate: capability_missing: {}", scope_name(scope))
        }
        DenialReason::AuthorityExpired => String::from("gate: authority_expired"),
        DenialReason::BudgetExhausted { .. } => String::from("gate: budget_exhausted"),
        DenialReason::UnitMismatch { .. } => String::from("gate: unit_mismatch"),
        _ => String::from("gate: denied"),
    }
}

/// The sealed report a gate denial yields: nothing ran, and the report says
/// what the gate refused, in the model-commands denial's exact shape.
fn denied_report(
    task_id: &str,
    task_digest: &str,
    reason: &str,
    key: &lgwks_std::sign::SecretKey,
) -> Report {
    Report::sealed(
        lgwks_std::json::json!({
            "task_id": task_id,
            "task_digest": task_digest,
            "diff": null,
            "results": [],
            "denied": [reason],
        }),
        key,
    )
}

/// `now + GATE_LEASE` as an instant, or the typed refusal when the platform
/// cannot hold the reading.
fn lease_end(now: SystemTime) -> Result<SystemTime, RuntimeError> {
    let elapsed = match now.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(elapsed) => elapsed,
        Err(_) => Duration::ZERO,
    };
    let total = elapsed.saturating_add(GATE_LEASE);
    SystemTime::UNIX_EPOCH
        .checked_add(total)
        .ok_or(RuntimeError::ClockOutOfRange { elapsed: total })
}

/// The owner boundary the executor presents: the task body is the one external
/// effect. `run` is the gated body, held until the bound reach runs it at most
/// once; its outcome is the effect's outcome. The executor cannot be queried
/// after an unbound dispatch, so both unbound dispatch and reconciliation
/// report unknown — a fact the proposal's `RequiresHuman` strategy records as
/// such rather than a success it never observed.
struct TaskOwner<'a, F>
where
    F: FnOnce() -> Result<Report, ExecError>,
{
    /// The repository path the task binds to.
    repo: &'a std::path::Path,
    /// The tenant the run belongs to; one state directory is one tenant.
    tenant: TenantId,
    /// The task document bytes the effect's payload binds to.
    task_bytes: &'a [u8],
    /// The gated body, held until the bound reach runs it once.
    run: Option<F>,
    /// The body's outcome, shared with the gate through one slot.
    slot: Rc<RefCell<Option<Result<Report, ExecError>>>>,
}

impl<F> EffectOwner for TaskOwner<'_, F>
where
    F: FnOnce() -> Result<Report, ExecError>,
{
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        let raw = self.repo.as_os_str().as_encoded_bytes();
        // The resolved link identity is the canonical repository path. A path
        // that cannot be canonicalized binds to its raw bytes instead, which
        // resolves identically on every call for a path that does not move.
        let canonical = match std::fs::canonicalize(self.repo) {
            Ok(path) => path.as_os_str().as_encoded_bytes().to_vec(),
            Err(_) => raw.to_vec(),
        };
        let target = EffectTarget::new(
            raw,
            &canonical,
            b"",
            self.tenant.as_bytes(),
            self.task_bytes,
            b"larry.executor/v1",
        )?;
        // The schema version of the executor's own adapter contract, and the
        // epoch the runtime's fence starts at under the default settings.
        Ok(Some(ResolvedEffect::new(target, 1, SecurityEpoch::INITIAL)))
    }

    fn dispatch_bound(
        &mut self,
        intent: &EffectIntent,
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        if self.slot.borrow().is_some() {
            // The body ran once; a second reach never re-runs it.
            return Ok(EffectOutcome::Unknown {
                operation: intent.operation,
            });
        }
        let Some(run) = self.run.take() else {
            return Ok(EffectOutcome::Unknown {
                operation: intent.operation,
            });
        };
        let finished = run();
        let outcome = match finished.is_ok() {
            true => EffectOutcome::Succeeded {
                operation: intent.operation,
            },
            false => EffectOutcome::Failed {
                operation: intent.operation,
            },
        };
        *self.slot.borrow_mut() = Some(finished);
        Ok(outcome)
    }

    fn dispatch(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }
}

/// Admits `task` to the runtime and runs its body only inside the authorized
/// owner call. A denied task returns a sealed report naming the refusal and
/// runs nothing; a runtime failure is a typed error; an admitted task's report
/// (or its own typed failure) is the body's outcome.
pub(super) fn admit_and_run(
    task: &Task,
    settings: &Settings,
    task_id: &str,
    task_digest: &str,
    key: &lgwks_std::sign::SecretKey,
    run: impl FnOnce() -> Result<Report, ExecError>,
) -> Result<Report, ExecError> {
    let policy = &settings.policy;
    let required = required_scopes(task);
    let capabilities = EXECUTOR_CAPABILITIES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    // One grant, scope for scope the policy admits and the executor is capable
    // of: the display's cover check needs one grant that stands alone.
    let granted: BTreeSet<Scope> = policy.intersection(&capabilities).copied().collect();
    let now = SystemTime::now();
    let expiry = lease_end(now).map_err(ExecError::Gate)?;
    let state_bytes = settings.state_dir.as_os_str().as_encoded_bytes();
    let tenant = TenantId::from_bytes(identity16(&[TENANT_TAG, state_bytes]));
    let holder = WorkerId::from_bytes(identity16(&[WORKER_TAG, state_bytes]));
    let run_id = RunId::from_bytes(identity16(&[RUN_TAG, task_id.as_bytes()]));
    let operation = OperationId::from_bytes(identity16(&[OPERATION_TAG, task_id.as_bytes()]));
    let command_id = CommandId::from_bytes(identity16(&[COMMAND_TAG, task_id.as_bytes()]));

    let proposal = WorkProposal::new(
        run_id,
        tenant,
        operation,
        required.clone(),
        TASK_COST,
        BudgetUnit::Requests,
        ReconciliationStrategy::RequiresHuman,
    );
    let payload = lgwks_std::json::to_vec(&proposal).map_err(RuntimeError::from)?;
    let command = Command::new(
        command_id,
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Submit, payload.into_boxed_slice()),
    )
    .map_err(RuntimeError::from)?;
    let grant = Grant::new(
        tenant,
        holder,
        granted,
        Budget::new(TASK_COST, BudgetUnit::Requests),
        expiry,
        0,
    )
    .map_err(RuntimeError::from)?;

    let slot: Rc<RefCell<Option<Result<Report, ExecError>>>> = Rc::new(RefCell::new(None));
    let owner = TaskOwner {
        repo: task.repo(),
        tenant,
        task_bytes: task.bytes(),
        run: Some(run),
        slot: Rc::clone(&slot),
    };
    let mut runtime = Runtime::<FoldProjection, _>::new(
        RuntimeSettings::with_defaults(
            PolicySnapshot::new(POLICY_REVISION, policy.clone()),
            CapabilitySnapshot::new(capabilities),
            ValiditySnapshot::new(expiry),
            BudgetUnit::Requests,
            TASK_COST,
        ),
        owner,
        Clock::wall(),
    )
    .map_err(ExecError::Gate)?;

    let shown = match runtime.display(&command, holder, &[grant]) {
        Ok(shown) => shown,
        Err(RuntimeError::Binding(BindingError::NoCoveringGrant)) => {
            // The policy does not admit what the task needs (the executor is
            // capable of every required scope, so the refusal is the policy's).
            // Nothing is cloned, written, spawned or invoked: the refusal comes
            // before any tick, and the owner is never reached.
            let line = match first_missing(&required, policy) {
                Some(scope) => format!("gate: policy_refuses: {}", scope_name(scope)),
                None => String::from("gate: no covering grant"),
            };
            return Ok(denied_report(task_id, task_digest, &line, key));
        }
        Err(other) => return Err(ExecError::Gate(other)),
    };
    runtime
        .submit_displayed(command, shown)
        .map_err(ExecError::Gate)?;
    let tick = runtime.tick().map_err(ExecError::Gate)?;

    // The body ran inside the bound reach; its outcome is the run's outcome.
    if let Some(finished) = slot.borrow_mut().take() {
        return finished;
    }
    // The gate refused at admission: the sealed report names the first reason.
    let line = match tick.denied().first() {
        Some(pair) => denial_line(&pair.1),
        None => String::from("gate: denied"),
    };
    Ok(denied_report(task_id, task_digest, &line, key))
}
