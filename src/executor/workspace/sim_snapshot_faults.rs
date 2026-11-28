//! Seeded adversarial fault injection through the snapshot, its copy, its apply and its summary
//! (larry#575): a listing past its byte bound, a damaged index, hostile names and entries, the
//! walk's own bounds, every apply fault and a diff too long to name. Each answer is asserted as
//! its exact typed error or exact lines, and one seed drives every draw.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use lgwks_std::seeded::Seeded;

use super::super::{
    ApplyFault, Copied, Limits, MAX_BYTES, MAX_DEPTH, MAX_LISTING_BYTES, copy_entries, summary,
    walk,
};
use crate::executor::ExecError;
use crate::executor::sim_tree::{Scratch, files_beneath, path_env, scratch};
use crate::executor::workspace::{Diff, DiffSink, Workspace};
use crate::sim_error::SimError;

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The seeds the fault models replay.
const SEEDS: [u64; 3] = [
    0x5EED_0000_0000_0041,
    0x5EED_0000_0000_0042,
    0xDEAD_BEEF_CAFE_0041,
];

/// A folder to copy and a state directory, both removed when their guards drop.
fn rooms() -> Result<(Scratch, Scratch), SimError> {
    Ok((scratch("snap-fault-origin")?, scratch("snap-fault-state")?))
}

/// Runs git with `arguments` inside `directory` under only the test's PATH.
fn git(directory: &Path, state: &Path, arguments: &[&str]) -> TestResult {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env_clear()
        .env("PATH", path_env()?)
        .env("HOME", state)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {arguments:?} failed: {status}").into())
    }
}

/// The bounds of a run with `files` entries and every other bound standard.
const fn files_bound(files: usize) -> Limits {
    Limits {
        files,
        bytes: MAX_BYTES,
        depth: MAX_DEPTH,
    }
}

/// Names the git listing cannot hold are refused by that bound, the listing child is ended, and
/// no workspace is left (CWE-770, larry#243). The names are long and nested so the bound is
/// passed with far fewer files than the file bound allows.
#[test]
fn sim_a_listing_past_its_byte_bound_is_refused_and_leaves_nothing() -> TestResult {
    let (root, state) = rooms()?;
    git(&root, &state, &["init", "--quiet"])?;
    let mut deep = root.to_path_buf();
    for _ in 0..3 {
        deep.push("d".repeat(200));
    }
    std::fs::create_dir_all(&deep)?;
    let entry_bytes: u64 = 3 * 201 + 180 + 1;
    let wanted = MAX_LISTING_BYTES
        .checked_div(entry_bytes)
        .ok_or("an entry has bytes")?
        + 2;
    for index in 0..wanted {
        std::fs::File::create(deep.join(format!("{index:0>180}")))?;
    }
    match Workspace::snapshot(&state, &root, "long", &path_env()?) {
        Err(ExecError::SnapshotTooLarge { what, limit }) => {
            assert_eq!((what, limit), ("bytes of file names", MAX_LISTING_BYTES));
        }
        other => return Err(format!("not the listing bound: {:?}", other.err()).into()),
    }
    assert!(!state.join("ws").join("long").exists(), "a copy was left");
    Ok(())
}

/// A repository whose index git cannot read is refused at the listing, never walked, since a
/// walk would copy what its ignore rules hide (CWE-754, larry#243).
#[test]
fn sim_a_damaged_index_is_a_failed_listing_never_a_walk() -> TestResult {
    for seed in SEEDS {
        let (root, state) = rooms()?;
        git(&root, &state, &["init", "--quiet"])?;
        let mut rng = Seeded::from_seed(seed);
        let junk = vec![u8::try_from(rng.below(256)?)?; 64];
        std::fs::write(root.join(".git").join("index"), junk)?;
        std::fs::write(root.join("a.txt"), "a\n")?;
        match Workspace::snapshot(&state, &root, "damaged", &path_env()?) {
            Err(ExecError::Git { step, detail }) => {
                assert_eq!(step, "ls-files", "seed {seed:#x}");
                assert_eq!(detail, "git could not list the folder", "seed {seed:#x}");
            }
            other => return Err(format!("seed {seed:#x}: {:?}", other.err()).into()),
        }
        assert!(!state.join("ws").join("damaged").exists(), "seed {seed:#x}");
    }
    Ok(())
}

