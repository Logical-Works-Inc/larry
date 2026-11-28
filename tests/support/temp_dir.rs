// The one temp guard of the integration tests (larry#267): a directory under the system temp dir
// that is empty when it is made and removed when it is dropped, so a process id the system reuses
// never lands a journey in a dead run's tree. Each test crate includes this file once, directly
// or through `support/origin.rs`; `governance::temp_paths` allows a process id in a temp path in
// this file alone. Written as plain comments so an `include!` of this file is items only.

use std::fs;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// Included as source, not loaded as a module file: the runtime's tests load this guard twice
// (`admission/tests.rs`), and the rule keeps one file.
pub(crate) mod scratch_base {
    include!("scratch_base.rs");
}
use scratch_base::guard_base;

/// A fresh directory under [`guard_base`], removed when dropped.
pub(crate) struct TempDir {
    /// Where the directory lives.
    path: PathBuf,
}

/// The path the next guard in this process takes: the process and the count, no clock and no
/// random.
fn next_path() -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let run = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = guard_base(std::env::temp_dir())?;
    Ok(base.join(format!("larry-exec-{}-{run}", std::process::id())))
}

/// The number of names one guard tries before it refuses, so two copies of this file in one test
/// binary (the runtime's unit tests include it twice) never share a directory.
const ATTEMPTS: u32 = 4096;

impl TempDir {
    /// Creates an empty directory whose name no other guard in this process holds. The name is
    /// claimed by `create_dir`, so a second copy of this guard in the same binary moves to the
    /// next name instead of sharing the first copy's directory.
    pub(crate) fn new() -> io::Result<TempDir> {
        for _ in 0..ATTEMPTS {
            let path = next_path()?;
            match fs::create_dir(&path) {
                Ok(()) => return Ok(TempDir { path }),
                Err(cause) if cause.kind() == io::ErrorKind::AlreadyExists => {}
                Err(cause) => return Err(cause),
            }
        }
        Err(io::Error::other("no free temp guard name"))
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
    pub(crate) fn path(&self) -> &Path {
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
        // A directory already gone is clean; any other failure leaves a tree behind, which fails
        // the test that owns the guard unless that test is already unwinding.
        let removed = match fs::remove_dir_all(&self.path) {
            Ok(()) => Ok(()),
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(cause) => Err(cause),
        };
        assert!(
            std::thread::panicking() || removed.is_ok(),
            "the temp dir {} outlived its guard: {removed:?}",
            self.path.display()
        );
    }
}

/// larry#577: on Linux the confinement lists every level above a state directory that is not a
/// shared scratch directory, and `setpriv` refuses the whole command when a listed entry is gone
/// (CWE-367). Beside other tests' short-lived guards that is a race, so a guard sits only in a
/// directory the walk never lists, whatever `TMPDIR` names (INVARIANTS: temp guard).
#[cfg(target_os = "linux")]
#[test]
fn a_guard_never_sits_in_a_directory_the_confinement_lists() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let guard = TempDir::new()?;
    let parent = guard
        .path()
        .parent()
        .ok_or_else(|| io::Error::other("a guard has a parent"))?;
    let mode = fs::metadata(parent)?.permissions().mode();
    assert_eq!(
        mode & 0o1002,
        0o1002,
        "{} is not sticky and world-writable, so the confinement lists it",
        parent.display()
    );
    Ok(())
}

/// Every mode short of sticky and world-writable is a directory the confinement lists, so it is
/// left for `/var/tmp`; a shared one is kept, and a missing one is the typed fault (larry#577,
/// CWE-367).
#[cfg(target_os = "linux")]
#[test]
fn only_a_plain_directory_sends_a_guard_to_the_shared_one() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let guard = TempDir::new()?;
    let configured = guard.path().join("configured");
    fs::create_dir(&configured)?;
    let shared = PathBuf::from("/var/tmp");
    for (mode, kept) in [
        (0o700, false),
        (0o777, false),
        (0o1700, false),
        (0o1775, false),
        (0o1777, true),
    ] {
        fs::set_permissions(&configured, fs::Permissions::from_mode(mode))?;
        let want = if kept { &configured } else { &shared };
        assert_eq!(&guard_base(configured.clone())?, want, "mode {mode:o}");
    }
    let missing = guard_base(guard.path().join("missing"));
    assert_eq!(
        missing.map_err(|cause| cause.kind()),
        Err(io::ErrorKind::NotFound)
    );
    Ok(())
}

/// A temp dir inside a git checkout makes every git call in a scratch climb to that checkout
/// (larry#595, git(1) repository discovery), so a scratch is never made there, whether `.git` is a
/// directory or the file a linked worktree holds, and whatever the mode.
#[test]
fn a_scratch_inside_a_checkout_is_left_for_the_shared_one() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let guard = TempDir::new()?;
    for marker in ["directory", "file"] {
        let checkout = guard.path().join(marker);
        let configured = checkout.join("target").join("tmp");
        fs::create_dir_all(&configured)?;
        fs::set_permissions(&configured, fs::Permissions::from_mode(0o1777))?;
        if marker == "directory" {
            fs::create_dir(checkout.join(".git"))?;
        } else {
            fs::write(checkout.join(".git"), "gitdir: elsewhere\n")?;
        }
        assert_eq!(
            guard_base(configured)?,
            PathBuf::from("/var/tmp"),
            "{marker}"
        );
    }
    Ok(())
}

/// A test git call bounded to its directory never climbs into the repository above it
/// (larry#595, git(1) `GIT_CEILING_DIRECTORIES`), and still finds a repository at the directory.
#[test]
fn a_bounded_git_never_sees_the_checkout_above_its_scratch() -> io::Result<()> {
    let guard = TempDir::new()?;
    let checkout = guard.path().join("checkout");
    let scratch = checkout.join("scratch");
    fs::create_dir_all(&scratch)?;
    let run = |dir: &Path, args: &[&str]| {
        let mut command = std::process::Command::new("git");
        command
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        scratch_base::bound_git(&mut command, dir);
        command.output()
    };
    assert!(run(&checkout, &["init", "--quiet"])?.status.success());
    let found = run(&checkout, &["rev-parse", "--is-inside-work-tree"])?;
    assert!(
        found.status.success(),
        "the directory's own repository is found"
    );
    let above = run(&scratch, &["rev-parse", "--show-toplevel"])?;
    assert!(
        !above.status.success(),
        "git climbed out of its scratch to {}",
        String::from_utf8_lossy(&above.stdout)
    );
    Ok(())
}

/// A dead run left a directory and a stale file at the name the guard takes: the guard starts
/// empty anyway and removes the tree when it drops (larry#267). The name carries this thread's id,
/// so the copies of this file compiled into one test binary never race on it.
#[test]
fn a_temp_dir_starts_empty_beside_a_leftover_and_removes_itself() -> io::Result<()> {
    let claimed = TempDir::new()?;
    assert!(
        fs::read_dir(claimed.path())?.next().is_none(),
        "a claimed temp dir is not empty"
    );
    drop(claimed);
    let leftover = std::env::temp_dir().join(format!(
        "larry-exec-left-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
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
