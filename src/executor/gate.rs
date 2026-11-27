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

use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope};
use larry_contracts::binding::BindingError;
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::EffectOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::protocol::{ProtocolCapability, ProtocolOffer};
use larry_runtime::runtime::{Runtime, RuntimeSettings, restore};
use lgwks_bot::kernel::clock::Clock;

use super::ExecError;
use super::Report;
use super::Settings;
use super::Task;
use super::identity;
use super::recovery::{JOURNALS_DIR, RunLock, journal_path, recovered_evidence};

/// The owner boundary: the task body as the runtime's one external effect.
/// `super` admits the run and seals the report; this module is the owner the
/// bound reach calls (larry#545).
#[path = "gate_owner.rs"]
pub(super) mod owner;

/// The prefix every policy revision digest carries, so a revision computed here
/// can never collide with a digest computed for another purpose.
const POLICY_REVISION_TAG: &[u8] = b"larry/executor/policy-revision/v1\0";

/// The request each task costs against its one-run budget.
const TASK_COST: u64 = 1;

/// The scopes the executor is capable of, pinned into the capability snapshot.
use scopes::EXECUTOR_CAPABILITIES;
/// The default and session scope bundles, from [`scopes`]: what an unset
/// `LARRY_POLICY` admits and what a terminal session runs under.
pub(super) use scopes::{default_policy, isolated_scopes, session_scopes};

/// The prefix the estate capability names carry and a policy name omits.
const CAPABILITY_PREFIX: &str = "larry.";

/// The tenants one executor state directory serves: its own, because the
/// tenant is derived from the directory (larry#182). Every shared runtime cap
/// is whole to that one tenant, and the shipped path names the count rather
/// than leaving a default nothing reached (larry#333, SD-1).
const SERVED_TENANTS: usize = 1;

/// The runtime settings one task runs under: the revision of the policy record
/// it runs under, the executor's capabilities, one request, and the runtime's
/// own default lease.
///
/// The revision is derived from the record itself, so the same `LARRY_POLICY`
/// always names the same revision and a changed policy names a different one,
/// rather than a constant only this build knows (larry#212).
pub(super) fn task_settings(policy: &BTreeSet<Scope>) -> RuntimeSettings {
    let mut settings = RuntimeSettings::with_defaults(
        PolicySnapshot::new(policy_revision(policy), policy.clone()),
        CapabilitySnapshot::new(EXECUTOR_CAPABILITIES.iter().copied().collect()),
        ValiditySnapshot::new(SystemTime::UNIX_EPOCH),
        BudgetUnit::Requests,
        TASK_COST,
    );
    settings.max_tenants = SERVED_TENANTS;
    settings
}

/// The revision of one policy record: a digest over the policy's own scope
/// names, in `Scope` order, so the same admitted set always yields the same
/// revision and a decision is only ever comparable inside one (larry#212).
fn policy_revision(policy: &BTreeSet<Scope>) -> u64 {
    let mut record = String::new();
    for scope in policy {
        record.push_str(&scope_name(*scope));
        record.push('\n');
    }
    let digest =
        larry_contracts::ids::ContentDigest::over(&[POLICY_REVISION_TAG, record.as_bytes()]);
    let mut revision = 0u64;
    for byte in digest.as_bytes().iter().take(8) {
        revision = (revision << 8) | u64::from(*byte);
    }
    revision
}

/// `settings` with its validity pinned to the admission lease that starts at
/// `now`: the lease length is the runtime's own setting, so the executor keeps
/// no clock constant of its own (larry#212).
pub(super) fn with_expiry(
    settings: RuntimeSettings,
    now: SystemTime,
) -> Result<RuntimeSettings, RuntimeError> {
    let mut settings = settings;
    settings.validity = ValiditySnapshot::new(lease_end(now, settings.lease)?);
    Ok(settings)
}

