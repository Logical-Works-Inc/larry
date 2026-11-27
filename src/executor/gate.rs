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
//!
//! Each task's runtime journal is durable (larry#75, ADR 0023): the file
//! `journals/<task>.journal` under the state directory takes every entry
//! before the runtime does, and goes once the task returns. A file a killed
//! executor left is recovered by `executor::recovery` before anything new
//! runs, and a keyed retry is answered from that evidence instead of running
//! the body again (larry#182 defect 3).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::time::SystemTime;

use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use larry_contracts::binding::{BindingError, EffectTarget, ResolvedEffect};
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{RunId, TenantId, WorkerId};
use larry_contracts::replay::DenialReason;
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::{EffectOwner, validate_proof};
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::protocol::{ProtocolCapability, ProtocolOffer};
use larry_runtime::runtime::{Runtime, restore};
use lgwks_bot::kernel::clock::Clock;

use super::ExecError;
use super::Report;
use super::Settings;
use super::Task;
use super::env::{EXECUTOR_CAPABILITIES, TASK_COST, scope_name, task_settings, with_expiry};
use super::identity;
use super::recovery::{JOURNALS_DIR, RunLock, journal_path, recovered_evidence};
use lgwks_bot::kernel::cap::Auth;

/// Schema version recorded in each resolved executor effect.
const ADAPTER_SCHEMA: u64 = 1;

/// The scopes a terminal session needs (larry#93): everything a briefed task needs,
/// plus the model choosing the commands.
pub(super) fn session_scopes() -> BTreeSet<Scope> {
    EXECUTOR_CAPABILITIES.iter().copied().collect()
}

/// The scopes a terminal session needs when Larry itself confines each command to a
/// workspace copy (larry#243): a briefed task's scopes without `model.command`, which
/// names a command run on the host with no confinement of Larry's own.
pub(super) fn isolated_scopes() -> BTreeSet<Scope> {
    let mut scopes = session_scopes();
    scopes.remove(&Scope::ModelCommand);
    scopes
}

/// The scopes one task requires: repository read and process spawn always,
/// and file write plus model invoke when the task carries a brief the model
/// writes.
pub(super) fn required_scopes(task: &Task) -> BTreeSet<Scope> {
    let mut scopes = BTreeSet::new();
    scopes.insert(Scope::FileRead);
    scopes.insert(Scope::ProcessSpawn);
    if task.brief().is_some() {
        scopes.insert(Scope::FileWrite);
        scopes.insert(Scope::ModelInvoke);
    }
    scopes
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
        DenialReason::NotApproved => String::from("gate: not_approved"),
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

/// The instant `clock` reads, placed after the Unix epoch the way the runtime places its tick
/// (a reading is elapsed time since 1970-01-01T00:00Z), so a lease started here and the tick
/// the runtime pins on the same clock are one timeline (larry#436, SD-4).
///
/// # Errors
///
/// [`RuntimeError::ClockOutOfRange`] when the platform's `SystemTime` cannot hold the reading.
pub(super) fn lease_start(clock: &Clock) -> Result<SystemTime, RuntimeError> {
    let elapsed = clock.now();
    SystemTime::UNIX_EPOCH
        .checked_add(elapsed)
        .ok_or(RuntimeError::ClockOutOfRange { elapsed })
}

/// The owner boundary the executor presents: the task body is the one external
/// effect. `run` is the gated body, held until the bound reach runs it at most
/// once; its outcome is the effect's outcome.
struct TaskOwner<'a, F>
where
    F: FnOnce() -> Result<Report, ExecError>,
{
    /// The repository path the task binds to.
    repo: &'a Path,
    /// The tenant the run belongs to; one state directory is one tenant.
    tenant: TenantId,
    /// The security generation the runtime's fence starts at.
    epoch: SecurityEpoch,
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
        Ok(Some(ResolvedEffect::new(
            target,
            ADAPTER_SCHEMA,
            self.epoch,
        )))
    }

    fn dispatch_bound(
        &mut self,
        intent: &EffectIntent,
        proof: &Auth,
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        validate_proof(proof, &[])?;
        if intent.tenant != self.tenant {
            return Err(BindingError::Changed {
                field: larry_contracts::binding::BindingField::Command,
            });
        }
        if self.slot.borrow().is_some() {
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

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }
}

/// What one gated run presents: the scopes it needs, the path its effect binds
/// to, and the document bytes its payload binds to.
pub(super) struct Subject<'a> {
    /// The scopes the run needs.
    pub(super) required: BTreeSet<Scope>,
    /// The repository a task clones, or the directory a session works in.
    pub(super) path: &'a Path,
    /// The task document, or the session's canonical document.
    pub(super) bytes: &'a [u8],
}

