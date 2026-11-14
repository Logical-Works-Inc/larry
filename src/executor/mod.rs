//! The executor behind `larry exec` (larry#85), its signed receipts (larry#86)
//! and the model step that writes a brief's files before acceptance (larry#123).

mod author;
mod gate;
mod identity;
mod ledger;
mod receipt;
mod recovery;
pub mod routes;
mod run;
mod session;
mod task;
mod workspace;

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub(crate) use author::is_writable_path;
use larry_contracts::authority::Scope;
use larry_runtime::error::RuntimeError;
pub(crate) use ledger::Ledger;
use lgwks_std::sign::PublicKey;
use run::run_command;
use workspace::{Workspace, report_members, sweep};
pub(crate) use workspace::{hold_origin, origin_lock_path};

use crate::model::{ModelClient, ModelConfig, ModelError, Route, RouteError};
pub use author::AuthorFault;
use receipt::parse_public_key;
pub use receipt::{MAX_REPORT_BYTES, ReceiptError, Verdict, verify};
pub use session::SessionError;
pub(crate) use session::{Observer, run_session};
pub use task::{Task, TaskError};

/// The denial a task asking for model commands receives until Nova isolation
/// lands (logicalworks-crates#846): nothing runs, and the signed report says so.
const MODEL_COMMANDS_DENIED: &str =
    "model_commands: isolation unavailable until Nova lands (logicalworks-crates#846)";

/// Default longest an acceptance command may run, in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 600_000;

/// Largest accepted command timeout, in milliseconds (one day).
const MAX_TIMEOUT_MS: u64 = 86_400_000;

/// Why `larry exec` failed; each variant keeps its cause.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExecError {
    /// Standard input could not be read.
    Input(std::io::Error),
    /// The task document was refused.
    Task(TaskError),
    /// Standard output could not be written.
    Output(std::io::Error),
    /// `LARRY_STATE_DIR` is unset or empty.
    StateDirUnset,
    /// `LARRY_EXEC_TIMEOUT_MS` is not a whole number of milliseconds in 1..=86400000.
    BadTimeout {
        /// The value that was refused.
        value: String,
    },
    /// `LARRY_POLICY` names a scope the executor does not know.
    BadPolicy {
        /// The name that was refused.
        value: String,
    },
    /// The sqlite ledger could not be opened or migrated.
    Ledger(lgwks_deps::sqlite::SqliteError),
    /// The state directory or the lock that serializes ledger opens failed.
    LedgerLock(std::io::Error),
    /// A ledger statement failed.
    Sql(lgwks_deps::storefront::rusqlite::Error),
    /// A process group could not be signalled, probed or reaped.
    Reap(std::io::Error),
    /// Another executor held the ledger lock for the whole wait, so this open gave up
    /// rather than wait forever (larry#214).
    LedgerLockTimeout {
        /// How long this open waited before it gave up.
        waited: std::time::Duration,
    },
    /// An acceptance command could not be started, waited on or read.
    Spawn {
        /// The program that failed.
        argv0: String,
        /// The cause.
        source: std::io::Error,
    },
    /// A git step in the workspace failed.
    Git {
        /// The git step that failed.
        step: &'static str,
        /// Git's stderr, trimmed.
        detail: String,
    },
    /// The repository could not be cloned.
    NotARepo {
        /// The repository path that was refused.
        repo: PathBuf,
    },
    /// The base commit-ish could not be checked out.
    BadBase {
        /// The base that was refused.
        base: String,
    },
    /// A process group outlived its leader and did not clear.
    GroupSurvived {
        /// The surviving process group.
        pgid: i32,
    },
    /// The workspace directory could not be created, read or removed.
    Workspace(std::io::Error),
    /// The store holding the receipt key could not be created or made owner-only.
    KeyStore(std::io::Error),
    /// The OS refused entropy for a new receipt key; there is no weaker fallback.
    Entropy(lgwks_std::random::EntropyError),
    /// A run id could not be minted: the wall clock or the OS entropy refused, and
    /// no value is substituted for it (larry#182 defect 3).
    Id(lgwks_std::id::V7Error),
    /// The receipt key row is missing right after its insert.
    KeyMissing,
    /// The stored receipt key seed is not 32 bytes.
    KeySeedLength(std::array::TryFromSliceError),
    /// A receipt was refused.
    Receipt(ReceiptError),
    /// A `LARRY_MODEL_*` setting is unusable, or the variable a route names for
    /// its key is unset.
    ModelConfig(ModelError),
    /// The route a task's model calls would use is unknown, absent or damaged
    /// (larry#201): nothing was sent.
    Route(RouteError),
    /// The model step could not write one of the brief's files; nothing was sealed.
    Author {
        /// The repo-relative path being written.
        path: String,
        /// Why it could not be written.
        fault: AuthorFault,
    },
    /// The admission gate refused to run the task.
    Gate(RuntimeError),
    /// The task body ran and its report was sealed, but the runtime could not
    /// make the run's record durable, so the sealed report is not returned
    /// (larry#182). A sealed report is only a receipt for work whose record
    /// landed, and a tick that failed its sync proves the record did not.
    Unsealed(RuntimeError),
    /// The task journals directory could not be created, listed or tidied (larry#75).
    Journals(std::io::Error),
    /// A terminal session was refused before anything ran (larry#93).
    Session(SessionError),
}