/// A listed name that is not UTF-8 is refused as bad data, never lossily renamed into a copy
/// (CWE-176). Only Linux file systems hold such a name; elsewhere the arm does not exist.
#[test]
fn sim_a_listed_name_that_is_not_utf8_is_refused() -> TestResult {
    let (root, state) = rooms()?;
    git(&root, &state, &["init", "--quiet"])?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        std::fs::write(root.join(std::ffi::OsStr::from_bytes(b"f-\xff")), "x\n")?;
        match Workspace::snapshot(&state, &root, "odd", &path_env()?) {
            Err(ExecError::Workspace(cause)) => assert_eq!(cause.kind(), ErrorKind::InvalidData),
            other => return Err(format!("not the name refusal: {:?}", other.err()).into()),
        }
    }
    Ok(())
}

/// A bare repository is no work tree, so its folder is walked and copied as the files it holds
/// (git-rev-parse(1) `--is-inside-work-tree`, ADR 0046).
#[test]
fn sim_a_bare_repository_is_walked_as_a_folder() -> TestResult {
    let (root, state) = rooms()?;
    git(&root, &state, &["init", "--bare", "--quiet"])?;
    let (workspace, copied) = Workspace::snapshot(&state, &root, "bare", &path_env()?)?;
    let want = files_beneath(&root)?;
    assert_eq!(files_beneath(workspace.path())?, want);
    assert_eq!(copied.files, want.len());
    assert_eq!(
        std::fs::read(workspace.path().join("HEAD"))?,
        std::fs::read(root.join("HEAD"))?
    );
    Ok(())
}

/// A walked folder copies its files and links but no `.git` entry of any case and no fifo,
/// which a copy would block on forever (CWE-400, CWE-178, larry#343).
#[cfg(unix)]
#[test]
fn sim_a_walk_skips_every_git_entry_and_every_fifo() -> TestResult {
    let (root, _state) = rooms()?;
    std::fs::create_dir_all(root.join("sub").join(".Git"))?;
    std::fs::write(root.join("sub").join(".Git").join("config"), "x\n")?;
    std::fs::create_dir_all(root.join("other"))?;
    std::fs::write(root.join("other").join(".git"), "gitdir: nowhere\n")?;
    std::fs::write(root.join("sub").join("kept"), "kept\n")?;
    std::os::unix::fs::symlink("kept", root.join("sub").join("link"))?;
    let made = Command::new("mkfifo").arg(root.join("pipe")).status()?;
    assert!(made.success(), "the fifo is made");
    let found = walk(&root, Limits::STANDARD)?;
    assert_eq!(
        found,
        [PathBuf::from("sub/kept"), PathBuf::from("sub/link")]
    );
    Ok(())
}

/// A walk past the file bound is refused whether one directory holds too many names or the
/// files found and the directories still to visit add up past it (CWE-770, larry#243).
#[test]
fn sim_a_walk_past_the_file_bound_is_refused_in_one_folder_or_across_them() -> TestResult {
    let (root, _state) = rooms()?;
    for name in ["x", "y", "z"] {
        std::fs::write(root.join(name), name)?;
    }
    match walk(&root, files_bound(2)) {
        Err(ExecError::SnapshotTooLarge { what, limit }) => {
            assert_eq!((what, limit), ("files", 2));
        }
        other => return Err(format!("one folder: {other:?}").into()),
    }
    let (spread, _state) = rooms()?;
    for name in ["a", "b"] {
        std::fs::create_dir(spread.join(name))?;
    }
    for name in ["one", "two"] {
        std::fs::write(spread.join("b").join(name), name)?;
    }
    match walk(&spread, files_bound(2)) {
        Err(ExecError::SnapshotTooLarge { what, limit }) => {
            assert_eq!((what, limit), ("files", 2));
        }
        other => return Err(format!("across folders: {other:?}").into()),
    }
    Ok(())
}

