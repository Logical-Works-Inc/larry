//! Seeded simulations of the workspace snapshot, its bounds and the review summary
//! (larry#243, ADR 0046). The copy is judged against a reference: the files the origin
//! builder wrote that a person's folder shows (git's own ignore rules apply to a repository,
//! none to a plain folder), compared byte for byte, with the origin's digest as the oracle
//! that the folder was only read. Sources: git-ls-files(1), git-config(1) `core.fsmonitor`
//! (a program named in a folder's own config), CWE-59 (a link followed out of a tree) and
//! CVE-2021-42574 (bidi control characters in a name).

use std::error::Error;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::process::Command;

use lgwks_std::seeded::Seeded;

use super::{Copied, Limits, MAX_BYTES, MAX_DEPTH, MAX_FILES, copy_entries, summary};
use crate::executor::sim_tree::{
    Shape, bound_git, files_beneath, origin, path_env, scratch, settings, tree_digest,
};
use crate::executor::workspace::{DiffSink, Workspace, sweep};
use crate::executor::{AuthorFault, ExecError};
use crate::sim_error::SimError;

/// Fault injection through the snapshot, its apply and its summary (larry#575).
#[path = "sim_snapshot_faults.rs"]
mod faults;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds each shape runs.
const SEEDS: u64 = 12;

/// Makes the symlink `link` pointing at `target`, which need not exist.
fn symlink(target: &str, link: &Path) -> Result<(), SimError> {
    let made = Command::new("ln")
        .arg("-s")
        .arg(target)
        .arg(link)
        .status()?;
    if made.success() {
        Ok(())
    } else {
        Err(SimError::Missing("a symlink: ln -s failed"))
    }
}

/// The copy of every shape holds exactly the reference files, byte for byte, links as links,
/// and nothing git ignores; the folder is byte-identical afterwards; the base commit is clean.
#[test]
fn sim_snapshot_matches_the_reference_listing_for_every_seed() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        for shape in Shape::ALL {
            let (root, state) = (scratch("snap-origin")?, scratch("snap-state")?);
            let settings = settings(&state, None)?;
            let mut made = origin(&settings, &root, seed, shape)?;
            symlink("../../outside-the-folder", &made.dir.join("ln"))?;
            made.files.push(String::from("ln"));
            made.files.sort();
            let before = tree_digest(&made.dir, false)?;
            let id = format!("t-{seed}");
            let (workspace, copied) = Workspace::snapshot(&state, &made.dir, &id, &path)?;
            let tag = format!("seed {seed} {shape:?}");
            assert_eq!(files_beneath(workspace.path())?, made.files, "{tag}");
            let mut bytes = 0_u64;
            for name in &made.files {
                let (from, to) = (made.dir.join(name), workspace.path().join(name));
                if std::fs::symlink_metadata(&from)?.is_symlink() {
                    assert_eq!(
                        std::fs::read_link(&from)?,
                        std::fs::read_link(&to)?,
                        "{tag}"
                    );
                    continue;
                }
                let want = std::fs::read(&from)?;
                bytes = bytes.saturating_add(u64::try_from(want.len())?);
                assert_eq!(want, std::fs::read(&to)?, "{tag}: {name}");
            }
            let files = made.files.len();
            assert_eq!(copied, Copied { files, bytes }, "{tag}");
            let base = workspace.head()?;
            let diff = workspace.diff(&base)?.ok_or("the work tree is gone")?;
            assert!(diff.text.is_empty(), "{tag}: a fresh copy has no change");
            assert_eq!(
                tree_digest(&made.dir, false)?,
                before,
                "{tag}: origin read-only"
            );
        }
    }
    Ok(())
}

/// saboteur: an untracked symlink shadows a tracked directory name, so a lexical check passes
/// every listed path while a copy that follows the link would write outside the folder. The
/// copy refuses typed, leaves the folder and a sentinel outside it byte-identical, and leaves
/// no workspace (CWE-59, larry#335).
#[cfg(unix)]
#[test]
fn sim_snapshot_refuses_a_symlink_shadowing_a_tracked_directory() -> TestResult {
    let path = path_env()?;
    let (root, state, sentinel) = (
        scratch("shadow-origin")?,
        scratch("shadow-state")?,
        scratch("shadow-sentinel")?,
    );
    let settings = settings(&state, None)?;
    std::fs::write(sentinel.join("keep.txt"), "outside the folder\n")?;
    let outside = tree_digest(&sentinel, false)?;
    std::fs::create_dir_all(root.join("shadow"))?;
    std::fs::write(root.join("shadow/inside.txt"), "tracked\n")?;
    std::fs::write(root.join("top.txt"), "top\n")?;
    settings.seed_origin(&root)?;
    // The index keeps `shadow/inside.txt`; on disk `shadow` becomes an untracked link out.
    std::fs::remove_dir_all(root.join("shadow"))?;
    symlink(&sentinel.to_string_lossy(), &root.join("shadow"))?;
    match Workspace::snapshot(&state, &root, "shadowed", &path) {
        Err(ExecError::Author {
            fault: AuthorFault::Symlink,
            ..
        }) => {}
        Err(other) => return Err(format!("expected the symlink refusal, got {other}").into()),
        Ok(_) => return Err("the copy followed the shadowing symlink".into()),
    }
    assert_eq!(
        tree_digest(&sentinel, false)?,
        outside,
        "a byte outside the folder changed"
    );
    assert!(
        !state.join("ws").join("shadowed").exists(),
        "a refused copy was left behind"
    );
    Ok(())
}

