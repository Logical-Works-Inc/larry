//! The executor behind `larry exec` (larry#85), its signed receipts (larry#86)
//! and the model step that writes a brief's files before acceptance (larry#123).

mod author;
mod compact;
mod confine;
mod error;
pub(super) mod fence_control;
mod gate;
mod identity;
mod island;
mod ledger;
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
pub(crate) use ledger::Ledger;
use lgwks_std::sign::PublicKey;
pub(crate) use run::{Place, run_command};
use workspace::{Workspace, report_members, sweep};
pub(crate) use workspace::{hold_origin, origin_lock_path};

use crate::model::{ModelClient, Route, Selection};
pub use author::AuthorFault;
pub use confine::ConfineError;
#[cfg(test)]
pub(crate) use confine::Platform;
pub use error::ExecError;
pub(crate) use island::Isolation;
use receipt::parse_public_key;
pub use receipt::{MAX_REPORT_BYTES, ReceiptError, Verdict, verify};
pub use session::SessionError;
pub(crate) use session::{
    Approval, Handoff, MAX_INSTRUCTION_BYTES, Observer, SessionDoor, Stop, run_session,
};
pub(crate) use task::Knobs;
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

/// The public half of the state directory's receipt key, created on first use.
pub fn state_public_key(read: impl Fn(&str) -> Option<String>) -> Result<PublicKey, ExecError> {
    let directory = state_dir(&read)?;
    Ok(Ledger::open(&directory)?.signing_key()?.public_key())
}

/// Provisions the tenant-owned executor ledger before a local or remote door runs.
/// The policy text uses the same comma-separated scope names as `LARRY_POLICY`;
/// roots are canonicalized and bounded by the ledger owner.
pub fn provision_tenant(
    read: impl Fn(&str) -> Option<String>,
    policy_text: &str,
    roots: &[&std::path::Path],
) -> Result<(), ExecError> {
    let state = state_dir(&read)?;
    let ledger = Ledger::open(&state)?;
    let tenant = tenant_key(&state);
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

/// One sealed run report and the verdict its receipt carries.
#[derive(Debug)]
#[non_exhaustive]
pub struct Report {
    /// The report with its `accepted` verdict and `receipt`.
    json: lgwks_std::json::Value,
    /// Whether the run was accepted.
    accepted: bool,
    /// The sealed report exactly as it was printed, so a retry answers with the
    /// bytes the ledger recorded rather than a re-rendering of them.
    bytes: Vec<u8>,
}

impl Report {
    /// The sealed report, as printed.
    #[must_use]
    pub const fn json(&self) -> &lgwks_std::json::Value {
        &self.json
    }

    /// The sealed report's exact bytes, the value the ledger keeps.
    #[must_use]
    pub(crate) fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Whether the run was accepted: no denial, a diff, every command passed.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        self.accepted
    }

    /// Whether this report carries a seal at all: one the gate could not make
    /// durable is never a record a retry may answer from (larry#182).
    #[must_use]
    pub(crate) fn carries_a_seal(&self) -> bool {
        self.json.get(receipt::RECEIPT).is_some()
    }

    /// Seals a report body under `key`.
    fn sealed(body: lgwks_std::json::Value, key: &lgwks_std::sign::SecretKey) -> Report {
        let (json, accepted) = receipt::seal(body, key);
        let bytes = json.to_string().into_bytes();
        Report {
            json,
            accepted,
            bytes,
        }
    }

    /// The report a ledger recorded, re-derived rather than trusted: its seal is
    /// checked under this state directory's key and its verdict re-derived, so a
    /// retry never answers with a record this state directory did not seal
    /// (larry#182 defect 3).
    ///
    /// # Errors
    ///
    /// [`ExecError::Receipt`] when the recorded bytes are not a report this
    /// directory's key seals.
    pub(crate) fn parse(
        bytes: &[u8],
        key: &lgwks_std::sign::SecretKey,
    ) -> Result<Report, ExecError> {
        let verdict = receipt::verify(bytes, &key.public_key()).map_err(ExecError::Receipt)?;
        let json = lgwks_std::json::from_slice(bytes)
            .map_err(|cause| ExecError::Receipt(ReceiptError::Malformed(cause)))?;
        Ok(Report {
            json,
            accepted: verdict.accepted(),
            bytes: bytes.to_vec(),
        })
    }
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
    /// Longest an acceptance command may run before its group is killed.
    timeout: Duration,
    /// The PATH the acceptance commands and git run under.
    path: String,
    /// The scopes the gate authorizes tasks against. Local settings read
    /// `LARRY_POLICY`; remote settings read the tenant ledger.
    policy: BTreeSet<Scope>,
    /// Whether this door is local (`larry exec`, bare `larry`): with no
    /// provisioned root row it allows any absolute path outside the state
    /// directory, while a remote door denies. Set once in `from_env_mode`.
    local: bool,
    /// Where a task's route reads its key and the `LARRY_MODEL_*` knobs from.
    read: EnvReader,
    /// Optional host authority action for the next admitted task.
    pub(crate) fence_control: Option<fence_control::FenceControl>,
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

    /// Reads the common settings and selects the policy owner for the door.
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
        let policy = if local {
            gate::policy(&read)?
        } else {
            Ledger::open(&state_dir)?.tenant_policy(&tenant_key(&state_dir))?
        };
        let fence_control = fence_control::read(&read)?;
        Ok(Settings {
            state_dir,
            timeout: Duration::from_millis(timeout_ms),
            path,
            policy,
            local,
            read: Arc::new(read),
            fence_control,
        })
    }

    /// Whether this door is local: remote doors deny a tenant with no root row.
    pub(crate) const fn is_local(&self) -> bool {
        self.local
    }

    /// The directory holding the ledger, the workspaces and `larry eval`'s origins.
    pub(crate) fn state_dir(&self) -> &std::path::Path {
        self.state_dir.as_path()
    }

    /// Refreshes the remote door's policy from the durable tenant ledger. The process may
    /// stay open while an operator provisions or withdraws a tenant, so a cached snapshot
    /// must not outlive the prompt that uses it.
    pub(crate) fn refresh_remote_policy(&mut self) -> Result<(), ExecError> {
        self.policy = Ledger::open(&self.state_dir)?.tenant_policy(&tenant_key(&self.state_dir))?;
        Ok(())
    }

    /// The `PATH` acceptance commands and git run under.
    pub(crate) fn path_env(&self) -> &str {
        self.path.as_str()
    }

    /// Longest an acceptance command may run before its group is killed.
    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
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
/// with the staged diff, sealed under the state directory's key. A task asking for model
/// commands is denied: nothing runs, and its sealed report is never accepted. Every other
/// task runs only after the admission gate authorized it (larry#164): the task body is the
/// runtime's one dispatched effect, and a policy refusal seals a report that ran nothing.
pub fn execute(task: &Task, settings: &Settings) -> Result<Report, ExecError> {
    let (ledger, key) = prepare(settings)?;
    let repository = ledger.authorized_repo(
        &tenant_key(&settings.state_dir),
        &settings.state_dir,
        task.repo(),
        settings.is_local(),
    )?;
    execute_prepared(task, settings, &ledger, &key, &repository)
}

