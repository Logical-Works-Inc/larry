//! Clone-per-task workspaces, their staged diff, and the sweep of stale ones (larry#85).
//!
//! A run lives in `<state>/ws/<task_id>/`: the work tree `tree/`, where the task's
//! commands run, and the clone's git directory `git/` beside it. The executor's own
//! git calls name both explicitly, so a command that deletes `tree/.git` or points it
//! at another repository (the origin's, say) never redirects them.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use lgwks_std::process::process_exists;

use super::ExecError;

/// Largest diff kept for one task, in bytes.
const DIFF_LIMIT: usize = 1_048_576;

/// The prefix `larry eval` names its origins by: `origin-<pid>-<index>`.
const EVAL_ORIGIN_PREFIX: &str = "origin-";

/// Git under the executor's environment: PATH only, HOME at the state directory, no system config.
struct Git<'a> {
    /// The state directory, which is also HOME for git.
    state: &'a Path,
    /// The PATH git runs under.
    path_env: &'a str,
}

impl Git<'_> {
    /// Runs git with `arguments`, inside `directory` when given, and returns its output.
    fn output(&self, directory: Option<&Path>, arguments: &[&OsStr]) -> Result<Output, ExecError> {
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
        command.output().map_err(|source| ExecError::Spawn {
            argv0: "git".to_owned(),
            source,
        })
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

    /// Stages everything in the work tree and returns the staged diff against `base`, capped
    /// at 1 MiB. `None` when the commands destroyed the work tree itself.
    pub(crate) fn diff(&self, base: &str) -> Result<Option<String>, ExecError> {
        if !self.tree.is_dir() {
            return Ok(None);
        }
        require(self.bound(&[OsStr::new("add"), OsStr::new("-A")])?, "add")?;
        let staged = self.bound(&[
            OsStr::new("diff"),
            OsStr::new("--cached"),
            OsStr::new("--no-color"),
            OsStr::new("--no-ext-diff"),
            OsStr::new(base),
        ])?;
        let mut bytes = require(staged, "diff")?;
        bytes.truncate(DIFF_LIMIT);
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Runs git against this workspace's own git directory and work tree, named explicitly,
    /// so nothing the commands left in the work tree can redirect it.
    fn bound(&self, arguments: &[&OsStr]) -> Result<Output, ExecError> {
        let git_dir = flag("git-dir", &self.git_dir);
        let work_tree = flag("work-tree", &self.tree);
        let mut full: Vec<&OsStr> = vec![git_dir.as_os_str(), work_tree.as_os_str()];
        full.extend_from_slice(arguments);
        self.git().output(Some(&self.root), &full)
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

/// Removes every `<state>/eval/origin-<pid>-<n>` directory whose `larry eval` is gone: a kill -9
/// runs no destructor, so its origins outlive it (larry#87). An origin's own name carries
/// the eval process's pid, so it keeps its own liveness rule.
pub(crate) fn sweep_origins(state: &Path) -> Result<(), ExecError> {
    sweep_dir(&state.join("eval"), &[], eval_alive)
}

/// Whether the `larry eval` that named `origin` (`origin-<pid>-<n>`) may still be
/// running: its pid is this process or a live one.
fn eval_alive(origin: &str) -> Result<bool, ExecError> {
    let Some(pid) = origin
        .strip_prefix(EVAL_ORIGIN_PREFIX)
        .and_then(|rest| rest.split('-').next())
        .and_then(|text| text.parse::<i32>().ok())
        .filter(|pid| *pid > 0)
    else {
        return Ok(false);
    };
    if i64::from(pid) == i64::from(std::process::id()) {
        return Ok(true);
    }
    process_exists(pid).map_err(ExecError::Reap)
}

/// Removes every directory under `root` that `live` does not name and whose owner is gone.
fn sweep_dir(
    root: &Path,
    live: &[String],
    owner_live: impl Fn(&str) -> Result<bool, ExecError>,
) -> Result<(), ExecError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ExecError::Workspace(error)),
    };
    for entry in entries {
        let entry = entry.map_err(ExecError::Workspace)?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        if live.iter().any(|id| id.as_str() == text) || owner_live(text)? {
            continue;
        }
        if entry.file_type().map_err(ExecError::Workspace)?.is_dir() {
            std::fs::remove_dir_all(entry.path()).map_err(ExecError::Workspace)?;
        }
    }
    Ok(())
}

/// Seeded adversarial simulations of the sweep (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_workspace.rs"]
mod sim;
