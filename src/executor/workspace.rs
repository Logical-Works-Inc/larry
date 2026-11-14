//! Clone-per-task workspaces, their staged diff, and the sweep of stale ones (larry#85).
//!
//! A run lives in `<state>/ws/<task_id>/`: the work tree `tree/`, where the task's
//! commands run, and the clone's git directory `git/` beside it. The executor's own
//! git calls name both explicitly, so a command that deletes `tree/.git` or points it
//! at another repository (the origin's, say) never redirects them.

use std::ffi::{OsStr, OsString};
use std::fs::{File, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use super::ExecError;

/// Largest diff kept for one task, in bytes; a longer diff is cut and sealed as truncated (larry#206).
pub(crate) const DIFF_LIMIT: usize = 1_048_576;

/// Bytes read from git's diff stream per piece.
const STREAM_CHUNK: usize = 8_192;

/// Bytes of git's stderr kept for an error message; the rest is drained.
const STDERR_LIMIT: u64 = 65_536;

/// The suffix of an origin's lock file, a sibling of the origin directory (larry#214).
const ORIGIN_LOCK_SUFFIX: &str = ".lock";

/// Git under the executor's environment: PATH only, HOME at the state directory, no system config.
struct Git<'a> {
    /// The state directory, which is also HOME for git.
    state: &'a Path,
    /// The PATH git runs under.
    path_env: &'a str,
}

impl Git<'_> {
    /// The git command under the executor's environment, inside `directory` when given.
    fn command(&self, directory: Option<&Path>, arguments: &[&OsStr]) -> Command {
        let mut command = Command::new("git");
        command
            .args(arguments)
            .env_clear()
            .env("PATH", self.path_env)
            .env("HOME", self.state)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null());
        if let Some(path) = directory {
            command.current_dir(path);
        }
        command
    }

    /// Runs git with `arguments`, inside `directory` when given, and returns its output.
    fn output(&self, directory: Option<&Path>, arguments: &[&OsStr]) -> Result<Output, ExecError> {
        self.command(directory, arguments)
            .output()
            .map_err(spawn_error)
    }
}

/// The typed failure of a git process that could not be started.
fn spawn_error(source: io::Error) -> ExecError {
    ExecError::Spawn {
        argv0: "git".to_owned(),
        source,
    }
}