/// A listed path that leaves the folder or names a `.git` is refused before anything is read,
/// so a hostile listing never reaches outside (CWE-22, CWE-59, ADR 0046).
#[test]
fn sim_a_listed_path_that_leaves_the_folder_is_refused() -> TestResult {
    let (root, into) = rooms()?;
    for hostile in [
        "../outside",
        "/etc/passwd",
        ".git/config",
        "a/./../../b",
        "x/.GIT/y",
    ] {
        let listed = [PathBuf::from(hostile)];
        match copy_entries(&root, &into, &listed, Limits::STANDARD) {
            Err(ExecError::Workspace(cause)) => {
                assert_eq!(cause.kind(), ErrorKind::InvalidInput, "{hostile}");
                assert_eq!(
                    cause.to_string(),
                    format!("a listed path leaves the folder: {hostile}")
                );
            }
            other => return Err(format!("{hostile}: {other:?}").into()),
        }
    }
    assert!(files_beneath(&into)?.is_empty(), "nothing was written");
    Ok(())
}

/// A listed directory (git lists a submodule as one) and a listed missing file are skipped, and
/// the file bound counts only what is copied (CWE-770, larry#243).
#[test]
fn sim_a_listed_directory_is_skipped_and_the_file_bound_counts_copies() -> TestResult {
    let (root, into) = rooms()?;
    std::fs::create_dir(root.join("module"))?;
    std::fs::write(root.join("one"), "1")?;
    std::fs::write(root.join("two"), "22")?;
    let listed = ["module", "gone", "one"].map(PathBuf::from);
    let copied = copy_entries(&root, &into, &listed, files_bound(1))?;
    assert_eq!(copied, Copied { files: 1, bytes: 1 });
    assert_eq!(files_beneath(&into)?, ["one"]);
    let (second, into) = rooms()?;
    std::fs::write(second.join("one"), "1")?;
    std::fs::write(second.join("two"), "22")?;
    match copy_entries(
        &second,
        &into,
        &["one", "two"].map(PathBuf::from),
        files_bound(1),
    ) {
        Err(ExecError::SnapshotTooLarge { what, limit }) => {
            assert_eq!((what, limit), ("files", 1));
        }
        other => return Err(format!("past the bound: {other:?}").into()),
    }
    Ok(())
}

/// A PATH with no git, or a workspace whose tree a file already holds, is a typed refusal and
/// leaves no copy (CWE-426, CWE-754, larry#243).
#[test]
fn sim_a_snapshot_whose_git_cannot_start_or_init_is_typed() -> TestResult {
    let (root, state) = rooms()?;
    std::fs::write(root.join("a"), "a")?;
    let absent = state.join("no-bin").display().to_string();
    match Workspace::snapshot(&state, &root, "nogit", &absent) {
        Err(ExecError::Spawn { argv0, source }) => {
            assert_eq!(
                (argv0.as_str(), source.kind()),
                ("git", ErrorKind::NotFound)
            );
        }
        other => return Err(format!("no git: {:?}", other.err()).into()),
    }
    let held = state.join("ws").join("held");
    std::fs::create_dir_all(&held)?;
    std::fs::write(held.join("tree"), "a file")?;
    match Workspace::snapshot(&state, &root, "held", &path_env()?) {
        Err(ExecError::Git { step, .. }) => assert_eq!(step, "init"),
        other => return Err(format!("a held tree: {:?}", other.err()).into()),
    }
    assert!(!held.exists(), "the refused copy was removed");
    Ok(())
}

