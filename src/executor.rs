//! The executor behind `larry exec` (larry#85), its signed receipts (larry#86)
//! and the model step that writes a brief's files before acceptance (larry#123).

mod author;
mod compact;
mod confine;
mod error;
mod gate;
mod identity;
mod island;
mod ledger;
mod prepared;
mod receipt;
mod recovery;
pub mod routes;
mod run;
mod session;
#[cfg(test)]
mod sim_env;
#[cfg(test)]
pub(crate) mod sim_tree;
mod task;
mod workspace;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub(crate) use author::is_writable_path;
use larry_contracts::authority::Scope;
use larry_contracts::ids::TenantId;
pub(crate) use ledger::Ledger;
use lgwks_std::sign::PublicKey;
use prepared::{Prepared, live_budget, prepare};
use run::{Place, Track, run_command};
use workspace::{Workspace, report_members, sweep};
pub(crate) use workspace::{hold_origin, origin_lock_path};

use crate::model::{ModelClient, ModelConfig, Route};
pub use author::AuthorFault;
pub use confine::ConfineError;
#[cfg(test)]
pub(crate) use confine::Platform;
pub use error::ExecError;
pub(crate) use island::Isolation;
pub(crate) use receipt::Seal;
use receipt::parse_public_key;
pub use receipt::{MAX_REPORT_BYTES, ReceiptError, Report, Verdict, verify};
pub use session::SessionError;
pub(crate) use session::{Approval, Handoff, MAX_INSTRUCTION_BYTES, Observer, Stop, run_session};
pub use task::{Task, TaskError};

/// The denial a task asking for model commands receives until Nova isolation
/// lands (logicalworks-crates#846): nothing runs, and the signed report says so.
const MODEL_COMMANDS_DENIED: &str =
    "model_commands: isolation unavailable until Nova lands (logicalworks-crates#846)";

/// Default longest an acceptance command may run, in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 600_000;

/// Largest accepted command timeout, in milliseconds (one day).
const MAX_TIMEOUT_MS: u64 = 86_400_000;

/// Whether Larry reads the variable `name`: `PATH`, `HOME` and every `LARRY_` one.
fn read_by_larry(name: &str) -> bool {
    name == "PATH" || name == "HOME" || name.starts_with("LARRY_")
}

/// The process environment as one snapshot every setting reads from: a
/// variable that is absent is unset, and a variable Larry reads whose name or
/// value is not UTF-8 refuses the whole snapshot, so no setting falls back to
/// a default on bytes it could not read.
///
/// # Errors
///
/// [`ExecError::NotUnicode`] naming the first such variable.
pub fn process_env() -> Result<impl Fn(&str) -> Option<String> + Send + Sync + 'static, ExecError> {
    let table = snapshot(std::env::vars_os())?;
    Ok(move |name: &str| table.get(name).cloned())
}

