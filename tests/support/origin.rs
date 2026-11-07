//! The origin repository and state directory every executor journey starts
//! from (larry#85, larry#86, larry#123): a temp directory that removes itself,
//! and a git repository with one commit and no background maintenance.
//! `tests/support/exec_cli.rs` re-exports both for the `larry exec` journeys;
//! `tests/mcp_stdio_journey.rs` includes this file alone.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh directory under the system temp dir, removed when dropped.
pub struct TempDir {
    /// Where the directory lives.
    path: PathBuf,
}

impl TempDir {
    /// Creates an empty directory whose name no other test in this process uses.
    pub fn new() -> io::Result<TempDir> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let run = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("larry-exec-{}-{run}", std::process::id()));
        fs::remove_dir_all(&path).ok();
        fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Cleanup is best effort: a leftover temp dir must never fail a journey.
        fs::remove_dir_all(&self.path).ok();
    }
}

/// A state dir for the executor and a fresh origin repository, in that order.
pub fn fixture() -> Result<(TempDir, TempDir), Box<dyn Error>> {
    let state = TempDir::new()?;
    let origin = TempDir::new()?;
    origin_repo(origin.path())?;
    Ok((state, origin))
}

/// Runs `git -C dir <args>` and fails with git's stderr when it exits non-zero.
fn git(dir: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {args:?} in {} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}

/// Makes `dir` a git repository with one commit holding `README` and `src/lib.rs`.
/// Automatic maintenance is off before the first commit: `git commit` otherwise forks
/// `git maintenance run --auto --detach`, whose object-database lock file can appear
/// under `.git/objects` while a journey hashes the origin (larry#138).
fn origin_repo(dir: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join("README"), "origin readme\n")?;
    fs::write(dir.join("src").join("lib.rs"), "pub fn origin() {}\n")?;
    git(dir, &["init", "-q"])?;
    git(dir, &["config", "maintenance.auto", "false"])?;
    git(dir, &["config", "gc.auto", "0"])?;
    commit_all(dir, "init")
}

/// Stages everything in `dir` and commits it as `message`.
pub fn commit_all(dir: &Path, message: &str) -> Result<(), Box<dyn Error>> {
    git(dir, &["add", "-A"])?;
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            message,
        ],
    )
}