impl fmt::Display for ExecError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Input(ref cause) => write!(out, "exec stdin unreadable: {cause}"),
            Self::Task(ref cause) => write!(out, "exec task: {cause}"),
            Self::Output(ref cause) => write!(out, "exec output unwritable: {cause}"),
            Self::StateDirUnset => out.write_str("exec LARRY_STATE_DIR is unset or empty"),
            Self::BadTimeout { ref value } => write!(
                out,
                "exec LARRY_EXEC_TIMEOUT_MS is not a timeout in 1..=86400000: {value}"
            ),
            Self::BadPolicy { ref value } => {
                write!(out, "exec LARRY_POLICY is not a scope name: {value}")
            }
            Self::Ledger(ref cause) => write!(out, "exec ledger: {cause}"),
            Self::LedgerLock(ref cause) => write!(out, "exec ledger lock: {cause}"),
            Self::Sql(ref cause) => write!(out, "exec ledger statement: {cause}"),
            Self::Reap(ref cause) => write!(out, "exec process group: {cause}"),
            Self::LedgerLockTimeout { waited } => write!(
                out,
                "exec ledger lock held by another executor for {waited:?}; retry later"
            ),
            Self::Spawn {
                ref argv0,
                ref source,
            } => write!(out, "exec could not run {argv0}: {source}"),
            Self::Git { step, ref detail } => write!(out, "exec git {step} failed: {detail}"),
            Self::NotARepo { ref repo } => {
                write!(out, "exec could not clone {}", repo.display())
            }
            Self::BadBase { ref base } => write!(out, "exec could not check out base {base}"),
            Self::GroupSurvived { pgid } => {
                write!(out, "exec process group {pgid} survived its kill")
            }
            Self::Workspace(ref cause) => write!(out, "exec workspace: {cause}"),
            Self::KeyStore(ref cause) => write!(out, "exec receipt key store: {cause}"),
            Self::Entropy(ref cause) => write!(out, "exec receipt key entropy: {cause}"),
            Self::Id(ref cause) => write!(out, "exec run id: {cause}"),
            Self::KeyMissing => out.write_str("exec receipt key row is missing after its insert"),
            Self::KeySeedLength(ref cause) => {
                write!(out, "exec receipt key seed is not 32 bytes: {cause}")
            }
            Self::Receipt(ref cause) => write!(out, "verify: {cause}"),
            Self::ModelConfig(ref cause) => write!(out, "exec model config: {cause}"),
            Self::Route(ref cause) => write!(out, "exec: {cause}"),
            Self::Author {
                ref path,
                ref fault,
            } => write!(out, "exec could not write {path}: {fault}"),
            Self::Gate(ref cause) => write!(out, "exec gate: {cause}"),
            Self::Unsealed(ref cause) => write!(
                out,
                "exec run finished but its record is not durable, so no report is returned: {cause}"
            ),
            Self::Journals(ref cause) => write!(out, "exec journals: {cause}"),
            Self::Session(ref cause) => write!(out, "session: {cause}"),
        }
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Input(ref cause) => Some(cause),
            Self::Task(ref cause) => Some(cause),
            Self::Output(ref cause) => Some(cause),
            Self::Ledger(ref cause) => Some(cause),
            Self::LedgerLock(ref cause) => Some(cause),
            Self::Sql(ref cause) => Some(cause),
            Self::Reap(ref cause) => Some(cause),
            Self::Spawn { ref source, .. } => Some(source),
            Self::Workspace(ref cause) => Some(cause),
            Self::KeyStore(ref cause) => Some(cause),
            Self::Entropy(ref cause) => Some(cause),
            Self::Id(ref cause) => Some(cause),
            Self::Receipt(ref cause) => Some(cause),
            Self::KeySeedLength(ref cause) => Some(cause),
            Self::ModelConfig(ref cause) => Some(cause),
            Self::Route(ref cause) => Some(cause),
            Self::Author { ref fault, .. } => Some(fault),
            Self::Gate(ref cause) => Some(cause),
            Self::Unsealed(ref cause) => Some(cause),
            Self::Journals(ref cause) => Some(cause),
            Self::Session(ref cause) => Some(cause),
            Self::KeyMissing
            | Self::StateDirUnset
            | Self::BadTimeout { .. }
            | Self::BadPolicy { .. }
            | Self::Git { .. }
            | Self::NotARepo { .. }
            | Self::BadBase { .. }
            | Self::GroupSurvived { .. }
            | Self::LedgerLockTimeout { .. } => None,
        }
    }
}