/// A link the folder's ignore rules hide stands where git still tracks files: git lists the
/// tracked paths beneath it and never the link, so a copy that followed it would pull files
/// from outside the folder into the workspace a model reads. The copy refuses typed at every
/// depth the link sits at, no workspace is left, and the folder and the outside stay
/// byte-identical (CWE-59, ADR 0046: a symlink is never followed).
#[cfg(unix)]
#[test]
fn sim_snapshot_never_reads_through_a_linked_directory_of_the_folder() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let names = ["data", "inner", "leaf"];
        let depth = usize::try_from(rng.below(3)?)?.saturating_add(1);
        let linked_at = usize::try_from(rng.below(u64::try_from(depth)?)?)?;
        let nested: PathBuf = names.iter().take(depth).collect();
        let link: PathBuf = names.iter().take(linked_at.saturating_add(1)).collect();
        let (root, state, outside) = (
            scratch("linked-origin")?,
            scratch("linked-state")?,
            scratch("linked-outside")?,
        );
        let settings = settings(&state, None)?;
        let tracked = nested.join("inside.txt");
        std::fs::create_dir_all(root.join(&nested))?;
        std::fs::write(root.join(&tracked), "tracked\n")?;
        settings.seed_origin(&root)?;
        // The same relative path outside the folder holds bytes the copy must never read.
        let secret = outside.join(tracked.strip_prefix(&link)?);
        std::fs::create_dir_all(secret.parent().ok_or("a parent")?)?;
        std::fs::write(&secret, "bytes outside the folder\n")?;
        std::fs::remove_dir_all(root.join(&link))?;
        symlink(&outside.to_string_lossy(), &root.join(&link))?;
        std::fs::write(root.join(".gitignore"), format!("{}\n", link.display()))?;
        let (before, beyond) = (tree_digest(&root, false)?, tree_digest(&outside, false)?);
        match Workspace::snapshot(&state, &root, "linked", &path) {
            Err(ExecError::Author {
                path: refused,
                fault: AuthorFault::Symlink,
            }) => assert_eq!(Path::new(&refused), nested, "seed {seed}: the linked path"),
            Err(other) => return Err(format!("seed {seed}: expected the refusal, {other}").into()),
            Ok((workspace, copied)) => {
                return Err(format!(
                    "seed {seed}: copied {copied:?} through the link: {:?}",
                    files_beneath(workspace.path())?
                )
                .into());
            }
        }
        assert!(
            !state.join("ws").join("linked").exists(),
            "seed {seed}: a refused copy was left behind"
        );
        assert_eq!(
            tree_digest(&root, false)?,
            before,
            "seed {seed}: the folder"
        );
        assert_eq!(
            tree_digest(&outside, false)?,
            beyond,
            "seed {seed}: outside"
        );
    }
    Ok(())
}

/// A person replaced a tracked directory with a file: git still lists the files that were
/// beneath it, and none of them is on disk. Each is a missing entry, so the copy is what is
/// there, whether the new file is listed or ignored (ADR 0046: the copy is the working state).
#[test]
fn sim_snapshot_copies_a_folder_whose_tracked_directory_became_a_file() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let ignored = rng.below(2)? == 1;
        let (root, state) = (scratch("swap-origin")?, scratch("swap-state")?);
        let settings = settings(&state, None)?;
        std::fs::create_dir_all(root.join("d/deep"))?;
        std::fs::write(root.join("d/one.txt"), "one\n")?;
        std::fs::write(root.join("d/deep/two.txt"), "two\n")?;
        std::fs::write(root.join("keep.txt"), "keep\n")?;
        settings.seed_origin(&root)?;
        std::fs::remove_dir_all(root.join("d"))?;
        let now = format!("now a file {seed}\n");
        std::fs::write(root.join("d"), &now)?;
        let mut want = vec![String::from("keep.txt")];
        let mut bytes = u64::try_from("keep\n".len())?;
        if ignored {
            std::fs::write(root.join(".gitignore"), "/d\n")?;
            want.push(String::from(".gitignore"));
            bytes = bytes.saturating_add(u64::try_from("/d\n".len())?);
        } else {
            want.push(String::from("d"));
            bytes = bytes.saturating_add(u64::try_from(now.len())?);
        }
        want.sort();
        let before = tree_digest(&root, false)?;
        let (workspace, copied) = Workspace::snapshot(&state, &root, "swapped", &path)?;
        assert_eq!(files_beneath(workspace.path())?, want, "seed {seed}");
        let files = want.len();
        assert_eq!(copied, Copied { files, bytes }, "seed {seed}");
        assert_eq!(tree_digest(&root, false)?, before, "seed {seed}: read only");
    }
    Ok(())
}