/// Returns a git call's stdout, or a typed failure that names the step and carries git's stderr.
fn require(output: Output, step: &'static str) -> Result<Vec<u8>, ExecError> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(ExecError::Git {
            step,
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// `--<name>=<path>` as one git argument.
fn flag(name: &str, path: &Path) -> OsString {
    let mut argument = OsString::from(format!("--{name}="));
    argument.push(path.as_os_str());
    argument
}

/// One task's clone; removed when dropped, whether the run succeeded or not.
#[non_exhaustive]
pub(crate) struct Workspace {
    /// The run's directory, `<state>/ws/<task_id>`, holding `tree/` and `git/`.
    root: PathBuf,
    /// The work tree the task's commands run in.
    tree: PathBuf,
    /// The clone's git directory, outside the work tree.
    git_dir: PathBuf,
    /// The state directory, which is HOME for git in this workspace.
    state: PathBuf,
    /// The PATH git runs under in this workspace.
    path_env: String,
}

impl Workspace {
    /// Clones `repo` locally and checks out `base` detached, in a new workspace named `task_id`.
    /// Never a `git worktree`: a worktree shares the origin's `.git`.
    pub(crate) fn create(
        state: &Path,
        repo: &Path,
        base: &str,
        task_id: &str,
        path_env: &str,
    ) -> Result<Workspace, ExecError> {
        let root = state.join("ws").join(task_id);
        std::fs::create_dir_all(&root).map_err(ExecError::Workspace)?;
        // Built before any git call: a failed clone or checkout still leaves a directory to remove.
        let workspace = Workspace {
            tree: root.join("tree"),
            git_dir: root.join("git"),
            root,
            state: state.to_path_buf(),
            path_env: path_env.to_owned(),
        };
        let separate = flag("separate-git-dir", &workspace.git_dir);
        let clone = workspace.git().output(
            None,
            &[
                OsStr::new("clone"),
                OsStr::new("--local"),
                OsStr::new("--no-hardlinks"),
                OsStr::new("--quiet"),
                separate.as_os_str(),
                OsStr::new("--"),
                repo.as_os_str(),
                workspace.tree.as_os_str(),
            ],
        )?;
        if !clone.status.success() {
            return Err(ExecError::NotARepo {
                repo: repo.to_path_buf(),
            });
        }
        let checkout = workspace.bound(&[
            OsStr::new("checkout"),
            OsStr::new("--detach"),
            OsStr::new("--quiet"),
            OsStr::new(base),
        ])?;
        if !checkout.status.success() {
            return Err(ExecError::BadBase {
                base: base.to_owned(),
            });
        }
        Ok(workspace)
    }

    /// The work tree the task's commands run in.
    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        self.tree.as_path()
    }

    /// Stages everything in the work tree and returns the staged diff against `base`, streamed
    /// through the BLAKE3 hasher with its head kept (larry#206). `None` when the commands destroyed
    /// the work tree itself.
    pub(crate) fn diff(&self, base: &str) -> Result<Option<Diff>, ExecError> {
        if !self.tree.is_dir() {
            return Ok(None);
        }
        require(self.bound(&[OsStr::new("add"), OsStr::new("-A")])?, "add")?;
        self.stream_diff(base).map(Some)
    }

    /// Streams git's staged diff against `base` into a sink. The diff is never buffered whole,
    /// and git's stderr is kept to `STDERR_LIMIT` bytes and drained past it.
    fn stream_diff(&self, base: &str) -> Result<Diff, ExecError> {
        let mut command = self.bound_command(&[
            OsStr::new("diff"),
            OsStr::new("--cached"),
            OsStr::new("--no-color"),
            OsStr::new("--no-ext-diff"),
            OsStr::new(base),
        ]);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().map_err(spawn_error)?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            drop(child.kill());
            drop(child.wait());
            return Err(diff_failed("the diff pipes were not opened"));
        };
        let (sunk, errors) = std::thread::scope(|scope| {
            let errors = scope.spawn(move || read_capped(stderr));
            let sunk = pump(stdout);
            if sunk.is_err() {
                drop(child.kill());
            }
            (sunk, errors.join())
        });
        let sink = sunk?;
        let detail = match errors {
            Ok(Ok(kept)) => String::from_utf8_lossy(&kept).trim().to_owned(),
            Ok(Err(source)) => return Err(ExecError::Workspace(source)),
            Err(_) => {
                return Err(ExecError::Workspace(io::Error::other(
                    "the diff stderr reader panicked",
                )));
            }
        };
        let status = child.wait().map_err(spawn_error)?;
        if !status.success() {
            return Err(diff_failed(&detail));
        }
        Ok(sink.finish())
    }

    /// Runs git against this workspace's own git directory and work tree, named explicitly,
    /// so nothing the commands left in the work tree can redirect it.
    fn bound(&self, arguments: &[&OsStr]) -> Result<Output, ExecError> {
        self.bound_command(arguments).output().map_err(spawn_error)
    }

    /// The git command bound to this workspace's git directory and work tree.
    fn bound_command(&self, arguments: &[&OsStr]) -> Command {
        let git_dir = flag("git-dir", &self.git_dir);
        let work_tree = flag("work-tree", &self.tree);
        let mut full: Vec<&OsStr> = vec![git_dir.as_os_str(), work_tree.as_os_str()];
        full.extend_from_slice(arguments);
        self.git().command(Some(&self.root), &full)
    }

    /// Git bound to this workspace's environment.
    fn git(&self) -> Git<'_> {
        Git {
            state: self.state.as_path(),
            path_env: self.path_env.as_str(),
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // A removal that fails leaves a directory the next start's sweep collects.
        drop(std::fs::remove_dir_all(&self.root));
    }
}