impl From<RuntimeError> for ExecError {
    fn from(cause: RuntimeError) -> Self {
        Self::Gate(cause)
    }
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
    /// The scopes `LARRY_POLICY` admits; what the gate authorizes tasks against.
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
        let state_dir = state_dir(&read)?;
        let timeout_ms = match read("LARRY_EXEC_TIMEOUT_MS") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => match value.parse::<u64>() {
                Ok(milliseconds) if (1..=MAX_TIMEOUT_MS).contains(&milliseconds) => milliseconds,
                _ => return Err(ExecError::BadTimeout { value }),
            },
        };
        // An absent PATH is the empty string: the optional value collects to nothing.
        let path: String = read("PATH").into_iter().collect();
        let policy = gate::policy(&read)?;
        Ok(Settings {
            state_dir,
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

    /// Removes the origins a dead `larry eval` left under the state directory.
    pub(crate) fn sweep_origins(&self) -> Result<(), ExecError> {
        workspace::sweep_origins(&self.state_dir)
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
    let task_id = identity::run_id(task, &settings.state_dir)?;
    let task_digest = digest_hex(task.bytes());
    // A task that names an idempotency key is the client's statement that a retry
    // of it is the same request, so a retry whose record the ledger already holds
    // is answered from that record and the body never runs a second time
    // (larry#182 defect 3).
    let resumable = task.idempotency_key().is_some();
    if resumable && let Some(earlier) = ledger.report(&task_id, MAX_REPORT_BYTES)? {
        return Report::parse(&earlier, &key);
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
            &key,
        );
        if resumable {
            ledger.keep_report(&task_id, &task_digest, report.bytes())?;
        }
        return Ok(report);
    }
    let subject = gate::Subject {
        required: gate::required_scopes(task),
        path: task.repo(),
        bytes: task.bytes(),
    };
    let report = gate::admit_and_run(subject, settings, &task_id, &task_digest, &key, || {
        run_task(task, settings, &task_id, &task_digest, &ledger, &key)
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
    ledger.reap()?;
    let state = settings.state_dir.as_path();
    let policy = &settings.policy;
    sweep(&settings.state_dir, &ledger.live_tasks()?, |task| {
        recovery::journal_live(state, policy, task)
    })?;
    recovery::recover(&settings.state_dir, &settings.policy)?;
    Ok((ledger, key))
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
    let client = ModelClient::new(settings.model_config(&route)?, backoff);
    Ok(Some(ModelStep {
        route,
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
    settings: &Settings,
    task_id: &str,
    task_digest: &str,
    ledger: &Ledger,
    key: &lgwks_std::sign::SecretKey,
) -> Result<Report, ExecError> {
    let step = model_step(task, settings, ledger)?;
    let workspace = Workspace::create(
        &settings.state_dir,
        task.repo(),
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
            let mut records = author::author(brief, workspace.path(), &client)?;
            for record in &mut records {
                if let Some(object) = record.as_object_mut() {
                    object.insert(String::from("route"), route.seal());
                }
            }
            Some(records)
        }
    };
    let mut results = Vec::with_capacity(task.acceptance().len());
    for argv in task.acceptance() {
        let result = run_command(
            argv,
            workspace.path(),
            &settings.path,
            settings.timeout,
            ledger,
            task_id,
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
