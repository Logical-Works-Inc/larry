//! Why `larry exec` failed: the one error type of the executor, its message and its source
//! chain. Its own module because the executor's run and this type are two jobs (larry#398).

use std::fmt;
use std::path::PathBuf;

use larry_runtime::error::RuntimeError;

use super::{AuthorFault, ConfineError, ReceiptError, SessionError, TaskError};
use crate::model::{ModelError, RouteError};

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
    /// `PATH` is unset: no command is looked up under an empty path (larry#213).
    PathUnset,
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
    /// The state directory could not be made.
    StateDir(std::io::Error),
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
    /// A child or group could not be ended after an earlier refusal. The earlier refusal
    /// stays the source, so the operator sees both causes and neither is flattened into a
    /// string (larry#343).
    Cleanup {
        /// The refusal the cleanup ran after.
        cause: Box<ExecError>,
        /// The cleanup step that failed, as the operator reads it.
        step: &'static str,
        /// Why that step failed.
        fault: Box<ExecError>,
    },
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
    /// A task journal's lock could not be probed, so the run's liveness is unknown and
    /// is never reported as dead (larry#334).
    Journal(RuntimeError),
    /// A terminal session was refused before anything ran (larry#93).
    Session(SessionError),
    /// A variable Larry reads holds bytes that are not UTF-8: it is refused,
    /// never read as unset (larry#239).
    NotUnicode {
        /// The variable's name.
        name: String,
    },
    /// A session's commands could not be confined to a workspace copy, so none ran
    /// (larry#243).
    Confine(ConfineError),
    /// The folder is past a bound a workspace copy keeps, so no copy was made (larry#243).
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
            Self::StateDir(ref cause) => write!(out, "exec state directory: {cause}"),
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
            Self::Cleanup {
                ref cause,
                step,
                ref fault,
            } => write!(out, "{cause}; {step}: {fault}"),
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
            Self::Journal(ref cause) => write!(out, "exec journal: {cause}"),
            Self::Session(ref cause) => write!(out, "session: {cause}"),
            Self::NotUnicode { ref name } => {
                write!(out, "exec {name} is not UTF-8; set it to UTF-8 text")
            }
            Self::PathUnset => out.write_str(
                "exec PATH is unset; set it to the directories the commands are found in",
            ),
            Self::Confine(ref cause) => write!(out, "exec confinement: {cause}"),
            Self::SnapshotTooLarge { what, limit } => write!(
                out,
                "the folder is too large to copy for an isolated session: more than {limit} {what}"
            ),
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
            Self::StateDir(ref cause) => Some(cause),
            Self::Sql(ref cause) => Some(cause),
            Self::Reap(ref cause) => Some(cause),
            Self::Spawn { ref source, .. } => Some(source),
            Self::Workspace(ref cause) => Some(cause),
            Self::Cleanup { ref cause, .. } => Some(&**cause),
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
            Self::Journal(ref cause) => Some(cause),
            Self::Session(ref cause) => Some(cause),
            Self::Confine(ref cause) => Some(cause),
            Self::KeyMissing
            | Self::StateDirUnset
            | Self::PathUnset
            | Self::BadTimeout { .. }
            | Self::BadPolicy { .. }
            | Self::Git { .. }
            | Self::NotARepo { .. }
            | Self::BadBase { .. }
            | Self::GroupSurvived { .. }
            | Self::NotUnicode { .. }
            | Self::SnapshotTooLarge { .. } => None,
        }
    }
}

impl From<RuntimeError> for ExecError {
    fn from(cause: RuntimeError) -> Self {
        Self::Gate(cause)
    }
}