/// The variables Larry reads out of `vars`, as text: the one place a byte string from the
/// environment becomes a setting. A refusal names the variable, never its value.
fn snapshot(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<BTreeMap<String, String>, ExecError> {
    let mut table = BTreeMap::new();
    for (name, value) in vars {
        let lossy = name.to_string_lossy().into_owned();
        if !read_by_larry(&lossy) {
            continue;
        }
        match (name.into_string(), value.into_string()) {
            (Ok(name), Ok(value)) => {
                table.insert(name, value);
            }
            (Ok(_) | Err(_), Err(_)) | (Err(_), Ok(_)) => {
                return Err(ExecError::NotUnicode { name: lossy });
            }
        }
    }
    Ok(table)
}

/// The non-empty `LARRY_STATE_DIR` that `read` yields.
fn state_dir(read: &impl Fn(&str) -> Option<String>) -> Result<PathBuf, ExecError> {
    read("LARRY_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(ExecError::StateDirUnset)
}

/// The state directory's store, opened (and created or migrated) under its lock: the
/// one home of the receipt key and the tenant key digests (larry#89).
pub(crate) fn state_ledger(read: &impl Fn(&str) -> Option<String>) -> Result<Ledger, ExecError> {
    Ledger::open(&state_dir(read)?)
}

/// The public half of the selected tenant's active receipt key, read-only: a
/// tenant with no key yet is refused, and no seed is ever drawn here.
/// `larry key` and `larry verify` on an empty state directory refuse without
/// creating the store or minting a secret (larry#211).
pub fn state_public_key(read: impl Fn(&str) -> Option<String>) -> Result<PublicKey, ExecError> {
    let directory = state_dir(&read)?;
    if !directory.join("executor.db").exists() {
        return Err(ExecError::NoStore);
    }
    let ledger = Ledger::open(&directory)?;
    let (_, tenant) = ledger.selected_tenant(&directory, &read)?;
    ledger
        .active_receipt_key(&tenant)?
        .map(|(_, public)| public)
        .ok_or(ExecError::NoReceiptKey)
}

/// Provisions the tenant-owned executor ledger before a local or remote door runs.
/// The policy text uses the same comma-separated scope names as `LARRY_POLICY`;
/// roots are canonicalized and bounded by the ledger owner. The tenant is the
/// selected one: `LARRY_TENANT` when it names one, else the store's default.
pub fn provision_tenant(
    read: impl Fn(&str) -> Option<String>,
    policy_text: &str,
    roots: &[&std::path::Path],
) -> Result<(), ExecError> {
    let state = state_dir(&read)?;
    let ledger = Ledger::open(&state)?;
    let (_, tenant) = ledger.selected_tenant(&state, &read)?;
    let policy = gate::policy_names(Some(policy_text))?;
    ledger.set_tenant_policy(&tenant, &policy)?;
    for root in roots {
        ledger.set_tenant_repo_root(&tenant, root)?;
    }
    Ok(())
}

/// The key a verifier trusts: `LARRY_RECEIPT_KEY` (64 hex characters) when it is
/// set, which needs no state directory, else the state directory's own key.
pub fn trusted_key(read: impl Fn(&str) -> Option<String>) -> Result<PublicKey, ExecError> {
    match read("LARRY_RECEIPT_KEY").filter(|value| !value.is_empty()) {
        Some(text) => parse_public_key(&text).map_err(ExecError::Receipt),
        None => state_public_key(read),
    }
}

/// Verifies a sealed report against the selected tenant's keys in the state
/// directory: the receipt's key id selects the key, so a rotated tenant
/// verifies old receipts by id and new ones under the new id, while another
/// tenant's receipt is refused as untrusted (larry#211).
///
/// # Errors
///
/// [`ExecError::NoStore`] when the store is absent, [`ExecError::NoReceiptKey`]
/// when the named key id is gone, and [`ExecError::Receipt`] when the report
/// is not a report this tenant sealed.
pub fn verify_state_report(
    read: impl Fn(&str) -> Option<String>,
    bytes: &[u8],
) -> Result<Verdict, ExecError> {
    let directory = state_dir(&read)?;
    if !directory.join("executor.db").exists() {
        return Err(ExecError::NoStore);
    }
    let ledger = Ledger::open(&directory)?;
    let (_, tenant) = ledger.selected_tenant(&directory, &read)?;
    let body = lgwks_std::json::from_slice::<lgwks_std::json::Value>(bytes)
        .map_err(|cause| ExecError::Receipt(ReceiptError::Malformed(cause)))?;
    let key_id = receipt::key_id_of(&body).map_err(ExecError::Receipt)?;
    let keys = ledger.receipt_public_keys(&tenant)?;
    let trusted = keys
        .iter()
        .find(|entry| entry.0 == key_id)
        .map(|entry| &entry.1)
        .ok_or(ExecError::NoReceiptKey)?;
    verify(bytes, trusted).map_err(ExecError::Receipt)
}

/// Rotates the selected tenant's receipt key: appends the next key id and
/// returns it with the new public key. Old ids stay verifiable up to the kept
/// history; sealing moves to the new id at once (larry#211).
pub fn rotate_receipt_key(
    read: impl Fn(&str) -> Option<String>,
) -> Result<(i64, PublicKey), ExecError> {
    let directory = state_dir(&read)?;
    let ledger = Ledger::open(&directory)?;
    let (_, tenant) = ledger.selected_tenant(&directory, &read)?;
    let (id, secret) = ledger.rotate_receipt_key(&tenant)?;
    Ok((id, secret.public_key()))
}

/// Reads one environment variable: the process environment in the binary, a
/// table in a test. A route's key is read through it when a task reaches the
/// model, not before, so a key rotated between two tasks is the next task's.
type EnvReader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Settings the executor reads from its environment.
#[derive(Clone)]
#[non_exhaustive]
pub struct Settings {
    /// Directory holding the ledger and the workspaces.
    state_dir: PathBuf,
    /// The tenant this run executes as: `LARRY_TENANT` when it names one, else
    /// the store's default (larry#211).
    tenant: TenantId,
    /// The hex text every ledger row of this tenant carries.
    tenant_text: String,
    /// Longest an acceptance command may run before its group is killed.
    timeout: Duration,
    /// The PATH the acceptance commands and git run under.
    path: String,
    /// The scopes the gate authorizes tasks against. Local settings read
    /// `LARRY_POLICY`; remote settings read the tenant ledger.
    policy: BTreeSet<Scope>,
    /// Where a task's route reads its key and the `LARRY_MODEL_*` knobs from.
    read: EnvReader,
}

impl fmt::Debug for Settings {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Settings")
            .field("state_dir", &self.state_dir)
            .field("timeout", &self.timeout)
            .field("path", &self.path)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Reads the settings through `read`, which yields one variable or `None`.
    /// The model is not read here: a task's route is resolved, and its key and
    /// `LARRY_MODEL_*` knobs are read, only when the task reaches a model.
    pub fn from_env(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Result<Settings, ExecError> {
        Self::from_env_mode(read, true)
    }

    /// Reads executor settings for a remote door. Remote policy never falls back
    /// to `LARRY_POLICY`, and a missing ledger row is deny-all.
    pub(crate) fn remote_from_env(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Result<Settings, ExecError> {
        Self::from_env_mode(read, false)
    }

    /// Reads the common settings and selects the policy owner for the door. The
    /// tenant is selected first, so the remote policy read names the same
    /// tenant the run executes as (larry#211).
    fn from_env_mode(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
        local: bool,
    ) -> Result<Settings, ExecError> {
        let state_dir = state_dir(&read)?;
        let timeout_ms = match read("LARRY_EXEC_TIMEOUT_MS") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => match value.parse::<u64>() {
                Ok(milliseconds) if (1..=MAX_TIMEOUT_MS).contains(&milliseconds) => milliseconds,
                _ => return Err(ExecError::BadTimeout { value }),
            },
        };
        let path = read("PATH").ok_or(ExecError::PathUnset)?;
        let store = Ledger::open(&state_dir)?;
        let (tenant, tenant_text) = store.selected_tenant(&state_dir, &read)?;
        // One spelling for every downstream path: a relative or symlinked state
        // directory canonicalizes here, once, so workspaces, journals and git
        // never see two spellings of one directory (larry#211).
        let state_dir = std::fs::canonicalize(&state_dir).map_err(ExecError::StateDir)?;
        let policy = if local {
            gate::policy(&read)?
        } else {
            store.tenant_policy(&tenant_text)?
        };
        Ok(Settings {
            state_dir,
            tenant,
            tenant_text,
            timeout: Duration::from_millis(timeout_ms),
            path,
            policy,
            read: Arc::new(read),
        })
    }

    /// The directory holding the ledger, the workspaces and `larry eval`'s origins.
    pub(crate) fn state_dir(&self) -> &std::path::Path {
        self.state_dir.as_path()
    }

    /// The tenant this run executes as.
    pub(crate) fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// The hex text every ledger row of this tenant carries.
    pub(crate) fn tenant_text(&self) -> &str {
        self.tenant_text.as_str()
    }

    /// Refreshes the remote door's policy from the durable tenant ledger. The process may
    /// stay open while an operator provisions or withdraws a tenant, so a cached snapshot
    /// must not outlive the prompt that uses it.
    pub(crate) fn refresh_remote_policy(&mut self) -> Result<(), ExecError> {
        self.policy = Ledger::open(&self.state_dir)?.tenant_policy(&self.tenant_text)?;
        Ok(())
    }

    /// The route a task naming `named` (or none) sends its model calls to: the
    /// named route, else the tenant's default, else the environment's, else a
    /// refusal. Nothing is sent to resolve it.
    pub(crate) fn route(&self, ledger: &Ledger, named: Option<&str>) -> Result<Route, ExecError> {
        routes::resolve_for(ledger, &self.state_dir, named, &|name: &str| {
            (self.read)(name)
        })
    }

    /// The route `larry eval` scores: what a task naming none resolves to.
    pub(crate) fn arm_route(&self) -> Result<Route, ExecError> {
        self.route(&Ledger::open(&self.state_dir)?, None)
    }

    /// The model configuration for calls over `route`: its key is read now, from
    /// the variable the route names.
    pub(crate) fn model_config(&self, route: &Route) -> Result<ModelConfig, ExecError> {
        ModelConfig::for_route(route, &|name: &str| (self.read)(name))
            .map_err(ExecError::ModelConfig)
    }

    /// Makes `dir`, whose files are written, a one-commit repository under these settings.
    pub(crate) fn seed_origin(&self, dir: &std::path::Path) -> Result<(), ExecError> {
        workspace::seed_origin(&self.state_dir, &self.path, dir)
    }

    /// Removes the origins a dead `larry eval` left under the state directory. An origin
    /// that cannot be settled is reported and kept, never a refusal of the eval (larry#334).
    pub(crate) fn sweep_origins(&self) -> Result<(), ExecError> {
        for fault in workspace::sweep_origins(&self.state_dir)? {
            fault.report();
        }
        Ok(())
    }
}

/// Whether to retry a model refusal after `wait`. The executor never blocks on a
/// clock: a retry that needs no wait goes ahead (`LARRY_MODEL_BACKOFF_MS=0`), and
/// one that would wait stops, so the planner gets the typed refusal, with any
/// `Retry-After`, at once and decides whether to resubmit.
const fn backoff(wait: Duration) -> bool {
    wait.is_zero()
}

/// Runs a task: clones its repository at the base, has the model write the brief's files
/// when the task carries one, runs each acceptance command in order, and returns the report
/// with the staged diff, sealed under the tenant's key. A task asking for model
/// commands is denied: nothing runs, and its sealed report is never accepted. Every other
/// task runs only after the admission gate authorized it (larry#164): the task body is the
/// runtime's one dispatched effect, and a policy refusal seals a report that ran nothing.
pub fn execute(task: &Task, settings: &Settings) -> Result<Report, ExecError> {
    let prepared = prepare(settings)?;
    let repository = prepared.ledger.authorized_repo(
        prepared.tenant_text.as_str(),
        &settings.state_dir,
        task.repo(),
    )?;
    execute_prepared(task, settings, &prepared, &repository)
}

/// Runs an evaluator-owned origin. Public task execution refuses every state-directory path;
/// this crate-private route is narrower still, accepting only a canonical origin beneath the
/// evaluator's own `<state>/eval` directory.
pub(crate) fn execute_eval(task: &Task, settings: &Settings) -> Result<Report, ExecError> {
    let prepared = prepare(settings)?;
    let repository = std::fs::canonicalize(task.repo()).map_err(|cause| ExecError::RepoPath {
        repo: task.repo().to_path_buf(),
        cause,
    })?;
    let eval_root =
        std::fs::canonicalize(settings.state_dir().join("eval")).map_err(ExecError::StateDir)?;
    if !repository.starts_with(eval_root) {
        return Err(ExecError::RepoInsideState { repo: repository });
    }
    execute_prepared(task, settings, &prepared, &repository)
}

/// Executes after the caller has supplied a repository authorized for this route.
fn execute_prepared(
    task: &Task,
    settings: &Settings,
    prepared: &Prepared,
    repository: &std::path::Path,
) -> Result<Report, ExecError> {
    let ledger = &prepared.ledger;
    let tenant = prepared.tenant_text.as_str();
    let seal = Seal {
        id: prepared.seal_id,
        key: &prepared.seal_key,
    };
    let task_id = identity::run_id(task, &prepared.tenant)?;
    let task_digest = digest_hex(task.bytes());
    // A task that names an idempotency key is the client's statement that a retry
    // of it is the same request, so a retry whose record the ledger already holds
    // is answered from that record and the body never runs a second time
    // (larry#182 defect 3).
    let resumable = task.idempotency_key().is_some();
    if resumable && let Some(earlier) = ledger.report(tenant, &task_id, MAX_REPORT_BYTES)? {
        return Report::parse_for(ledger, tenant, &earlier);
    }
    if task.has_model_commands() {
        let report = Report::sealed(
            lgwks_std::json::json!({
                "task_id": task_id,
                "task_digest": task_digest,
                "diff": null,
                "results": [],
                "denied": [MODEL_COMMANDS_DENIED],
            }),
            seal.key,
            seal.id,
        );
        if resumable {
            ledger.keep_report(
                tenant,
                &task_id,
                &task_digest,
                report.bytes(),
                &live_budget()?,
            )?;
        }
        return Ok(report);
    }
    let subject = gate::Subject {
        required: gate::required_scopes(task),
        path: repository,
        bytes: task.bytes(),
    };
    let report = gate::admit_and_run(
        subject,
        settings,
        &task_id,
        &task_digest,
        prepared.tenant,
        seal,
        || {
            run_task(
                task,
                repository,
                settings,
                &task_id,
                &task_digest,
                prepared,
                seal,
            )
        },
    )?;
    if resumable && report.carries_a_seal() {
        ledger.keep_report(
            tenant,
            &task_id,
            &task_digest,
            report.bytes(),
            &live_budget()?,
        )?;
    }
    Ok(report)
}

/// What a task with a brief needs before anything is cloned: the route its model
/// calls go to and a client built from it.
struct ModelStep<'task> {
    /// The route the task resolved to; the sealed report records it.
    route: Route,
    /// The brief the model writes files for.
    brief: &'task author::Brief,
    /// The client over `route`, holding the key read for this task alone.
    client: ModelClient,
}

/// Resolves the task's route and builds its client, only when the task has a
/// brief: a task with none never reaches a model, so its route is only checked
/// when it names one, and a typo in a model setting fails nothing it does not use.
/// An unknown name is refused here, before any byte leaves the host.
fn model_step<'task>(
    task: &'task Task,
    settings: &Settings,
    ledger: &Ledger,
) -> Result<Option<ModelStep<'task>>, ExecError> {
    let Some(brief) = task.brief() else {
        if let Some(named) = task.route() {
            settings.route(ledger, Some(named))?;
        }
        return Ok(None);
    };
    let route = settings.route(ledger, task.route())?;
    // The one timeline the retry budget reads, built the way the gate builds its own (larry#436).
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let client = ModelClient::new(settings.model_config(&route)?, clock, backoff);
    Ok(Some(ModelStep {
        route,
        brief,
        client,
    }))
}