/// Runs an evaluator-owned origin. Public task execution refuses every state-directory path;
/// this crate-private route is narrower still, accepting only a canonical origin beneath the
/// evaluator's own `<state>/eval` directory.
pub(crate) fn execute_eval(task: &Task, settings: &Settings) -> Result<Report, ExecError> {
    let (ledger, key) = prepare(settings)?;
    let repository = Ledger::authorized_eval_repo(settings.state_dir(), task.repo())?;
    execute_prepared(task, settings, &ledger, &key, &repository)
}

/// Executes after the caller has supplied a repository authorized for this route.
fn execute_prepared(
    task: &Task,
    settings: &Settings,
    ledger: &Ledger,
    key: &lgwks_std::sign::SecretKey,
    repository: &std::path::Path,
) -> Result<Report, ExecError> {
    let task_id = identity::run_id(task, &settings.state_dir)?;
    let task_digest = digest_hex(task.bytes());
    // A task that names an idempotency key is the client's statement that a retry
    // of it is the same request, so a retry whose record the ledger already holds
    // is answered from that record and the body never runs a second time
    // (larry#182 defect 3).
    let resumable = task.idempotency_key().is_some();
    if resumable && let Some(earlier) = ledger.report(&task_id, MAX_REPORT_BYTES)? {
        return Report::parse(&earlier, key);
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
            key,
        );
        if resumable {
            ledger.keep_report(&task_id, &task_digest, report.bytes())?;
        }
        return Ok(report);
    }
    let subject = gate::Subject {
        required: gate::required_scopes(task),
        path: repository,
        bytes: task.bytes(),
    };
    let report = gate::admit_and_run(subject, settings, &task_id, &task_digest, key, || {
        run_task(
            task,
            repository,
            settings,
            &task_id,
            &task_digest,
            ledger,
            key,
        )
    })?;
    if resumable && report.carries_a_seal() {
        ledger.keep_report(&task_id, &task_digest, report.bytes())?;
    }
    Ok(report)
}