/// Admits `subject` to the runtime and runs its body only inside the authorized
/// owner call. A denied run returns a sealed report naming the refusal and
/// runs nothing; a runtime failure is a typed error; an admitted run's report
/// (or its own typed failure) is the body's outcome.
pub(super) fn admit_and_run(
    subject: Subject<'_>,
    settings: &Settings,
    task_id: &str,
    task_digest: &str,
    key: &lgwks_std::sign::SecretKey,
    run: impl FnOnce() -> Result<Report, ExecError>,
) -> Result<Report, ExecError> {
    if let Some(line) = recovered_evidence(&settings.state_dir, task_id)? {
        // This request was already dispatched once and its executor died inside
        // it. The journal it left is the evidence, recovered or refused; the retry
        // answers from it and nothing here reaches the world again (larry#182
        // defect 3, larry#394).
        return Ok(denied_report(task_id, task_digest, line, key));
    }
    let policy = &settings.policy;
    let required = subject.required;
    let capabilities = EXECUTOR_CAPABILITIES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    // One grant, scope for scope the policy admits and the executor is capable
    // of: the display's cover check needs one grant that stands alone.
    let granted: BTreeSet<Scope> = policy.intersection(&capabilities).copied().collect();
    // The one clock of this run, built once: the lease starts at its reading and the runtime
    // ticks on the same clock, so the expiry and the tick share one timeline (larry#436).
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let now = lease_start(&clock).map_err(ExecError::Gate)?;
    // One settings record decides the policy revision, the admission lease and
    // the security generation the fence starts at, so the executor keeps no
    // copy of any of them (larry#212).
    let run_settings = with_expiry(task_settings(policy), now).map_err(ExecError::Gate)?;
    let expiry = run_settings.validity.valid_until;
    let tenant = identity::tenant_of(&settings.state_dir);
    let holder = identity::worker_of(&settings.state_dir);
    let (run_id, operation, command_id) = identity::ids_of(task_id);

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
    let budget = Budget::new(TASK_COST, BudgetUnit::Requests);
    let grant =
        Grant::new(tenant, holder, granted, budget, expiry, 0).map_err(RuntimeError::from)?;
    let grant = if grant.scopes().is_empty() {
        grant
    } else {
        let policy = larry_contracts::authority::ResourcePolicy::new(grant.scopes().clone());
        let request = larry_contracts::authority::AttenuationRequest::new(
            holder,
            grant.scopes().clone(),
            budget,
            expiry,
        );
        larry_contracts::authority::attenuate(&grant, &policy, &request)
            .map_err(RuntimeError::from)?
    };

    let slot: Rc<RefCell<Option<Result<Report, ExecError>>>> = Rc::new(RefCell::new(None));
    let owner = TaskOwner {
        repo: subject.path,
        tenant,
        epoch: run_settings.security_epoch,
        task_bytes: subject.bytes,
        run: Some(run),
        slot: Rc::clone(&slot),
    };
    std::fs::create_dir_all(settings.state_dir.join(JOURNALS_DIR)).map_err(ExecError::Journals)?;
    let path = journal_path(&settings.state_dir, task_id);
    // A journal already here is a dead run's the recovery kept for a passing fault
    // (larry#394): this retry's failure must never remove that evidence.
    let leftover = path.try_exists().map_err(ExecError::Journals)?;
    // The run's liveness lock, taken before its journal exists and released after the journal
    // is gone, so no other executor's recovery opens this run's journal (larry#456). `None` is a
    // live run with this id; its journal's own lock then refuses this open, as before.
    let lock = RunLock::take(&settings.state_dir, task_id)?;
    let opened = Journal::open(&path, run_settings.journal_capacity)
        .and_then(|journal| restore::<FoldProjection, _>(run_settings, owner, clock, journal));
    let outcome = match opened {
        Ok(mut runtime) => {
            let admission = Admission {
                command,
                holder,
                grant,
                operation,
                tenant,
                required: &required,
                task_id,
                run_id,
                task_digest,
                key,
            };
            drive(&mut runtime, admission, policy, settings, &slot)
        }
        Err(error) => Err(ExecError::Gate(error)),
    };
    let finished = finish(&path, outcome, leftover);
    // A release fault never hides the task's own outcome, only a success.
    match (lock.map(RunLock::release), finished) {
        (Some(Err(fault)), Ok(_)) => Err(fault),
        (_, finished) => finished,
    }
}

