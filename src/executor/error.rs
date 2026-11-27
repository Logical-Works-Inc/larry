//! Why `larry exec` failed: the one error type of the executor, its message and its source
//! chain. Its own module because the executor's run and this type are two jobs (larry#398).

use std::path::PathBuf;

use larry_runtime::error::RuntimeError;
use lgwks_macros::Error;

use super::{AuthorFault, ConfineError, ReceiptError, SessionError, TaskError};
use crate::model::{ModelError, RouteError};

/// Why `larry exec` failed; each variant keeps its cause.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExecError {
    /// Standard input could not be read.
    #[error("exec stdin unreadable: {0}")]
    Input(#[source] std::io::Error),
    /// The task document was refused.
    #[error("exec task: {0}")]
    Task(#[source] TaskError),
    /// Standard output could not be written.
    #[error("exec output unwritable: {0}")]
    Output(#[source] std::io::Error),
    /// `LARRY_STATE_DIR` is unset or empty.
    #[error("exec LARRY_STATE_DIR is unset or empty")]
    StateDirUnset,
    /// `PATH` is unset: no command is looked up under an empty path (larry#213).
    #[error("exec PATH is unset; set it to the directories the commands are found in")]
    PathUnset,
    /// `LARRY_POLICY` is required when an operator provisions a remote tenant.
    #[error("exec LARRY_POLICY is unset; set it before provisioning a tenant")]
    PolicyUnset,
    /// `LARRY_EXEC_TIMEOUT_MS` is not a whole number of milliseconds in 1..=86400000.
    #[error("exec LARRY_EXEC_TIMEOUT_MS is not a timeout in 1..=86400000: {value}")]
    BadTimeout {
        /// The value that was refused.
        value: String,
    },
    /// `LARRY_POLICY` names a scope the executor does not know.
    #[error("exec LARRY_POLICY is not a scope name: {value}")]
    BadPolicy {
        /// The name that was refused.
        value: String,
    },
    /// The sqlite ledger could not be opened or migrated.
    #[error("exec ledger: {0}")]
    Ledger(#[source] lgwks_deps::sqlite::SqliteError),
    /// The state directory could not be made.
    #[error("exec state directory: {0}")]
    StateDir(#[source] std::io::Error),
    /// A ledger statement failed.
    #[error("exec ledger statement: {0}")]
    Sql(#[source] lgwks_deps::storefront::rusqlite::Error),
    /// A process group could not be signalled, probed or reaped.
    #[error("exec process group: {0}")]
    Reap(#[source] std::io::Error),
    /// An acceptance command could not be started, waited on or read.
    #[error("exec could not run {argv0}: {source}")]
    Spawn {
        /// The program that failed.
        argv0: String,
        /// The cause.
        source: std::io::Error,
    },
    /// A git step in the workspace failed.
    #[error("exec git {step} failed: {detail}")]
    Git {
        /// The git step that failed.
        step: &'static str,
        /// Git's stderr, trimmed.
        detail: String,
    },
    /// The repository path could not be canonicalized before authorization.
    #[error("exec repository path could not be read {}: {cause}", repo.display())]
    RepoPath {
        /// The repository path that was refused.
        repo: PathBuf,
        /// The filesystem refusal.
        #[source]
        cause: std::io::Error,
    },
    /// The repository is inside the executor's state directory.
    #[error("exec repository is inside the state directory: {}", repo.display())]
    RepoInsideState {
        /// The canonical repository path.
        repo: PathBuf,
    },
    /// The repository is not below one of this tenant's durable roots.
    #[error("exec repository is not in the tenant allowlist: {}", repo.display())]
    RepoNotAllowed {
        /// The canonical repository path.
        repo: PathBuf,
    },
    /// A configured repository root could not be canonicalized.
    #[error("exec repository root could not be read {}: {cause}", repo.display())]
    RepoRoot {
        /// The root path that was refused.
        repo: PathBuf,
        /// The filesystem refusal.
        #[source]
        cause: std::io::Error,
    },
    /// A repository root was not absolute.
    #[error("exec repository root is not absolute: {repo}")]
    RelativeRepoRoot {
        /// The root path that was refused.
        repo: String,
    },
    /// A configured repository root is already at the per-tenant bound.
    #[error("exec tenant repository allowlist is full ({limit} roots)")]
    RepoRootLimit {
        /// The maximum roots per tenant.
        limit: usize,
    },
    /// The repository could not be cloned.
    #[error("exec could not clone {}", repo.display())]
    NotARepo {
        /// The repository path that was refused.
        repo: PathBuf,
    },
    /// The base commit-ish could not be checked out.
    #[error("exec could not check out base {base}")]
    BadBase {
        /// The base that was refused.
        base: String,
    },
    /// A process group outlived its leader and did not clear.
    #[error("exec process group {pgid} survived its kill")]
    GroupSurvived {
        /// The surviving process group.
        pgid: i32,
    },
    /// The workspace directory could not be created, read or removed.
    #[error("exec workspace: {0}")]
    Workspace(#[source] std::io::Error),
    /// A child or group could not be ended after an earlier refusal. The earlier refusal
    /// stays the source, so the operator sees both causes and neither is flattened into a
    /// string (larry#343).
    #[error("{cause}; {step}: {fault}")]
    Cleanup {
        /// The refusal the cleanup ran after.
        #[source]
        cause: Box<ExecError>,
        /// The cleanup step that failed, as the operator reads it.
        step: &'static str,
        /// Why that step failed.
        fault: Box<ExecError>,
    },
    /// The store holding the receipt key could not be created or made owner-only.
    #[error("exec receipt key store: {0}")]
    KeyStore(#[source] std::io::Error),
    /// The OS refused entropy for a new receipt key; there is no weaker fallback.
    #[error("exec receipt key entropy: {0}")]
    Entropy(#[source] lgwks_std::random::EntropyError),
    /// A run id could not be minted: the wall clock or the OS entropy refused, and
    /// no value is substituted for it (larry#182 defect 3).
    #[error("exec run id: {0}")]
    Id(#[source] lgwks_std::id::V7Error),
    /// The receipt key row is missing right after its insert.
    #[error("exec receipt key row is missing after its insert")]
    KeyMissing,
    /// The stored receipt key seed is not 32 bytes.
    #[error("exec receipt key seed is not 32 bytes: {0}")]
    KeySeedLength(#[source] std::array::TryFromSliceError),
    /// A receipt was refused.
    #[error("verify: {0}")]
    Receipt(#[source] ReceiptError),
    /// A `LARRY_MODEL_*` setting is unusable, or the variable a route names for
    /// its key is unset.
    #[error("exec model config: {0}")]
    ModelConfig(#[source] ModelError),
    /// The route a task's model calls would use is unknown, absent or damaged
    /// (larry#201): nothing was sent.
    #[error("exec: {0}")]
    Route(#[source] RouteError),
    /// The model step could not write one of the brief's files; nothing was sealed.
    #[error("exec could not write {path}: {fault}")]
    Author {
        /// The repo-relative path being written.
        path: String,
        /// Why it could not be written.
        #[source]
        fault: AuthorFault,
    },
    /// The admission gate refused to run the task.
    #[error("exec gate: {0}")]
    Gate(#[from] RuntimeError),
    /// The task body ran and its report was sealed, but the runtime could not
    /// make the run's record durable, so the sealed report is not returned
    /// (larry#182). A sealed report is only a receipt for work whose record
    /// landed, and a tick that failed its sync proves the record did not.
    #[error("exec run finished but its record is not durable, so no report is returned: {0}")]
    Unsealed(#[source] RuntimeError),
    /// The task journals directory could not be created, listed or tidied (larry#75).
    #[error("exec journals: {0}")]
    Journals(#[source] std::io::Error),
    /// A task journal's lock could not be probed, so the run's liveness is unknown and
    /// is never reported as dead (larry#334).
    #[error("exec journal: {0}")]
    Journal(#[source] RuntimeError),
    /// A terminal session was refused before anything ran (larry#93).
    #[error("session: {0}")]
    Session(#[source] SessionError),
    /// A variable Larry reads holds bytes that are not UTF-8: it is refused,
    /// never read as unset (larry#239).
    #[error("exec {name} is not UTF-8; set it to UTF-8 text")]
    NotUnicode {
        /// The variable's name.
        name: String,
    },
    /// A session's commands could not be confined to a workspace copy, so none ran
    /// (larry#243).
    #[error("exec confinement: {0}")]
    Confine(#[source] ConfineError),
    /// The folder is past a bound a workspace copy keeps, so no copy was made (larry#243).
    #[error("the folder is too large to copy for an isolated session: more than {limit} {what}")]
    SnapshotTooLarge {
        /// What was counted: files, bytes, directory levels or bytes of file names.
        what: &'static str,
        /// The bound it passed.
        limit: u64,
    },
}

impl ExecError {
    /// A cleanup that failed after `cause`, which stays the source (larry#343).
    pub(crate) fn cleanup(cause: Self, step: &'static str, fault: Self) -> Self {
        Self::Cleanup {
            cause: Box::new(cause),
            step,
            fault: Box::new(fault),
        }
    }
}