/// The diff a report records (larry#206): its first `DIFF_LIMIT` bytes, whether anything was cut
/// past them, and the full diff's byte count and BLAKE3 digest.
pub(crate) struct Diff {
    /// The kept bytes, lossily decoded.
    pub(crate) text: String,
    /// Whether the full diff is longer than the kept bytes.
    pub(crate) truncated: bool,
    /// The full diff's length in bytes.
    pub(crate) bytes: u64,
    /// The BLAKE3 digest of the full diff, as 64 hex characters.
    pub(crate) digest: String,
}

/// A diff streamed through the BLAKE3 hasher: it keeps the head and counts every byte, so
/// memory holds at most `DIFF_LIMIT` bytes however large the change is (larry#206).
pub(crate) struct DiffSink {
    /// The first `DIFF_LIMIT` bytes.
    head: Vec<u8>,
    /// Whether any byte came past the head.
    truncated: bool,
    /// Every byte pushed so far.
    bytes: u64,
    /// The hash of every byte pushed so far.
    hasher: lgwks_std::hash::Hasher,
}

impl DiffSink {
    /// An empty sink.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            head: Vec::new(),
            truncated: false,
            bytes: 0,
            hasher: lgwks_std::hash::Hasher::new(),
        }
    }

    /// Adds the next piece of the diff, in the order git wrote it.
    pub(crate) fn push(&mut self, piece: &[u8]) -> io::Result<()> {
        let length = u64::try_from(piece.len()).map_err(io::Error::other)?;
        self.bytes = self
            .bytes
            .checked_add(length)
            .ok_or_else(|| io::Error::other("the diff is longer than u64 bytes"))?;
        self.hasher.update(piece);
        let kept = DIFF_LIMIT.saturating_sub(self.head.len()).min(piece.len());
        self.head.extend_from_slice(&piece[..kept]);
        self.truncated = self.truncated || kept < piece.len();
        Ok(())
    }

    /// The diff as the report records it.
    #[must_use]
    pub(crate) fn finish(self) -> Diff {
        Diff {
            text: String::from_utf8_lossy(&self.head).into_owned(),
            truncated: self.truncated,
            bytes: self.bytes,
            digest: self.hasher.finalize().to_hex(),
        }
    }
}

/// The four report members a staged diff contributes. `None`, when the commands destroyed the
/// work tree, records no diff, which acceptance already refuses (larry#206).
#[must_use]
pub(crate) fn report_members(diff: Option<Diff>) -> [(&'static str, lgwks_std::json::Value); 4] {
    match diff {
        Some(diff) => [
            ("diff", lgwks_std::json::json!(diff.text)),
            ("diff_truncated", lgwks_std::json::json!(diff.truncated)),
            ("diff_bytes", lgwks_std::json::json!(diff.bytes)),
            ("diff_digest", lgwks_std::json::json!(diff.digest)),
        ],
        None => [
            ("diff", lgwks_std::json::Value::Null),
            ("diff_truncated", lgwks_std::json::json!(false)),
            ("diff_bytes", lgwks_std::json::Value::Null),
            ("diff_digest", lgwks_std::json::Value::Null),
        ],
    }
}

/// Feeds every piece of git's diff stream to a sink until the stream ends.
fn pump(mut stdout: impl Read) -> Result<DiffSink, ExecError> {
    let mut sink = DiffSink::new();
    let mut chunk = [0_u8; STREAM_CHUNK];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => return Ok(sink),
            Ok(read) => sink.push(&chunk[..read]).map_err(ExecError::Workspace)?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ExecError::Workspace(error)),
        }
    }
}

