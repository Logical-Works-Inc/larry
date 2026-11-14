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
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use larry_contracts::binding::{BindingError, EffectTarget, ResolvedEffect};
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{RunId, TenantId, WorkerId};
use larry_contracts::replay::{CapabilitySnapshot, DenialReason, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::EffectOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::protocol::{
    MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, ProtocolCapability, ProtocolOffer,
    ProtocolVersionRange,
};
use larry_runtime::runtime::{Runtime, RuntimeSettings, restore};

use super::ExecError;
use super::Report;
use super::Settings;
use super::Task;
use super::identity;
use super::recovery::{JOURNALS_DIR, journal_path, recovered_evidence};

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
const EXECUTOR_CAPABILITIES: [Scope; 5] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
    Scope::ModelCommand,
];

/// The policy when `LARRY_POLICY` is unset: the four scopes a task uses. A model
/// command (larry#93) is never in it: only an operator who isolates the commands
/// outside Larry, as a Harbor task container does, names `model.command`.
const DEFAULT_POLICY: [Scope; 4] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
];

/// The policy when `LARRY_POLICY` is unset.
fn default_policy() -> BTreeSet<Scope> {
    DEFAULT_POLICY.iter().copied().collect()
}

/// The scopes a terminal session needs (larry#93): everything a briefed task needs,
/// plus the model choosing the commands.
pub(super) fn session_scopes() -> BTreeSet<Scope> {
    EXECUTOR_CAPABILITIES.iter().copied().collect()
}

/// The prefix the estate capability names carry and a policy name omits.
const CAPABILITY_PREFIX: &str = "larry.";

/// The runtime settings one task runs under: this build's policy revision,
/// the executor's capabilities, the lease ending at `expiry`, one request.
pub(super) fn task_settings(policy: &BTreeSet<Scope>, expiry: SystemTime) -> RuntimeSettings {
    RuntimeSettings::with_defaults(
        PolicySnapshot::new(POLICY_REVISION, policy.clone()),
        CapabilitySnapshot::new(EXECUTOR_CAPABILITIES.iter().copied().collect()),
        ValiditySnapshot::new(expiry),
        BudgetUnit::Requests,
        TASK_COST,
    )
}

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

/// `now + GATE_LEASE` as an instant, or the typed refusal when the platform
/// cannot hold the reading.
pub(super) fn lease_end(now: SystemTime) -> Result<SystemTime, RuntimeError> {
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
    if recovered_evidence(&settings.state_dir, task_id) {
        // This request was already dispatched once and its executor died inside
        // it. The journal it left is the evidence; the retry answers from it and
        // nothing here reaches the world again (larry#182 defect 3).
        return Ok(denied_report(
            task_id,
            task_digest,
            "gate: the run was recovered from its journal as unknown",
            key,
        ));
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
    let now = SystemTime::now();
    let expiry = lease_end(now).map_err(ExecError::Gate)?;
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
        repo: subject.path,
        tenant,
        task_bytes: subject.bytes,
        run: Some(run),
        slot: Rc::clone(&slot),
    };
    std::fs::create_dir_all(settings.state_dir.join(JOURNALS_DIR)).map_err(ExecError::Journals)?;
    let path = journal_path(&settings.state_dir, task_id);
    let run_settings = task_settings(policy, expiry);
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let opened = Journal::open(&path, run_settings.journal_capacity)
        .and_then(|journal| restore::<FoldProjection, _>(run_settings, owner, clock, journal));
    let outcome = match opened {
        Ok(mut runtime) => {
            let admission = Admission {
                command,
                holder,
                grant,
                tenant,
                required: &required,
                task_id,
                run_id,
                task_digest,
                key,
            };
            drive(&mut runtime, admission, policy, &slot)
        }
        Err(error) => Err(ExecError::Gate(error)),
    };
    finish(&path, outcome)
}

/// What one task presents to its runtime once the journal is open.
struct Admission<'a> {
    /// The task's `Submit` command.
    command: Command,
    /// The worker the grant is held by.
    holder: WorkerId,
    /// The one grant the policy and capabilities allow.
    grant: Grant,
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
/// recovers it. A removal fault never hides the task's own failure.
fn finish(path: &Path, outcome: Result<Report, ExecError>) -> Result<Report, ExecError> {
    let kept = matches!(
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
    slot: &RefCell<Option<Result<Report, ExecError>>>,
) -> Result<Report, ExecError> {
    let Admission {
        command,
        holder,
        grant,
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
    // executor offers the runtime's own version range and the SubmitCommand
    // capability only, and it is headless — a task's question can wait for
    // no human here.
    let offered = BTreeSet::from([ProtocolCapability::SubmitCommand]);
    let offer = ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
        offered,
        true,
        false,
    );
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
        None => String::from("gate: denied"),
    };
    Ok(denied_report(task_id, task_digest, &line, key))
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{GATE_LEASE, lease_end};

    /// The executor's lease and the runtime's tick are one timeline: the live clock the executor
    /// passes reads after 2023, not 1970, and the expiry sits one lease after that reading
    /// (larry#182).
    #[test]
    fn the_executors_lease_and_the_runtimes_tick_share_one_timeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let clock = larry_runtime::live_clock()?;
        let tick = SystemTime::UNIX_EPOCH
            .checked_add(clock.now())
            .ok_or("tick instant overflow")?;
        let anchored = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .ok_or("anchor instant overflow")?;
        assert!(
            tick > anchored,
            "the live clock is anchored after 2023, not at 1970: {tick:?}"
        );
        let expiry = lease_end(tick)?;
        let gap = expiry.duration_since(tick)?;
        assert_eq!(gap, GATE_LEASE, "the expiry sits one lease after the tick");
        Ok(())
    }
}
