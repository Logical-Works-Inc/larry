//! Shared helpers of the isolated-session simulations (larry#243): scratch directories,
//! seeded origin folders of each shape, settings over a state directory, and a digest of a
//! whole tree that is the oracle for "the origin stayed byte-identical". The digest reads
//! the folder itself, never the code under test.

use std::io::{self, Write as _};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::seeded::Seeded;

use super::Settings;
use crate::sim_error::SimError;

/// Names scratch directories apart inside one test process.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The shape of an origin folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::executor) enum Shape {
    /// A git repository with one commit.
    Git,
    /// A git repository with no commit: every file is untracked.
    NoCommit,
    /// No repository at all.
    Plain,
    /// A folder inside a repository, as a person may open one.
    Subfolder,
}

impl Shape {
    /// Every shape.
    pub(in crate::executor) const ALL: [Shape; 4] =
        [Shape::Git, Shape::NoCommit, Shape::Plain, Shape::Subfolder];
}

/// Scratch trees whose removal failed when their guard dropped.
static LEFT: AtomicU64 = AtomicU64::new(0);

/// A scratch directory that is empty when made and removed when dropped, so a process id the
/// system reuses never lands a test in a stale tree. The crate's tests hold it, and the checker
/// in `governance::temp_paths` allows the process id in a temp path here alone (larry#267).
#[derive(Debug)]
pub(crate) struct Scratch {
    /// The directory, resolved the way the kernel resolves it.
    path: PathBuf,
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.path) {
            Ok(()) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => {
                LEFT.fetch_add(1, Ordering::Relaxed);
                let mut errors = io::stderr().lock();
                let said = writeln!(errors, "scratch {} left: {cause}", self.path.display());
                if said.is_err() {
                    LEFT.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

/// A stale tree a dead process with this process id left at a fixture's old `<label>-<pid>`
/// name (larry#267): a test makes one to prove the fixture that replaced it starts empty anyway.
/// It removes itself when dropped, so a failing test leaves nothing behind either.
#[derive(Debug)]
pub(crate) struct Leftover {
    /// The old fixture's directory.
    path: PathBuf,
}

/// A leftover at the old `<label>-<pid>` name, holding one stale file named `file`.
pub(crate) fn leftover(label: &str, file: &str) -> Result<Leftover, SimError> {
    let dir = std::env::temp_dir().join(format!("{label}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).or_else(|cause| match cause.kind() {
        io::ErrorKind::NotFound => Ok(()),
        _ => Err(cause),
    })?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(file), "left by a dead run")?;
    Ok(Leftover { path: dir })
}

impl Deref for Leftover {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Leftover {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.path));
    }
}

/// The directory name of scratch number `number` under `label`: the process and the count,
/// no clock and no random.
fn scratch_name(label: &str, number: u64) -> String {
    format!("larry-243-{label}-{}-{number}", std::process::id())
}

/// A fresh empty directory: one already at the name is removed first.
pub(crate) fn scratch(label: &str) -> Result<Scratch, SimError> {
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(scratch_name(label, number));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
        Err(cause) => return Err(cause.into()),
    }
    std::fs::create_dir_all(&dir)?;
    Ok(Scratch {
        path: std::fs::canonicalize(dir)?,
    })
}

/// A directory left at the next name by a dead process with the same id is gone before the
/// test starts, a scratch is empty, and dropping it removes the tree and counts no leftover.
#[test]
fn sim_tree_scratch_starts_empty_and_removes_itself() -> Result<(), SimError> {
    let stale = std::env::temp_dir().join(scratch_name("stale", NEXT.load(Ordering::Relaxed)));
    std::fs::create_dir_all(stale.join("old"))?;
    std::fs::write(stale.join("old").join("file"), "left by a dead run")?;
    let fresh = scratch("stale")?;
    assert!(
        std::fs::read_dir(&fresh)?.next().is_none(),
        "a stale tree leaked into a fresh scratch"
    );
    std::fs::write(fresh.join("mine"), "x")?;
    let path = fresh.to_path_buf();
    drop(fresh);
    assert!(!path.exists(), "the scratch outlived its guard");
    assert_eq!(LEFT.load(Ordering::Relaxed), 0, "a removal failed");
    Ok(())
}

/// A leftover at a fixture's old name holds its stale file while it lives, a fresh scratch under
/// the same label is empty with it present, and the leftover removes itself (larry#267).
#[test]
fn sim_tree_a_leftover_at_an_old_name_never_reaches_a_fresh_scratch() -> Result<(), SimError> {
    let stale = leftover("larry-267-leftover", "hello.txt")?;
    assert!(
        stale.join("hello.txt").exists(),
        "the leftover holds the stale file the old fixture tripped on"
    );
    let fresh = scratch("larry-267-leftover")?;
    assert!(
        std::fs::read_dir(&fresh)?.next().is_none(),
        "a leftover at an old name leaked into the new fixture"
    );
    drop(fresh);
    let path = stale.to_path_buf();
    drop(stale);
    assert!(!path.exists(), "the leftover outlived its guard");
    Ok(())
}