/// Keeps the first `STDERR_LIMIT` bytes of a pipe and drains the rest, so git never blocks on it.
fn read_capped(pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    let mut limited = pipe.take(STDERR_LIMIT);
    limited.read_to_end(&mut kept)?;
    let mut rest = limited.into_inner();
    io::copy(&mut rest, &mut io::sink())?;
    Ok(kept)
}

/// A failed `git diff`, named by its step and carrying git's stderr.
fn diff_failed(detail: &str) -> ExecError {
    ExecError::Git {
        step: "diff",
        detail: detail.to_owned(),
    }
}

/// Makes `dir`, whose files are already written, a repository with one commit of them, for
/// `larry eval` to clone (larry#87). Background maintenance is off before the commit, so only
/// the executor ever changes it (larry#138), and git runs under the executor's environment.
pub(crate) fn seed_origin(state: &Path, path_env: &str, dir: &Path) -> Result<(), ExecError> {
    let git = Git { state, path_env };
    let steps: [(&'static str, &[&str]); 8] = [
        ("init", &["init", "--quiet"]),
        ("config", &["config", "maintenance.auto", "false"]),
        ("config", &["config", "gc.auto", "0"]),
        ("config", &["config", "user.name", "larry eval"]),
        ("config", &["config", "user.email", "eval@larry.invalid"]),
        ("config", &["config", "commit.gpgsign", "false"]),
        ("add", &["add", "-A"]),
        ("commit", &["commit", "--quiet", "--message", "origin"]),
    ];
    for (step, arguments) in steps {
        let arguments: Vec<&OsStr> = arguments.iter().map(OsStr::new).collect();
        require(git.output(Some(dir), &arguments)?, step)?;
    }
    Ok(())
}

/// Removes every `<state>/ws/*` directory that no live ledger row names and whose
/// run is not live: a concurrent run's workspace is kept between its clone and its
/// first command.
///
/// `live` is the caller's liveness proof, because a run id carries no process id
/// any more (larry#182 defect 3): the executor asks its journals' locks, and
/// `larry eval`'s origins have no journal at all.
pub(crate) fn sweep(
    state: &Path,
    live: &[String],
    owner_live: impl Fn(&str) -> Result<bool, ExecError>,
) -> Result<(), ExecError> {
    sweep_dir(&state.join("ws"), live, owner_live)
}

/// Removes every `<state>/eval/origin-<pid>-<n>` directory whose `larry eval` holds no
/// lock: a kill -9 runs no destructor, so its origins outlive it (larry#87). The lock
/// is the proof of a live eval, never the pid in the name (larry#214), and the stale
/// lock files of removed origins go with them.
pub(crate) fn sweep_origins(state: &Path) -> Result<(), ExecError> {
    let eval = state.join("eval");
    sweep_dir(&eval, &[], |origin| origin_held(&eval, origin))?;
    remove_unheld_locks(&eval)
}

/// The lock file of origin `name` under `eval`: a sibling of the origin directory, so
/// the lock is never part of the origin's tree.
pub(crate) fn origin_lock_path(eval: &Path, name: &str) -> PathBuf {
    eval.join(format!("{name}{ORIGIN_LOCK_SUFFIX}"))
}

/// Takes the exclusive lock of origin `name`, which the `larry eval` that owns it holds
/// until it removes the origin. The kernel releases the lock when that process dies.
///
/// # Errors
///
/// [`ExecError::Workspace`] when the lock file cannot be opened, or the lock is already
/// held by another live holder (`WouldBlock`).
pub(crate) fn hold_origin(eval: &Path, name: &str) -> Result<File, ExecError> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(origin_lock_path(eval, name))
        .map_err(ExecError::Workspace)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(ExecError::Workspace(io::Error::new(
            io::ErrorKind::WouldBlock,
            "the origin is already held by a live eval",
        ))),
        Err(TryLockError::Error(cause)) => Err(ExecError::Workspace(cause)),
    }
}

