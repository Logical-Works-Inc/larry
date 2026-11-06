//! The executor behind `larry exec` (larry#85).

mod ledger;
mod run;
mod task;
mod workspace;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ledger::Ledger;
use run::run_command;
use workspace::{Workspace, sweep};

pub use task::{Task, TaskError};

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
    /// Isolation is not available.
    IsolationUnavailable,
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
}

impl fmt::Display for ExecError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Input(ref cause) => write!(out, "exec stdin unreadable: {cause}"),
            Self::Task(ref cause) => write!(out, "exec task: {cause}"),
            Self::IsolationUnavailable => out.write_str("exec isolation unavailable"),
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
            Self::IsolationUnavailable
            | Self::StateDirUnset
            | Self::BadTimeout { .. }
            | Self::Git { .. }
            | Self::NotARepo { .. }
            | Self::BadBase { .. }
            | Self::GroupSurvived { .. } => None,
        }
    }
}

/// No host fallback until Nova isolation lands (logicalworks-crates#846).
#[must_use]
pub const fn refuse_model_command() -> ExecError {
    ExecError::IsolationUnavailable
}

/// Settings the executor reads from its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// Directory holding the ledger and the workspaces.
    state_dir: PathBuf,
    /// Longest an acceptance command may run before its group is killed.
    timeout: Duration,
    /// The PATH the acceptance commands and git run under.
    path: String,
}

impl Settings {
    /// Reads the settings through `read`, which yields one variable or `None`.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Settings, ExecError> {
        let state_dir = read("LARRY_STATE_DIR")
            .filter(|value| !value.is_empty())
            .ok_or(ExecError::StateDirUnset)?;
        let timeout_ms = match read("LARRY_EXEC_TIMEOUT_MS") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => match value.parse::<u64>() {
                Ok(milliseconds) if (1..=MAX_TIMEOUT_MS).contains(&milliseconds) => milliseconds,
                _ => return Err(ExecError::BadTimeout { value }),
            },
        };
        // An absent PATH is the empty string: the optional value collects to nothing.
        let path: String = read("PATH").into_iter().collect();
        Ok(Settings {
            state_dir: PathBuf::from(state_dir),
            timeout: Duration::from_millis(timeout_ms),
            path,
        })
    }
}

/// Runs a task: clones its repository at the base, runs each acceptance command in order,
/// and returns the report with the staged diff.
pub fn execute(task: &Task, settings: &Settings) -> Result<lgwks_std::json::Value, ExecError> {
    let ledger = Ledger::open(&settings.state_dir)?;
    ledger.reap()?;
    sweep(&settings.state_dir, &ledger.live_tasks()?)?;
    let task_id = fresh_task_id(task.bytes());
    let workspace = Workspace::create(
        &settings.state_dir,
        task.repo(),
        task.base(),
        &task_id,
        &settings.path,
    )?;
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
    Ok(lgwks_std::json::json!({
        "task_id": task_id,
        "diff": diff,
        "results": results,
    }))
}

/// Names one run: the task's BLAKE3 prefix, the process id and a per-process counter.
fn fresh_task_id(bytes: &[u8]) -> String {
    static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(bytes);
    let digest = hasher.finalize().to_hex();
    let prefix: String = digest.chars().take(TASK_PREFIX_CHARS).collect();
    let run = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{run}", std::process::id())
}