/// What one task presents to its runtime once the journal is open.
struct Admission<'a> {
    /// The task's `Submit` command.
    command: Command,
    /// The worker the grant is held by.
    holder: WorkerId,
    /// The one grant the policy and capabilities allow.
    grant: Grant,
    /// The operation the host controls apply to.
    operation: larry_contracts::ids::OperationId,
    /// The state directory's tenant.
    tenant: TenantId,
    /// The scopes the task needs.
    required: &'a BTreeSet<Scope>,
    /// The run's identity.
    task_id: &'a str,
    /// The run id that identity names: what the journal records this request as.
    run_id: RunId,
    /// The task document's digest.
    task_digest: &'a str,
    /// The receipt key a denial is sealed under.
    key: &'a lgwks_std::sign::SecretKey,
}

/// Removes a returned task's journal: its record is now the sealed report the
/// caller holds. A journal the store itself refused is kept, because what it
/// holds is the only evidence of how far the task got, and the next start
/// recovers it. A journal that was `leftover` before this run opened it is a dead run's
/// evidence, and a failed retry keeps it whatever the failure (larry#394). A removal
/// fault never hides the task's own failure.
pub(super) fn finish(
    path: &Path,
    outcome: Result<Report, ExecError>,
    leftover: bool,
) -> Result<Report, ExecError> {
    let kept = (leftover && outcome.is_err())
        || matches!(
            outcome,
            Err(ExecError::Gate(
                RuntimeError::Durable(_)
                    | RuntimeError::DurableRepeat { .. }
                    | RuntimeError::DurabilityShort { .. }
                    | RuntimeError::Codec(_)
            )) | Err(ExecError::Unsealed(_))
        );
    if kept {
        return outcome;
    }
    match std::fs::remove_file(path) {
        Ok(()) => outcome,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => outcome,
        Err(error) => match outcome {
            Ok(_) => Err(ExecError::Journals(error)),
            Err(failure) => Err(failure),
        },
    }
}