/// The task body the gate authorizes: clones the repository at the base, has
/// the model write the brief's files when the task carries one, runs each
/// acceptance command in order, and returns the report with the staged diff,
/// sealed under the tenant's key. It runs at most once, inside the
/// gate's bound owner reach.
fn run_task(
    task: &Task,
    repository: &std::path::Path,
    settings: &Settings,
    task_id: &str,
    task_digest: &str,
    prepared: &Prepared,
    seal: Seal<'_>,
) -> Result<Report, ExecError> {
    // A brief lets the model write the tree, so the command that runs next is model-chosen
    // code; it runs confined to the workspace. `larry_submit_task` confines too, because a
    // remote peer drives the run and no person watches a command (larry#324, ADR 0088).
    let confined = task.confines_acceptance();
    // A host that cannot confine refuses the task here, before the route, the clone or a
    // command, with the typed refusal a terminal session already uses (larry#243).
    if confined {
        confine::supported(confine::Platform::host()).map_err(ExecError::Confine)?;
    }
    let step = model_step(task, settings, &prepared.ledger)?;
    let workspace = Workspace::create(
        &settings.state_dir,
        repository,
        task.base(),
        task_id,
        &settings.path,
    )?;
    let authored = match step {
        None => None,
        Some(ModelStep {
            route,
            brief,
            client,
        }) => {
            // The tenant's digest, the same text a route row is keyed by, names the
            // cache partition so no two tenants share a prompt prefix (larry#230).
            let partition = digest_hex(settings.tenant().as_bytes());
            let mut records = author::author(brief, workspace.path(), &client, &partition)?;
            for record in &mut records {
                if let Some(object) = record.as_object_mut() {
                    object.insert(String::from("route"), route.seal());
                }
            }
            Some(records)
        }
    };
    let confine = if confined {
        // The scratch directory sits beside the tree and outside it, so a command's scratch
        // files never enter the diff (larry#243).
        std::fs::create_dir_all(workspace.scratch()).map_err(ExecError::Workspace)?;
        let confine = confine::Confine::new(
            confine::Platform::host(),
            &settings.state_dir,
            workspace.path(),
            &workspace.scratch(),
        )
        .map_err(ExecError::Confine)?;
        // The host proves it enforces the confinement before a command runs (larry#243).
        confine.probe(&settings.path).map_err(ExecError::Confine)?;
        Some(confine)
    } else {
        None
    };
    let mut results = Vec::with_capacity(task.acceptance().len());
    for argv in task.acceptance() {
        // An acceptance command has no person watching it who could cancel it,
        // so only the timeout ends it (larry#242).
        let place = match confine {
            Some(ref confine) => Place::Confined(confine),
            None => Place::Open(workspace.path()),
        };
        let result = run_command(
            argv,
            place,
            &settings.path,
            settings.timeout,
            Track {
                ledger: &prepared.ledger,
                tenant: prepared.tenant_text.as_str(),
                task_id,
            },
            &|| false,
        )?;
        results.push(result.into_json());
    }
    let diff = workspace.diff(task.base())?;
    let mut body = lgwks_std::json::json!({
        "task_id": task_id,
        "task_digest": task_digest,
        "results": results,
        "denied": [],
    });
    // The diff members name whether the diff is the whole change (larry#206).
    if let Some(object) = body.as_object_mut() {
        for (name, member) in report_members(diff) {
            object.insert(name.to_owned(), member);
        }
    }
    // Only a briefed run names what the model wrote, so a plain run's report keeps its shape.
    if let Some(records) = authored
        && let Some(object) = body.as_object_mut()
    {
        object.insert(
            String::from("authored"),
            lgwks_std::json::Value::Array(records),
        );
    }
    Ok(Report::sealed(body, seal.key, seal.id))
}