/// What every run does first: opens the ledger and its receipt key, reaps the process
/// groups a killed executor left, sweeps dead workspaces, and settles every journal a
/// killed executor left before anything new runs (larry#75). A run is live while a
/// process holds its journal's lock, so no sweep and no recovery reaps a concurrent
/// run's evidence, and neither asks for a process id (larry#182 defect 3).
fn prepare(settings: &Settings) -> Result<(Ledger, lgwks_std::sign::SecretKey), ExecError> {
    let ledger = Ledger::open(&settings.state_dir)?;
    let key = ledger.signing_key()?;
    // The one clock the reap's group waits read, built the way the gate builds its own, so a
    // host whose clock reads before the epoch refuses with nothing reaped (larry#268, larry#308).
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let state = settings.state_dir.as_path();
    ledger.reap(&clock, |task| recovery::run_held(state, task))?;
    let policy = &settings.policy;
    for fault in sweep(&settings.state_dir, &ledger.live_tasks()?, |task| {
        recovery::journal_live(state, policy, task)
    })? {
        fault.report();
    }
    recovery::recover(&settings.state_dir, &settings.policy)?;
    Ok((ledger, key))
}

/// What a task with a brief needs before anything is cloned: the route its model
/// calls go to, how it was chosen for the sealed record, and a client built
/// from it.
struct ModelStep<'task> {
    /// The route the task resolved to; the sealed report records it.
    route: Route,
    /// How the route was chosen; the sealed report records it beside the route.
    selection: Selection,
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
            routes::task_route(settings, ledger, Some(named))?;
        }
        return Ok(None);
    };
    let selection = routes::task_select(settings, ledger, task)?;
    let route = selection.route().clone();
    // The one timeline the retry budget reads, built the way the gate builds its own (larry#436).
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let client = ModelClient::new(routes::task_model_config(settings, &route)?, clock, backoff);
    Ok(Some(ModelStep {
        route,
        selection,
        brief,
        client,
    }))
}

/// The task body the gate authorizes: clones the repository at the base, has
/// the model write the brief's files when the task carries one, runs each
/// acceptance command in order, and returns the report with the staged diff,
/// sealed under the state directory's key. It runs at most once, inside the
/// gate's bound owner reach.
fn run_task(
    task: &Task,
    repository: &std::path::Path,
    settings: &Settings,
    task_id: &str,
    task_digest: &str,
    ledger: &Ledger,
    key: &lgwks_std::sign::SecretKey,
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
    let step = model_step(task, settings, ledger)?;
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
            selection,
            brief,
            client,
        }) => {
            // The tenant's digest, the same text a route row is keyed by, names the
            // cache partition so no two tenants share a prompt prefix (larry#230).
            let partition = digest_hex(identity::tenant_of(&settings.state_dir).as_bytes());
            let mut records = author::author(brief, workspace.path(), &client, &partition)?;
            for record in &mut records {
                if let Some(object) = record.as_object_mut() {
                    object.insert(String::from("route"), route.seal());
                    object.insert(String::from("route_selection"), selection.seal());
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
        // so only the timeout ends it (larry#242). A task that names a shell runs
        // each command under it as `<shell> -c <command>` (larry#439); a task that
        // names a timeout runs under it, cut to the default when it names more.
        let timeout = match task.timeout_ms() {
            Some(millis) => Duration::from_millis(millis).min(settings.timeout),
            None => settings.timeout,
        };
        let place = match confine {
            Some(ref confine) => Place::Confined(confine),
            None => Place::Open(workspace.path()),
        };
        let result = run_command(
            &task.argv_for(argv),
            place,
            &settings.path,
            timeout,
            ledger,
            task_id,
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
    Ok(Report::sealed(body, key))
}

/// The BLAKE3 digest of `bytes`, as 64 hex characters.
fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex()
}

/// The stable text key shared by the executor's tenant-owned ledger tables.
fn tenant_key(state: &std::path::Path) -> String {
    digest_hex(identity::tenant_of(state).as_bytes())
}

#[cfg(test)]
mod issue207_tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use larry_contracts::authority::Scope;
    use lgwks_std::json::json;

    use super::sim_tree::{Origin, Shape, origin, scratch, settings};
    use super::{Ledger, Settings, Task, execute, tenant_key};

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
        ledger_a.set_tenant_policy(&tenant_key(&state_a), &allowed)?;
        let ledger_b = Ledger::open(&state_b)?;
        ledger_b.set_tenant_repo_root(&tenant_key(&state_b), &root)?;
        ledger_b.set_tenant_policy(&tenant_key(&state_b), &BTreeSet::new())?;
        let same = task(&repository)?;
        let first = execute(&same, &remote_settings(&state_a)?)?;
        let second = execute(&same, &remote_settings(&state_b)?)?;
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