/// Displays, negotiates and ticks one admitted task on its runtime, and
/// returns the body's outcome or the sealed refusal.
fn drive<O: EffectOwner>(
    runtime: &mut Runtime<FoldProjection, O>,
    admission: Admission<'_>,
    policy: &BTreeSet<Scope>,
    settings: &Settings,
    slot: &RefCell<Option<Result<Report, ExecError>>>,
) -> Result<Report, ExecError> {
    let Admission {
        command,
        holder,
        grant,
        operation,
        tenant,
        required,
        task_id,
        run_id,
        task_digest,
        key,
    } = admission;
    let shown = match runtime.display(&command, holder, &[grant]) {
        Ok(shown) => shown,
        Err(RuntimeError::Binding(BindingError::NoCoveringGrant)) => {
            // The policy does not admit what the task needs (the executor is
            // capable of every required scope, so the refusal is the policy's).
            // Nothing is cloned, written, spawned or invoked: the refusal comes
            // before any tick, and the owner is never reached.
            let line = match first_missing(required, policy) {
                Some(scope) => format!("gate: policy_refuses: {}", scope_name(scope)),
                None => String::from("gate: no covering grant"),
            };
            return Ok(denied_report(task_id, task_digest, &line, key));
        }
        Err(other) => return Err(ExecError::Gate(other)),
    };
    // The ingress negotiates before the command is queued (E47): the
    // executor offers the runtime's own configured version range and the
    // SubmitCommand capability only, and it is headless — a task's question can
    // wait for no human here. It binds the worker it holds as the offer's
    // principal, the same host-authenticated identity the display carries.
    let offered = BTreeSet::from([ProtocolCapability::SubmitCommand]);
    let offer = ProtocolOffer::new(
        tenant,
        runtime.settings().protocol_range(),
        offered,
        true,
        false,
    )
    .authenticated(holder);
    let submitted = runtime.submit_protocol(&offer, command, Some(shown));
    let acceptance = match submitted {
        Ok((acceptance, _version)) => acceptance,
        Err(error) => {
            // A protocol refusal is a sealed denial exactly like a policy
            // refusal: nothing reached the queue, so nothing ran.
            return match error {
                RuntimeError::Protocol(refusal) => Ok(denied_report(
                    task_id,
                    task_digest,
                    &format!("gate: protocol: {refusal}"),
                    key,
                )),
                other => Err(ExecError::Gate(other)),
            };
        }
    };
    if let Some(control) = settings.fence_control {
        apply_fence_control(runtime, control, operation)?;
    }
    // A duplicate is this request already recorded under this one run id (E28,
    // larry#182 defect 3). The body must not run again, so the caller is answered
    // from the journal's own evidence: a settled run keeps its sealed report in
    // the ledger, and one whose executor died mid-task settles as unknown here.
    if acceptance == CommandAcceptance::DuplicateReplay {
        let line = settled_line(runtime, run_id);
        return Ok(denied_report(task_id, task_digest, &line, key));
    }
    let tick = match runtime.tick() {
        Ok(tick) => tick,
        Err(error) => {
            // The body may already have run and its report be sealed. A tick
            // whose sync refuses would otherwise throw that report away, so the
            // caller is told the run finished and its record could not be made
            // durable (larry#182). A body that failed keeps its own failure,
            // which is the more specific one.
            if let Some(finished) = slot.borrow_mut().take() {
                return match finished {
                    Ok(_) => Err(ExecError::Unsealed(error)),
                    Err(failure) => Err(failure),
                };
            }
            return Err(ExecError::Gate(error));
        }
    };

    // The body ran inside the bound reach; its outcome is the run's outcome.
    if let Some(finished) = slot.borrow_mut().take() {
        return finished;
    }
    // The gate refused at admission: the sealed report names the first reason.
    let line = match tick.denied().first() {
        Some(pair) => denial_line(&pair.1),
        None => match tick.fenced().first() {
            Some(&(_operation, ref refusal)) => format!("gate: fence: {refusal}"),
            None => match tick.refused().first().map(|pair| &pair.1) {
                Some(refusal) => format!("gate: refusal: {refusal}"),
                None => String::from("gate: denied"),
            },
        },
    };
    Ok(denied_report(task_id, task_digest, &line, key))
}

/// Applies one operator authority action after durable admission and before
/// `Runtime::tick` reaches the external owner.
fn apply_fence_control<O: EffectOwner>(
    runtime: &mut Runtime<FoldProjection, O>,
    control: super::FenceControl,
    operation: larry_contracts::ids::OperationId,
) -> Result<(), RuntimeError> {
    match control {
        super::FenceControl::Revoke => runtime
            .fence_mut()
            .revoke(operation)
            .map(|_| ())
            .map_err(RuntimeError::from),
        super::FenceControl::Advance => runtime
            .fence_mut()
            .advance_epoch()
            .map(|_| ())
            .map_err(RuntimeError::from),
        super::FenceControl::Authenticate(epoch) => runtime
            .fence_mut()
            .authenticate(epoch)
            .map(|_| ())
            .map_err(RuntimeError::from),
    }
}

/// The one denial line a duplicate answers with, from the journal's own evidence:
/// what the run this request names settled as. A run nobody here can observe any
/// more reports `recovered`, so the caller can tell an earlier run's evidence from
/// work this call refused (larry#182 defect 3).
fn settled_line<P, O>(runtime: &Runtime<P, O>, run: RunId) -> String
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    match runtime.projection().run_state(run) {
        Some(larry_contracts::state::RunState::OutcomeUnknown) => {
            String::from("gate: the run was recovered from its journal as unknown")
        }
        Some(state) => format!("gate: the run is already recorded as {state:?}"),
        None => String::from("gate: the request is already recorded under this run id"),
    }
}

/// Seeded simulations of the admission lease on an injected clock (larry#436).
#[cfg(test)]
#[path = "sim_gate_clock.rs"]
mod sim_clock;
