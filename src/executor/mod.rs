//! The executor behind `larry exec` (larry#85), its signed receipts (larry#86)
//! and the model step that writes a brief's files before acceptance (larry#123).

mod author;
mod ledger;
mod receipt;
mod run;
mod task;
mod workspace;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) use author::is_writable_path;
use ledger::Ledger;
use lgwks_std::sign::PublicKey;
use run::run_command;
use workspace::{Workspace, sweep};

use crate::model::{ModelClient, ModelConfig, ModelError};
pub use author::AuthorFault;
use receipt::parse_public_key;
pub use receipt::{MAX_REPORT_BYTES, ReceiptError, Verdict, verify};
pub use task::{Task, TaskError};

/// The denial a task asking for model commands receives until Nova isolation
/// lands (logicalworks-crates#846): nothing runs, and the signed report says so.
const MODEL_COMMANDS_DENIED: &str =
    "model_commands: isolation unavailable until Nova lands (logicalworks-crates#846)";

/// Default longest an acceptance command may run, in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 600_000;

/// Largest accepted command timeout, in milliseconds (one day).
const MAX_TIMEOUT_MS: u64 = 86_400_000;

/// Characters of the task's BLAKE3 digest that name a run.
const TASK_PREFIX_CHARS: usize = 16;

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
    /// The sqlite ledger could not be opened or migrated.
    Ledger(lgwks_deps::sqlite::SqliteError),
    /// The state directory or the lock that serializes ledger opens failed.
    LedgerLock(std::io::Error),
    /// A ledger statement failed.
    Sql(lgwks_deps::storefront::rusqlite::Error),
    /// A process group could not be signalled, probed or reaped.
    Reap(std::io::Error),
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
    /// The receipt key row is missing right after its insert.
    KeyMissing,
    /// The stored receipt key seed is not 32 bytes.
    KeySeedLength(std::array::TryFromSliceError),
    /// A receipt was refused.
    Receipt(ReceiptError),
    /// A `LARRY_MODEL_*` setting is unusable.
    ModelConfig(ModelError),
    /// The task carries a brief and no `LARRY_MODEL_KEY` is set, so no model can write it.
    ModelUnconfigured,
    /// The model step could not write one of the brief's files; nothing was sealed.
    Author {
        /// The repo-relative path being written.
        path: String,
        /// Why it could not be written.
        fault: AuthorFault,
    },
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
            Self::Ledger(ref cause) => write!(out, "exec ledger: {cause}"),
            Self::LedgerLock(ref cause) => write!(out, "exec ledger lock: {cause}"),
            Self::Sql(ref cause) => write!(out, "exec ledger statement: {cause}"),
            Self::Reap(ref cause) => write!(out, "exec process group: {cause}"),
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
            Self::KeyMissing => out.write_str("exec receipt key row is missing after its insert"),
            Self::KeySeedLength(ref cause) => {
                write!(out, "exec receipt key seed is not 32 bytes: {cause}")
            }
            Self::Receipt(ref cause) => write!(out, "verify: {cause}"),
            Self::ModelConfig(ref cause) => write!(out, "exec model config: {cause}"),
            Self::ModelUnconfigured => {
                out.write_str("exec LARRY_MODEL_KEY is unset: a task with a goal needs the model")
            }
            Self::Author {
                ref path,
                ref fault,
            } => write!(out, "exec could not write {path}: {fault}"),
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
            Self::Receipt(ref cause) => Some(cause),
            Self::KeySeedLength(ref cause) => Some(cause),
            Self::ModelConfig(ref cause) => Some(cause),
            Self::Author { ref fault, .. } => Some(fault),
            Self::KeyMissing
            | Self::ModelUnconfigured
            | Self::StateDirUnset
            | Self::BadTimeout { .. }
            | Self::Git { .. }
            | Self::NotARepo { .. }
            | Self::BadBase { .. }
            | Self::GroupSurvived { .. } => None,
        }
    }
}

