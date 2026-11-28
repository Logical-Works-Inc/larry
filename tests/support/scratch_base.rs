// The one rule for where a test scratch directory sits and how far a test git call may look
// (larry#577, larry#595). `support/temp_dir.rs` and the crate's `executor::sim_tree` both include
// this file by `#[path]`, so the rule has one owner. Plain comments, so the file is items only.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The directory a scratch is made in: the configured temp dir, unless a scratch there would not
/// stand alone. Two places are left for `/var/tmp`, the shared scratch directory that stays on
/// disk (FHS 3.0, 5.15): on Linux a plain directory, which the confinement lists (larry#577), and
/// anywhere inside a git checkout, which every git call in a scratch would climb to (larry#595).
/// A temp dir that cannot be read is the caller's error.
pub(crate) fn guard_base(configured: PathBuf) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    let plain = cfg!(target_os = "linux")
        && fs::metadata(&configured)?.permissions().mode() & 0o1002 != 0o1002;
    Ok(if plain || in_checkout(&configured)? {
        PathBuf::from("/var/tmp")
    } else {
        configured
    })
}

/// Whether `dir`, resolved, or a directory above it holds a `.git` entry: a repository's
/// directory, or the file a linked worktree holds.
fn in_checkout(dir: &Path) -> io::Result<bool> {
    for above in fs::canonicalize(dir)?.ancestors() {
        if above.join(".git").try_exists()? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Bounds the test git call `command`, which runs in `dir`: git never climbs from `dir` into its
/// parent, so no repository above a scratch is seen (git(1), `GIT_CEILING_DIRECTORIES`). A `dir`
/// with no parent has nothing above it.
pub(crate) fn bound_git(command: &mut Command, dir: &Path) {
    if let Some(parent) = dir.parent() {
        command.env("GIT_CEILING_DIRECTORIES", parent);
    }
}