/// The change a session made goes back only whole and unaltered: a cut diff, a changed digest,
/// an unwritable patch, a missing folder and a patch git refuses are each their own fault, and
/// the folder is unchanged by every one of them (CWE-345, larry#243).
#[test]
fn sim_every_apply_fault_is_typed_and_leaves_the_folder_unchanged() -> TestResult {
    for seed in SEEDS {
        let (root, state) = rooms()?;
        std::fs::write(root.join("a.txt"), format!("{seed:x}\n"))?;
        let (workspace, _) = Workspace::snapshot(&state, &root, "apply", &path_env()?)?;
        let base = workspace.head()?;
        std::fs::write(workspace.path().join("a.txt"), "changed\n")?;
        let mut diff = workspace.diff(&base)?.ok_or("the work tree is gone")?;
        let sealed = diff.digest.clone();

        diff.truncated = true;
        assert!(matches!(
            workspace.apply(&root, &diff),
            Err(ApplyFault::Truncated)
        ));
        diff.truncated = false;
        diff.digest = "0".repeat(64);
        assert!(matches!(
            workspace.apply(&root, &diff),
            Err(ApplyFault::DigestMismatch)
        ));
        diff.digest = sealed;

        match workspace.apply(&state.join("absent"), &diff) {
            Err(ApplyFault::Io(cause)) => assert_eq!(cause.kind(), ErrorKind::NotFound),
            other => return Err(format!("seed {seed:#x}: missing folder: {other:?}").into()),
        }
        let patch = workspace.root.join("apply.patch");
        std::fs::remove_file(&patch)?;
        std::fs::create_dir(&patch)?;
        match workspace.apply(&root, &diff) {
            Err(ApplyFault::Io(cause)) => assert_eq!(cause.kind(), ErrorKind::IsADirectory),
            other => return Err(format!("seed {seed:#x}: held patch: {other:?}").into()),
        }
        std::fs::remove_dir(&patch)?;
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt"))?,
            format!("{seed:x}\n"),
            "seed {seed:#x}: refused applies wrote nothing"
        );

        workspace.apply(&root, &diff)?;
        match workspace.apply(&root, &diff) {
            Err(ApplyFault::Git { detail }) => assert!(
                detail.contains("a.txt"),
                "seed {seed:#x}: git names the file: {detail:?}"
            ),
            other => return Err(format!("seed {seed:#x}: applied twice: {other:?}").into()),
        }
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt"))?,
            "changed\n",
            "seed {seed:#x}: the refused second apply changed nothing"
        );
    }
    Ok(())
}

/// A diff text as the review reads it.
fn diff_of(text: &str) -> Result<Diff, std::io::Error> {
    let mut sink = DiffSink::new();
    sink.push(text.as_bytes())?;
    Ok(sink.finish())
}

/// A diff over the named-file bound names the first twelve and counts the rest, counts removed
/// and added lines only inside hunks, and ignores what precedes the first header (larry#243).
#[test]
fn sim_a_long_summary_names_twelve_files_and_counts_the_rest() -> TestResult {
    for seed in SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let extra = usize::try_from(rng.below(5)?)?.saturating_add(1);
        let total = 12 + extra;
        let mut text = String::from("+stray line before any file\n-another\n");
        text.push_str("diff --git header-without-a-b-side\n");
        text.push_str("--- not a hunk line\n@@ -1 +0,0 @@\n-gone\n");
        text.extend((1..total).map(|index| {
            format!(
                "diff --git a/f{index} b/f{index}\nindex 0..1\n--- a/f{index}\n+++ b/f{index}\n@@ -1 +1 @@\n-old\n+new\n"
            )
        }));
        let lines = summary(&diff_of(&text)?);
        let added = total - 1;
        assert_eq!(
            lines.first().map(String::as_str),
            Some(format!("{total} files changed, +{added} -{total}").as_str()),
            "seed {seed:#x}"
        );
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some("header-without-a-b-side  changed  +0 -1"),
            "seed {seed:#x}"
        );
        assert_eq!(
            lines.get(2).map(String::as_str),
            Some("f1  changed  +1 -1"),
            "seed {seed:#x}"
        );
        assert_eq!(lines.len(), 14, "seed {seed:#x}: totals, twelve, rest");
        assert_eq!(
            lines.last().cloned(),
            Some(format!("… and {extra} more")),
            "seed {seed:#x}"
        );
    }
    assert_eq!(summary(&diff_of("")?), ["0 files changed, +0 -0"]);
    Ok(())
}