/// The dotted name a scope carries in `LARRY_POLICY` and in a denial line: the
/// estate capability name `replay::capability` gives it (larry#77), without
/// its `larry.` prefix, so the scope-to-name mapping has one owner.
pub(super) fn scope_name(scope: Scope) -> String {
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

/// The `LARRY_POLICY` scopes `read` yields: unset is the five model-task scopes the
/// executor uses, the empty string is no scope, names are trimmed and
/// comma-separated, and an unknown name is a typed refusal.
pub(super) fn policy(read: &impl Fn(&str) -> Option<String>) -> Result<BTreeSet<Scope>, ExecError> {
    let Some(text) = read("LARRY_POLICY") else {
        return Ok(default_policy());
    };
    policy_names(Some(&text))
}

/// Parses a durable policy record. A missing ledger row is remote deny-all.
pub(super) fn policy_names(text: Option<&str>) -> Result<BTreeSet<Scope>, ExecError> {
    let Some(text) = text else {
        return Ok(BTreeSet::new());
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
/// and file write, model invoke and network egress when the task carries a
/// brief the model writes.
pub(super) fn required_scopes(task: &Task) -> BTreeSet<Scope> {
    let mut scopes = BTreeSet::new();
    scopes.insert(Scope::FileRead);
    scopes.insert(Scope::ProcessSpawn);
    if task.brief().is_some() {
        scopes.insert(Scope::FileWrite);
        scopes.insert(Scope::ModelInvoke);
        scopes.insert(Scope::NetworkEgress);
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

/// `now + lease` as an instant, or the typed refusal when the platform cannot
/// hold the reading.
pub(super) fn lease_end(now: SystemTime, lease: Duration) -> Result<SystemTime, RuntimeError> {
    let elapsed = match now.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(elapsed) => elapsed,
        Err(_) => Duration::ZERO,
    };
    let total = elapsed.saturating_add(lease);
    SystemTime::UNIX_EPOCH
        .checked_add(total)
        .ok_or(RuntimeError::ClockOutOfRange { elapsed: total })
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
    if let Some(line) = recovered_evidence(settings.state_dir(), task_id)? {
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
    let tenant = identity::tenant_of(settings.state_dir());
    let holder = identity::worker_of(settings.state_dir());
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
    let owner = owner::TaskOwner {
        repo: subject.path,
        tenant,
        epoch: run_settings.security_epoch,
        task_bytes: subject.bytes,
        run: Some(run),
        slot: Rc::clone(&slot),
    };
    std::fs::create_dir_all(settings.state_dir().join(JOURNALS_DIR))
        .map_err(ExecError::Journals)?;
    let path = journal_path(settings.state_dir(), task_id);
    // A journal already here is a dead run's the recovery kept for a passing fault
    // (larry#394): this retry's failure must never remove that evidence.
    let leftover = path.try_exists().map_err(ExecError::Journals)?;
    // The run's liveness lock, taken before its journal exists and released after the journal
    // is gone, so no other executor's recovery opens this run's journal (larry#456). `None` is a
    // live run with this id; its journal's own lock then refuses this open, as before.
    let lock = RunLock::take(settings.state_dir(), task_id)?;
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
                operation,
                task_digest,
                key,
                fence_control: settings.fence_control,
            };
            drive(&mut runtime, admission, policy, &slot)
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
    /// The state directory's tenant.
    tenant: TenantId,
    /// The scopes the task needs.
    required: &'a BTreeSet<Scope>,
    /// The run's identity.
    task_id: &'a str,
    /// The run id that identity names: what the journal records this request as.
    run_id: RunId,
    /// The operation the fence withdraws when the host action names one.
    operation: OperationId,
    /// The task document's digest.
    task_digest: &'a str,
    /// The receipt key a denial is sealed under.
    key: &'a lgwks_std::sign::SecretKey,
    /// The operator's one host fence action, applied after durable admission.
    fence_control: Option<super::fence_control::FenceControl>,
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
        operation,
        task_digest,
        key,
        fence_control,
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
    if let Some(control) = fence_control {
        super::fence_control::apply(runtime, control, operation)?;
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
    let line = super::fence_control::report_line(&tick);
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

mod scopes;
/// Seeded simulations of the admission lease on an injected clock (larry#436).
#[cfg(test)]
#[path = "sim_gate_clock.rs"]
mod sim_clock;

#[cfg(test)]
#[path = "gate_owner_tests.rs"]
mod owner_tests;

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use larry_contracts::authority::Scope;

    use super::{SERVED_TENANTS, default_policy, policy_revision, task_settings, with_expiry};

    /// The executor's lease and the runtime's tick are one timeline: the live
    /// clock the executor passes reads after 2023, not 1970, and the validity
    /// the runtime is built with sits exactly one configured lease after that
    /// reading (larry#182, larry#212).
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
        let run_settings = with_expiry(task_settings(&default_policy()), tick)?;
        let gap = run_settings.validity.valid_until.duration_since(tick)?;
        assert_eq!(
            gap, run_settings.lease,
            "the validity sits one configured lease after the tick"
        );
        Ok(())
    }

    /// The executor's settings name the tenants one state directory serves, so
    /// the runtime's shared caps divide by a value the shipped task path sets,
    /// not by a default nothing reached (larry#333, SD-1).
    #[test]
    fn task_settings_name_the_tenants_the_executor_serves() -> Result<(), Box<dyn std::error::Error>>
    {
        let settings = task_settings(&default_policy());
        assert_eq!(
            settings.max_tenants, SERVED_TENANTS,
            "the task settings carry the tenant count"
        );
        assert_eq!(SERVED_TENANTS, 1, "one state directory is one tenant");
        let tick = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .ok_or("tick instant overflow")?;
        let carried = with_expiry(settings, tick)?;
        assert_eq!(
            carried.max_tenants, 1,
            "the settings a task runs under carry it"
        );
        Ok(())
    }

    /// The policy revision is a fact about the policy record, not a constant:
    /// the same admitted set yields the same revision, and one more scope
    /// yields a different one (larry#212).
    #[test]
    fn task_settings_derive_the_revision_from_the_policy_record() {
        let first = task_settings(&default_policy());
        let again = task_settings(&default_policy());
        assert_eq!(
            first.policy.revision(),
            again.policy.revision(),
            "the same policy record yields the same revision"
        );

        let mut widened = default_policy();
        widened.insert(Scope::ModelCommand);
        assert_ne!(
            policy_revision(&default_policy()),
            policy_revision(&widened)
        );
    }
}