/// The PATH the simulated executor runs under: the test process's own.
pub(in crate::executor) fn path_env() -> Result<String, SimError> {
    Ok(std::env::var("PATH")?)
}

/// Settings over `state`, with `policy` as `LARRY_POLICY` when given.
pub(in crate::executor) fn settings(
    state: &Path,
    policy: Option<&str>,
) -> Result<Settings, SimError> {
    let state = state.to_string_lossy().into_owned();
    let path = path_env()?;
    let policy = policy.map(str::to_owned);
    Ok(Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(state.clone()),
        "PATH" => Some(path.clone()),
        "LARRY_POLICY" => policy.clone(),
        _ => None,
    })?)
}

/// What the person's file `index` holds under `seed`.
fn content(rng: &mut Seeded, index: u64) -> Result<String, SimError> {
    let lines = rng.below(5)?.saturating_add(1);
    let mut text = String::new();
    for line in 0..lines {
        text = format!("{text}file {index} line {line} {}\n", rng.below(1000)?);
    }
    Ok(text)
}

/// One origin folder: where it is, and the files a snapshot must copy, relative and sorted.
#[derive(Debug)]
pub(in crate::executor) struct Origin {
    /// The folder the person opened.
    pub(in crate::executor) dir: PathBuf,
    /// Every file a faithful copy holds.
    pub(in crate::executor) files: Vec<String>,
}

/// Runs `git` with `arguments` in `dir` under a clean environment.
fn git(state: &Path, dir: &Path, arguments: &[&str]) -> Result<(), SimError> {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(dir)
        .env_clear()
        .env("PATH", path_env()?)
        .env("HOME", state)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(SimError::Missing("a git step of the origin builder"))
    }
}

/// A seeded origin of `shape` under `root`. Files are `.gitignore`, top-level files and
/// files in nested folders; a `target/` folder is ignored by git shapes and copied by the
/// plain one, which has no ignore rules.
pub(in crate::executor) fn origin(
    settings: &Settings,
    root: &Path,
    seed: u64,
    shape: Shape,
) -> Result<Origin, SimError> {
    let mut rng = Seeded::from_seed(seed.wrapping_add(7 << 32));
    let dir = match shape {
        Shape::Subfolder => root.join("sub"),
        Shape::Git | Shape::NoCommit | Shape::Plain => root.to_path_buf(),
    };
    std::fs::create_dir_all(dir.join("target"))?;
    let mut files = vec![String::from(".gitignore")];
    std::fs::write(dir.join(".gitignore"), "target/\n")?;
    let count = rng.below(6)?.saturating_add(2);
    for index in 0..count {
        let name = if index.checked_rem(2) == Some(0) {
            format!("f{index}.txt")
        } else {
            std::fs::create_dir_all(dir.join("nest/deep"))?;
            format!("nest/deep/g{index}.txt")
        };
        std::fs::write(dir.join(&name), content(&mut rng, index)?)?;
        files.push(name);
    }
    std::fs::write(dir.join("target/junk.o"), "build output")?;
    match shape {
        Shape::Git => settings.seed_origin(&dir)?,
        Shape::Subfolder => settings.seed_origin(root)?,
        Shape::NoCommit => git(settings.state_dir(), &dir, &["init", "--quiet"])?,
        Shape::Plain => files.push(String::from("target/junk.o")),
    }
    files.sort();
    Ok(Origin {
        dir: std::fs::canonicalize(dir)?,
        files,
    })
}

/// A digest of every entry beneath `root`: its relative path, its kind and its bytes or
/// link target. `skip_git` leaves `.git` out, for comparing two folders built apart.
pub(in crate::executor) fn tree_digest(root: &Path, skip_git: bool) -> Result<String, SimError> {
    let mut hasher = lgwks_std::hash::Hasher::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let mut names: Vec<_> = std::fs::read_dir(root.join(&relative))?
            .map(|entry| entry.map(|found| found.file_name()))
            .collect::<Result<_, _>>()?;
        names.sort();
        for name in names {
            if skip_git && name == ".git" {
                continue;
            }
            let path = relative.join(&name);
            let full = root.join(&path);
            let meta = std::fs::symlink_metadata(&full)?;
            hasher.update(path.to_string_lossy().as_bytes());
            if meta.is_dir() {
                hasher.update(b"\0dir\0");
                pending.push(path);
            } else if meta.is_symlink() {
                hasher.update(b"\0link\0");
                hasher.update(std::fs::read_link(&full)?.to_string_lossy().as_bytes());
            } else {
                hasher.update(b"\0file\0");
                hasher.update(&std::fs::read(&full)?);
            }
        }
    }
    Ok(hasher.finalize().to_hex())
}

/// Every file (not folder) beneath `root`, relative, sorted, with `.git` left out.
pub(in crate::executor) fn files_beneath(root: &Path) -> Result<Vec<String>, SimError> {
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(root.join(&relative))? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let path = relative.join(&name);
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                found.push(path.to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    Ok(found)
}