/// The BLAKE3 digest of `bytes`, as 64 hex characters.
fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex()
}

#[cfg(test)]
mod issue207_tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use larry_contracts::authority::Scope;
    use lgwks_std::json::json;

    use super::sim_tree::{Origin, Shape, origin, scratch, settings};
    use super::{Ledger, Settings, Task, execute};

    fn task(repo: &Path) -> Result<Task, Box<dyn std::error::Error>> {
        let document = json!({
            "repo": repo.to_string_lossy(),
            "base": "HEAD",
            "acceptance": [["true"]]
        });
        Ok(Task::parse(document.to_string().as_bytes())?)
    }

    fn remote_settings(state: &Path) -> Result<Settings, Box<dyn std::error::Error>> {
        let state = state.to_string_lossy().into_owned();
        let path = std::env::var("PATH")?;
        Ok(Settings::remote_from_env(move |name| match name {
            "LARRY_STATE_DIR" => Some(state.clone()),
            "PATH" => Some(path.clone()),
            _ => None,
        })?)
    }

    /// The real executor rejects the outside and state paths before workspace creation.
    #[test]
    fn sim_executor_authorizes_canonical_repositories_before_clone()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = scratch("issue207-state")?;
        let root = scratch("issue207-root")?;
        let local = settings(&state, Some("file.read,process.spawn"))?;
        let Origin {
            dir: repository, ..
        } = origin(&local, &root, 1, Shape::Git)?;
        let outside = scratch("issue207-outside")?;
        let valid = task(&repository)?;
        assert!(execute(&valid, &local)?.accepted());
        let outside_task = task(&outside)?;
        assert!(matches!(
            execute(&outside_task, &local),
            Err(super::ExecError::RepoNotAllowed { .. })
        ));
        std::fs::create_dir_all(state.join("ws"))?;
        let state_task = task(&state.join("ws"))?;
        assert!(matches!(
            execute(&state_task, &local),
            Err(super::ExecError::RepoInsideState { .. })
        ));
        assert!(state.join("ws").read_dir()?.next().is_none());
        Ok(())
    }

    /// Remote settings read each tenant's durable policy, so equal tasks do not share authority.
    #[test]
    fn sim_remote_tenant_policies_do_not_cross() -> Result<(), Box<dyn std::error::Error>> {
        let state_a = scratch("issue207-a")?;
        let state_b = scratch("issue207-b")?;
        let root = scratch("issue207-shared")?;
        let local = settings(&state_a, Some("file.read,process.spawn"))?;
        let Origin {
            dir: repository, ..
        } = origin(&local, &root, 2, Shape::Git)?;
        let allowed = [Scope::FileRead, Scope::ProcessSpawn]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let ledger_a = Ledger::open(&state_a)?;
        ledger_a.set_tenant_policy(local.tenant_text(), &allowed)?;
        let remote_b = remote_settings(&state_b)?;
        let ledger_b = Ledger::open(&state_b)?;
        ledger_b.set_tenant_repo_root(remote_b.tenant_text(), &root)?;
        ledger_b.set_tenant_policy(remote_b.tenant_text(), &BTreeSet::new())?;
        let same = task(&repository)?;
        let first = execute(&same, &remote_settings(&state_a)?)?;
        let second = execute(&same, &remote_b)?;
        assert!(first.accepted(), "the allowed tenant runs the task");
        assert!(!second.accepted(), "the denied tenant runs nothing");
        Ok(())
    }

    /// The repository-root bound is inclusive at its last valid row and refuses the next
    /// distinct canonical root without writing it.
    #[test]
    fn a_tenant_repository_root_cap_refuses_the_next_root() -> Result<(), Box<dyn std::error::Error>>
    {
        let state = scratch("issue207-cap-state")?;
        let roots = scratch("issue207-cap-roots")?;
        let ledger = Ledger::open(&state)?;
        for index in 0..64_usize {
            let root = roots.join(format!("root-{index}"));
            std::fs::create_dir_all(&root)?;
            ledger.set_tenant_repo_root("tenant", &root)?;
        }
        let extra = roots.join("root-extra");
        std::fs::create_dir_all(&extra)?;
        assert!(matches!(
            ledger.set_tenant_repo_root("tenant", &extra),
            Err(super::ExecError::RepoRootLimit { limit: 64 })
        ));
        Ok(())
    }
}