/// Only a missing entry is skipped: a listed name the file system cannot even stat is that
/// refusal, never a copy that silently leaves it out (`stat(2)` `ENAMETOOLONG`, CWE-703).
#[test]
fn sim_a_name_that_cannot_be_stated_is_refused_not_skipped() -> TestResult {
    let (root, into) = (scratch("long-from")?, scratch("long-into")?);
    let long = PathBuf::from("n".repeat(300));
    let limits = Limits {
        files: MAX_FILES,
        bytes: MAX_BYTES,
        depth: MAX_DEPTH,
    };
    match copy_entries(&root, &into, &[long], limits) {
        Err(ExecError::Workspace(cause)) => {
            assert_eq!(cause.kind(), std::io::ErrorKind::InvalidFilename);
        }
        other => return Err(format!("expected the name refusal, got {other:?}").into()),
    }
    Ok(())
}

/// A folder past each bound is refused with that bound named, leaves no workspace and leaves
/// the folder alone; the next, smaller one is served.
#[test]
fn sim_snapshot_refuses_past_each_bound_and_recovers() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let (root, state) = (scratch("bound-origin")?, scratch("bound-state")?);
        let settings = settings(&state, None)?;
        let at = usize::try_from(seed)?.checked_rem(Shape::ALL.len());
        let shape = at
            .and_then(|index| Shape::ALL.get(index))
            .ok_or("a shape")?;
        let made = origin(&settings, &root, seed, *shape)?;
        let before = tree_digest(&made.dir, false)?;
        let cases = [
            (
                Limits {
                    files: 1,
                    bytes: u64::MAX,
                    depth: MAX_DEPTH,
                },
                "files",
                1,
            ),
            (
                Limits {
                    files: MAX_FILES,
                    bytes: 1,
                    depth: MAX_DEPTH,
                },
                "bytes",
                1,
            ),
        ];
        for (limits, what, limit) in cases {
            let id = format!("over-{what}-{seed}");
            match Workspace::snapshot_within(&state, &made.dir, &id, &path, limits) {
                Err(ExecError::SnapshotTooLarge {
                    what: named,
                    limit: bound,
                }) => {
                    assert_eq!((named, bound), (what, limit), "seed {seed}");
                }
                Err(other) => return Err(format!("seed {seed} {what}: {other}").into()),
                Ok(_) => return Err(format!("seed {seed} {what}: a copy was made").into()),
            }
            assert!(
                !state.join("ws").join(&id).exists(),
                "seed {seed}: {what} leaves a copy"
            );
        }
        let (served, _) = Workspace::snapshot(&state, &made.dir, "after", &path)?;
        assert!(
            served.path().is_dir(),
            "seed {seed}: the next folder is served"
        );
        assert_eq!(tree_digest(&made.dir, false)?, before, "seed {seed}");
    }
    // Directory depth: a plain folder one level past the bound.
    let (root, state) = (scratch("deep-origin")?, scratch("deep-state")?);
    let mut deep = root.to_path_buf();
    for _ in 0..=MAX_DEPTH {
        deep.push("d");
    }
    std::fs::create_dir_all(&deep)?;
    std::fs::write(deep.join("leaf.txt"), "leaf")?;
    match Workspace::snapshot(&state, &root, "deep", &path) {
        Err(ExecError::SnapshotTooLarge {
            what: "directory levels",
            limit,
        }) => {
            assert_eq!(limit, u64::try_from(MAX_DEPTH)?);
        }
        Err(other) => return Err(format!("depth: {other}").into()),
        Ok(_) => return Err("depth: a copy was made".into()),
    }
    assert!(!state.join("ws").join("deep").exists());
    Ok(())
}

/// A workspace whose process was killed (no destructor ran) is swept by the next start, and
/// a live run's workspace beside it is kept.
#[test]
fn sim_snapshot_workspace_is_swept_after_a_kill() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let (root, state) = (scratch("sweep-origin")?, scratch("sweep-state")?);
        let settings = settings(&state, None)?;
        let made = origin(&settings, &root, seed, Shape::Git)?;
        let dead = ManuallyDrop::new(Workspace::snapshot(&state, &made.dir, "dead", &path)?.0);
        let live = Workspace::snapshot(&state, &made.dir, "live", &path)?.0;
        assert!(dead.path().is_dir(), "seed {seed}");
        sweep(&state, &[String::from("live")], |_| Ok(false))?;
        assert!(
            !state.join("ws").join("dead").exists(),
            "seed {seed}: dead copy swept"
        );
        assert!(live.path().is_dir(), "seed {seed}: the live copy is kept");
    }
    Ok(())
}