/// Whether a live holder keeps origin `name`'s lock. A lock file that is absent, or
/// that this probe can take, reads as dead; the probe releases what it takes.
///
/// # Errors
///
/// [`ExecError::Workspace`] when the lock file cannot be read or locked for any reason
/// other than a live holder.
fn origin_held(eval: &Path, name: &str) -> Result<bool, ExecError> {
    let file = match File::open(origin_lock_path(eval, name)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ExecError::Workspace(error)),
    };
    match file.try_lock() {
        Ok(()) => {
            file.unlock().map_err(ExecError::Workspace)?;
            Ok(false)
        }
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(cause)) => Err(ExecError::Workspace(cause)),
    }
}

/// Removes every origin lock file under `eval` that no live holder keeps.
fn remove_unheld_locks(eval: &Path) -> Result<(), ExecError> {
    let Some(entries) = listing(eval)? else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(ExecError::Workspace)?;
        let name = entry.file_name();
        let Some(origin) = name
            .to_str()
            .and_then(|text| text.strip_suffix(ORIGIN_LOCK_SUFFIX))
        else {
            continue;
        };
        if !origin_held(eval, origin)? {
            std::fs::remove_file(entry.path()).map_err(ExecError::Workspace)?;
        }
    }
    Ok(())
}

/// The entries of `root`, or `None` when it does not exist: a state directory that has
/// never run anything holds nothing to sweep.
fn listing(root: &Path) -> Result<Option<std::fs::ReadDir>, ExecError> {
    match std::fs::read_dir(root) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ExecError::Workspace(error)),
    }
}

/// Removes every directory under `root` that `live` does not name and whose owner is gone.
/// Only directories are candidates; a file under `root` is nobody's.
fn sweep_dir(
    root: &Path,
    live: &[String],
    owner_live: impl Fn(&str) -> Result<bool, ExecError>,
) -> Result<(), ExecError> {
    let Some(entries) = listing(root)? else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(ExecError::Workspace)?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        if !entry.file_type().map_err(ExecError::Workspace)?.is_dir() {
            continue;
        }
        if live.iter().any(|id| id.as_str() == text) || owner_live(text)? {
            continue;
        }
        std::fs::remove_dir_all(entry.path()).map_err(ExecError::Workspace)?;
    }
    Ok(())
}

/// Seeded adversarial simulations of the sweep (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_workspace.rs"]
mod sim;

/// Unit tests of the origin lock (larry#214).
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{hold_origin, origin_held, sweep_origins};

    /// A pid no live process holds: the largest positive `i32` less one.
    const DEAD_PID: i32 = 2_147_483_646;

    /// An origin whose pid is dead but whose lock a live eval holds is kept, and removed
    /// with its lock file once the lock is released (larry#214).
    #[test]
    fn an_eval_origin_is_live_while_its_lock_is_held() -> Result<(), Box<dyn std::error::Error>> {
        let state: PathBuf =
            std::env::temp_dir().join(format!("larry-origin-lock-{}", std::process::id()));
        let eval = state.join("eval");
        let name = format!("origin-{DEAD_PID}-0");
        let origin = eval.join(&name);
        std::fs::create_dir_all(&origin)?;
        let held = hold_origin(&eval, &name)?;
        assert!(
            origin_held(&eval, &name)?,
            "a held lock reads as live, whatever pid the name carries"
        );
        sweep_origins(&state)?;
        assert!(
            origin.is_dir(),
            "the sweep keeps an origin whose lock is held"
        );
        drop(held);
        assert!(!origin_held(&eval, &name)?, "a released lock reads as dead");
        sweep_origins(&state)?;
        assert!(
            !origin.exists(),
            "the sweep removes an origin once its lock is released"
        );
        assert!(
            !eval.join(format!("{name}.lock")).exists(),
            "the sweep removes the stale lock file"
        );
        drop(std::fs::remove_dir_all(&state));
        Ok(())
    }
}