/// The non-empty `LARRY_STATE_DIR` that `read` yields.
fn state_dir(read: &impl Fn(&str) -> Option<String>) -> Result<PathBuf, ExecError> {
    read("LARRY_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(ExecError::StateDirUnset)
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
}

impl Report {
    /// The sealed report, as printed.
    #[must_use]
    pub const fn json(&self) -> &lgwks_std::json::Value {
        &self.json
    }

    /// Whether the run was accepted: no denial, a diff, every command passed.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        self.accepted
    }

    /// Seals a report body under `key`.
    fn sealed(body: lgwks_std::json::Value, key: &lgwks_std::sign::SecretKey) -> Report {
        let (json, accepted) = receipt::seal(body, key);
        Report { json, accepted }
    }
}

/// Settings the executor reads from its environment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settings {
    /// Directory holding the ledger and the workspaces.
    state_dir: PathBuf,
    /// Longest an acceptance command may run before its group is killed.
    timeout: Duration,
    /// The PATH the acceptance commands and git run under.
    path: String,
    /// The gateway a brief's files are written through; `None` without `LARRY_MODEL_KEY`.
    model: Option<ModelConfig>,
}

impl Settings {
    /// Reads the settings through `read`, which yields one variable or `None`.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Settings, ExecError> {
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
        let model = match read("LARRY_MODEL_KEY").filter(|key| !key.is_empty()) {
            None => None,
            Some(_) => Some(ModelConfig::from_env(&read).map_err(ExecError::ModelConfig)?),
        };
        Ok(Settings {
            state_dir,
            timeout: Duration::from_millis(timeout_ms),
            path,
            model,
        })
    }

    /// The directory holding the ledger, the workspaces and `larry eval`'s origins.
    pub(crate) fn state_dir(&self) -> &std::path::Path {
        self.state_dir.as_path()
    }

    /// The pinned model a brief's files are written by, when a key is configured.
    pub(crate) fn model_name(&self) -> Option<&str> {
        self.model.as_ref().map(ModelConfig::model)
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
/// commands is denied: nothing runs, and its sealed report is never accepted.
pub fn execute(task: &Task, settings: &Settings) -> Result<Report, ExecError> {
    let ledger = Ledger::open(&settings.state_dir)?;
    let key = ledger.signing_key()?;
    ledger.reap()?;
    sweep(&settings.state_dir, &ledger.live_tasks()?)?;
    let task_id = fresh_task_id(task.bytes());
    let task_digest = digest_hex(task.bytes());
    if task.has_model_commands() {
        return Ok(Report::sealed(
            lgwks_std::json::json!({
                "task_id": task_id,
                "task_digest": task_digest,
                "diff": null,
                "results": [],
                "denied": [MODEL_COMMANDS_DENIED],
            }),
            &key,
        ));
    }
    let author = match task.brief() {
        None => None,
        Some(brief) => {
            let config = settings.model.clone().ok_or(ExecError::ModelUnconfigured)?;
            Some((brief, ModelClient::new(config, backoff)))
        }
    };
    let workspace = Workspace::create(
        &settings.state_dir,
        task.repo(),
        task.base(),
        &task_id,
        &settings.path,
    )?;
    let authored = match author {
        None => None,
        Some((brief, client)) => Some(author::author(brief, workspace.path(), &client)?),
    };
    let mut results = Vec::with_capacity(task.acceptance().len());
    for argv in task.acceptance() {
        let result = run_command(
            argv,
            workspace.path(),
            &settings.path,
            settings.timeout,
            &ledger,
            &task_id,
        )?;
        results.push(result.into_json());
    }
    let diff = workspace.diff(task.base())?;
    let mut body = lgwks_std::json::json!({
        "task_id": task_id,
        "task_digest": task_digest,
        "diff": diff,
        "results": results,
        "denied": [],
    });
    // Only a briefed run names what the model wrote, so a plain run's report keeps its shape.
    if let Some(records) = authored
        && let Some(object) = body.as_object_mut()
    {
        object.insert(
            String::from("authored"),
            lgwks_std::json::Value::Array(records),
        );
    }
    Ok(Report::sealed(body, &key))
}

/// The BLAKE3 digest of `bytes`, as 64 hex characters.
fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex()
}

/// Names one run: the task's BLAKE3 prefix, the process id and a per-process counter.
fn fresh_task_id(bytes: &[u8]) -> String {
    static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);
    let digest = digest_hex(bytes);
    let prefix: String = digest.chars().take(TASK_PREFIX_CHARS).collect();
    let run = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{run}", std::process::id())
}