/// The summary a person reads never carries a control or a bidi character from a file name,
/// and a newline in a name does not forge a line.
#[test]
fn sim_snapshot_summary_neutralizes_hostile_names() -> TestResult {
    let path = path_env()?;
    let (root, state) = (scratch("hostile-origin")?, scratch("hostile-state")?);
    let settings = settings(&state, None)?;
    let made = origin(&settings, &root, 3, Shape::Plain)?;
    let (workspace, _) = Workspace::snapshot(&state, &made.dir, "hostile", &path)?;
    let name = "x\u{1b}[2J\u{202e}gnp\nforged: 99 files changed";
    std::fs::write(workspace.path().join(name), "payload\n")?;
    let diff = workspace
        .diff(&workspace.head()?)?
        .ok_or("the work tree is gone")?;
    let lines = summary(&diff);
    assert_eq!(
        lines.len(),
        2,
        "one totals line and one file line: {lines:?}"
    );
    let bad = |character: char| {
        character.is_control()
            || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    };
    assert!(lines.iter().all(|line| !line.chars().any(bad)), "{lines:?}");
    // A diff that names the raw bytes (a hostile model cannot make git do this, a forged
    // patch could) is made plain by the summary itself.
    let mut sink = DiffSink::new();
    sink.push(
        b"diff --git a/x\x1b]0;t\x07y b/x\x1b]0;t\x07y\nnew file mode 100644\n@@ -0,0 +1 @@\n+z\n",
    )?;
    let forged = summary(&sink.finish());
    assert_eq!(
        forged.get(1).map(String::as_str),
        Some("x?]0;t?y  new  +1 -0")
    );
    Ok(())
}

/// A program named in the folder's own git config never runs: the positive control proves the
/// config is live (git status runs the monitor), and the snapshot leaves no mark.
#[test]
fn sim_snapshot_folder_config_cannot_run_a_program() -> TestResult {
    let path = path_env()?;
    let (root, state, hooks) = (
        scratch("fsm-origin")?,
        scratch("fsm-state")?,
        scratch("fsm-hooks")?,
    );
    let settings = settings(&state, None)?;
    let made = origin(&settings, &root, 5, Shape::Git)?;
    let (script, marker) = (hooks.join("monitor.sh"), hooks.join("marker"));
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf hit >> '{}'\n", marker.display()),
    )?;
    let chmod = Command::new("chmod").arg("755").arg(&script).status()?;
    assert!(chmod.success());
    let config = made.dir.join(".git").join("config");
    let mut text = std::fs::read_to_string(&config)?;
    text = format!("{text}[core]\n\tfsmonitor = {}\n", script.display());
    std::fs::write(&config, text)?;
    let mut control = Command::new("git");
    control
        .args(["status", "--porcelain"])
        .current_dir(&made.dir)
        .env_clear()
        .env("PATH", &path)
        .env("HOME", state.as_os_str());
    bound_git(&mut control, &made.dir);
    assert!(control.status()?.success());
    assert!(
        marker.exists(),
        "control: git runs the monitor the folder names"
    );
    std::fs::remove_file(&marker)?;
    let before = tree_digest(&made.dir, false)?;
    let (workspace, _) = Workspace::snapshot(&state, &made.dir, "fsm", &path)?;
    assert!(workspace.path().is_dir());
    assert!(!marker.exists(), "the folder's own config ran a program");
    assert_eq!(tree_digest(&made.dir, false)?, before);
    Ok(())
}

/// A repository git cannot read is refused with git's own refusal; it is not walked, which
/// would copy the build output its ignore rules hide.
#[test]
fn sim_snapshot_broken_repository_is_refused_not_walked() -> TestResult {
    let path = path_env()?;
    let (root, state) = (scratch("broken-origin")?, scratch("broken-state")?);
    let settings = settings(&state, None)?;
    let made = origin(&settings, &root, 9, Shape::Git)?;
    std::fs::write(made.dir.join(".git").join("HEAD"), "not a ref\n")?;
    match Workspace::snapshot(&state, &made.dir, "broken", &path) {
        Err(ExecError::Git { step, .. }) => assert!(step == "rev-parse" || step == "ls-files"),
        Err(other) => return Err(format!("expected a git refusal, got {other}").into()),
        Ok(_) => return Err("a broken repository was copied".into()),
    }
    assert!(!state.join("ws").join("broken").exists());
    Ok(())
}
