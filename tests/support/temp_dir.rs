//! The one temp guard of the integration tests (larry#267): a directory under the system temp dir
//! that is empty when it is made and removed when it is dropped, so a process id the system reuses
//! never lands a journey in a dead run's tree. Each test crate includes this file once, directly
//! or through `support/origin.rs`; `governance::temp_paths` allows a process id in a temp path in
//! this file alone.

use std::fs;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh directory under the system temp dir, removed when dropped.
pub struct TempDir {
    /// Where the directory lives.
    path: PathBuf,
}

/// The path the next guard in this process takes: the process and the count, no clock and no
/// random.
fn next_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let run = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("larry-exec-{}-{run}", std::process::id()))
}

impl TempDir {
    /// Creates an empty directory whose name no other test in this process uses.
    pub fn new() -> io::Result<TempDir> {
        Self::at(next_path())
    }

    /// The guard over the empty directory at `path`: one already there is removed first.
    fn at(path: PathBuf) -> io::Result<TempDir> {
        fs::remove_dir_all(&path).or_else(|cause| match cause.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(cause),
        })?;
        fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Cleanup is best effort: a leftover temp dir must never fail a journey.
        drop(fs::remove_dir_all(&self.path));
    }
}

/// A dead run left a directory and a stale file at the name the guard takes: the guard starts
/// empty anyway and removes the tree when it drops (larry#267).
#[test]
fn a_temp_dir_starts_empty_beside_a_leftover_and_removes_itself() -> io::Result<()> {
    let leftover = std::env::temp_dir().join(format!("larry-exec-left-{}", std::process::id()));
    fs::create_dir_all(leftover.join("old"))?;
    fs::write(leftover.join("old").join("file"), "left by a dead run")?;
    let fresh = TempDir::at(leftover.clone())?;
    assert!(
        fs::read_dir(fresh.path())?.next().is_none(),
        "a leftover tree leaked into a fresh temp dir"
    );
    drop(fresh);
    assert!(!leftover.exists(), "the temp dir outlived its guard");
    Ok(())
}
