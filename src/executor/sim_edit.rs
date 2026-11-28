//! Seeded simulations of a session's file read (larry#586) and anchored edit
//! (larry#606): the digest is the file's BLAKE3 for every generated body, an
//! edit applied then reverted gives the original bytes, and every path, file,
//! digest or anchor that cannot be taken gets its own typed refusal with the
//! file unchanged. Source: CWE-59 (a link that leaves the tree), CWE-367 (a
//! check that another writer can pass between) and Codelia's three-level edit
//! match with a content-hash precondition.

use std::io::Write as _;
use std::path::Path;

use lgwks_std::seeded::Seeded;

use super::{
    Edit, EditError, EditReceipt, Window, apply_edit, apply_edit_with, digest_of, read_file,
    read_file_with,
};
use crate::executor::sim_tree::{Scratch, scratch};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each generated simulation runs.
const SEEDS: u64 = 512;

/// The pieces a generated body is built from: line ends, a tab and non-ASCII.
const PIECES: [&str; 4] = ["alpha\n", "beta\r\n", "\tgamma", " é "];

/// A scratch snapshot holding `files`: each path with its bytes.
fn snapshot(label: &str, files: &[(&str, &[u8])]) -> Result<Scratch, SimError> {
    let root = scratch(label)?;
    for (path, bytes) in files.iter().copied() {
        std::fs::write(root.join(path), bytes)?;
    }
    Ok(root)
}

/// A generated body of up to 39 pieces.
fn body(rng: &mut Seeded) -> Result<String, SimError> {
    let mut out = String::new();
    for _ in 0..rng.below(40)? {
        let pick = rng.index(PIECES.len())?;
        out.push_str(PIECES.get(pick).ok_or(SimError::Missing("piece"))?);
    }
    Ok(out)
}

/// The text of the file `path` under `root`.
fn text(root: &Path, path: &str) -> Result<String, SimError> {
    Ok(String::from_utf8(std::fs::read(root.join(path))?)?)
}

/// The edit of `path` from `old` to `new` that last saw the file as `seen`.
fn edit(path: &str, old: &str, new: &str, seen: &str) -> Edit {
    Edit {
        path: path.to_owned(),
        old: old.to_owned(),
        new: new.to_owned(),
        expect_digest: digest_of(seen),
    }
}

/// `before` written at `path`, then edited from `old` to `new`.
fn applied(
    root: &Path,
    path: &str,
    before: &str,
    old: &str,
    new: &str,
) -> Result<Result<EditReceipt, EditError>, SimError> {
    std::fs::write(root.join(path), before)?;
    Ok(apply_edit(root, &edit(path, old, new, before)))
}

/// For every seed, the read returns the file whole and the BLAKE3 hex of its
/// bytes, so the digest names exactly what the model was shown.
#[test]
fn sim_a_read_shows_the_file_and_its_digest_for_every_seed() -> TestResult {
    let root = scratch("sim-read-digest")?;
    for seed in 0..SEEDS {
        let body = body(&mut Seeded::from_seed(seed))?;
        let name = format!("read-{seed}.txt");
        std::fs::write(root.join(&name), body.as_bytes())?;
        let view = read_file(&root, &name)?;
        assert_eq!(view.text, body, "seed {seed}: the whole file");
        assert_eq!(
            view.digest,
            lgwks_std::hash::blake3(body.as_bytes()).to_hex(),
            "seed {seed}: the digest of its bytes"
        );
    }
    Ok(())
}

/// A path outside the snapshot is refused by the read and by the edit, and
/// nothing is written.
#[test]
fn sim_a_path_outside_the_snapshot_is_refused() -> TestResult {
    let root = snapshot("sim-read-outside", &[("inside.txt", b"kept\n")])?;
    for outside in [
        "../escape.txt",
        "/etc/hosts",
        "sub/../../escape.txt",
        ".git/hooks/x",
        "",
    ] {
        match read_file(&root, outside) {
            Err(EditError::PathOutside { path }) => assert_eq!(path, outside),
            other => return Err(format!("{outside:?}: wrong outcome: {other:?}").into()),
        }
        match apply_edit(&root, &edit(outside, "kept", "gone", "kept\n")) {
            Err(EditError::PathOutside { path }) => assert_eq!(path, outside),
            other => return Err(format!("{outside:?}: wrong outcome: {other:?}").into()),
        }
    }
    assert_eq!(text(&root, "inside.txt")?, "kept\n");
    Ok(())
}

/// A link in the tree is refused as outside, whether it is the file itself or a
/// directory on the way to it, and its target is never written.
#[cfg(unix)]
#[test]
fn sim_a_link_in_the_tree_is_refused() -> TestResult {
    let outside = snapshot("sim-read-link-target", &[("secret.txt", b"secret\n")])?;
    let root = scratch("sim-read-link")?;
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("file.txt"))?;
    std::os::unix::fs::symlink(&*outside, root.join("folder"))?;
    for path in ["file.txt", "folder/secret.txt"] {
        match read_file(&root, path) {
            Err(EditError::PathOutside { .. }) => {}
            other => return Err(format!("{path}: wrong outcome: {other:?}").into()),
        }
        match apply_edit(&root, &edit(path, "secret", "gone", "secret\n")) {
            Err(EditError::PathOutside { .. }) => {}
            other => return Err(format!("{path}: wrong outcome: {other:?}").into()),
        }
    }
    assert_eq!(text(&outside, "secret.txt")?, "secret\n");
    Ok(())
}

/// A file that grows after its length was checked is read no further than one
/// byte past the bound (CWE-367): grown past it, the read is refused; grown to
/// within it, the whole grown file is read.
#[test]
fn sim_a_file_grown_after_the_stat_is_refused_too_large() -> TestResult {
    let root = scratch("sim-read-grown")?;
    for seed in 0..64 {
        let mut rng = Seeded::from_seed(seed);
        let start = rng.index(1024)?;
        let past = seed % 2 == 0;
        let end = if past {
            262_145_usize.saturating_add(rng.index(4096)?)
        } else {
            262_144_usize.saturating_sub(rng.index(4096)?)
        };
        let name = format!("grown-{seed}.txt");
        let target = root.join(&name);
        std::fs::write(&target, "k".repeat(start))?;
        let grown = read_file_with(&root, &name, &mut |_| {
            let mut file = std::fs::OpenOptions::new().append(true).open(&target)?;
            file.write_all("k".repeat(end.saturating_sub(start)).as_bytes())
        });
        match grown {
            Err(EditError::TooLarge { len: 262_145, .. }) if past => {}
            Ok(view) if !past => assert_eq!(view.text.len(), end, "seed {seed}: the grown file"),
            other => {
                let kept = other.map(|view| view.text.len());
                return Err(format!("seed {seed}: grown to {end} gave {kept:?}").into());
            }
        }
    }
    Ok(())
}

/// Every bound of a read and an edit admits its limit and refuses one past it,
/// and what is not a UTF-8 regular file gets its own error.
#[test]
fn sim_each_read_bound_admits_its_limit_and_refuses_past_it() -> TestResult {
    let large = "k\n".repeat(131_072);
    let past = format!("{large}k");
    let full = format!("x{}", "k".repeat(262_143));
    let root = snapshot(
        "sim-read-bounds",
        &[
            ("large.txt", large.as_bytes()),
            ("past.txt", past.as_bytes()),
            ("full.txt", full.as_bytes()),
            ("binary.bin", &[0xff, 0xfe]),
        ],
    )?;
    std::fs::create_dir(root.join("folder"))?;
    assert_eq!(read_file(&root, "large.txt")?.text.len(), 262_144);
    assert!(matches!(
        read_file(&root, "past.txt"),
        Err(EditError::TooLarge { len: 262_145, .. })
    ));
    assert!(matches!(
        apply_edit(&root, &edit("full.txt", "x", "xy", &full)),
        Err(EditError::TooLarge { len: 262_145, .. })
    ));
    assert_eq!(
        text(&root, "full.txt")?,
        full,
        "a refused edit writes nothing"
    );
    apply_edit(&root, &edit("full.txt", "x", "y", &full))?;
    assert!(matches!(
        read_file(&root, "binary.bin"),
        Err(EditError::NotText { .. })
    ));
    assert!(matches!(
        read_file(&root, "folder"),
        Err(EditError::NotAFile { .. })
    ));
    assert!(matches!(
        read_file(&root, "absent.txt"),
        Err(EditError::Io { .. })
    ));
    Ok(())
}

/// For every seed, an edit applied then reverted gives the original bytes, and
/// each receipt names the digests on both sides.
#[test]
fn sim_an_edit_applied_then_reverted_gives_the_original_bytes() -> TestResult {
    let root = scratch("sim-edit-round-trip")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 32));
        let anchor = format!("anchor-{seed}");
        let replacement = format!("replaced-{seed}");
        let before = format!("{}{anchor}{}", body(&mut rng)?, body(&mut rng)?);
        let after = before.replace(&anchor, &replacement);
        let path = format!("edit-{seed}.txt");
        let first = applied(&root, &path, &before, &anchor, &replacement)??;
        assert_eq!(text(&root, &path)?, after, "seed {seed}: the edit landed");
        assert_eq!(first.before_digest, digest_of(&before), "seed {seed}");
        assert_eq!(first.after_digest, digest_of(&after), "seed {seed}");
        let back = apply_edit(&root, &edit(&path, &replacement, &anchor, &after))?;
        assert_eq!(
            text(&root, &path)?,
            before,
            "seed {seed}: the original bytes"
        );
        assert_eq!(back.after_digest, first.before_digest, "seed {seed}");
    }
    Ok(())
}

/// For every seed, a digest of anything but the file's bytes is refused naming
/// the digest it holds, and the file is unchanged.
#[test]
fn sim_a_stale_digest_is_refused_and_the_file_is_unchanged() -> TestResult {
    let root = scratch("sim-edit-stale")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(2 << 32));
        let before = format!("kept-{seed}\n{}", body(&mut rng)?);
        let path = format!("stale-{seed}.txt");
        std::fs::write(root.join(&path), &before)?;
        let stale = edit(&path, "kept", "gone", &format!("{before}other"));
        match apply_edit(&root, &stale) {
            Err(EditError::StaleDigest { found, .. }) => {
                assert_eq!(found, digest_of(&before), "seed {seed}: the digest on disk");
            }
            other => return Err(format!("seed {seed}: wrong outcome: {other:?}").into()),
        }
        assert_eq!(text(&root, &path)?, before, "seed {seed}: unchanged");
    }
    Ok(())
}

/// Each anchor level replaces its one place: exact bytes, whitespace-trimmed
/// lines, then the block the first and last lines bound; line endings are kept
/// and an empty replacement removes its lines.
#[test]
fn sim_each_anchor_level_replaces_its_one_place() -> TestResult {
    let root = scratch("sim-edit-levels")?;
    for (before, old, new, after) in [
        ("alpha\nbeta\n", "beta", "BETA", "alpha\nBETA\n"),
        (
            "fn main() {\n    let kept = 1;\n}\n",
            "  let kept = 1;  ",
            "    let kept = 2;",
            "fn main() {\n    let kept = 2;\n}\n",
        ),
        (
            "top\nfn total() {\n    let sum = 0;\n    sum\n}\nend\n",
            "\nfn total() {\n  another middle\n  and more\n}\n\n",
            "fn total() { 0 }\n",
            "top\nfn total() { 0 }\nend\n",
        ),
        (
            "one\r\ntwo\r\nthree\r\n",
            "one\ntwo",
            "uno\ndos",
            "uno\r\ndos\r\nthree\r\n",
        ),
        ("one\r\ntwo", "one\ntwo\n", "uno\ndos\n", "uno\r\ndos"),
        ("top\n  last", "last  ", "end", "top\nend"),
        ("top\n    middle\nbottom\n", "middle  ", "", "top\nbottom\n"),
        ("top\n  gap\nbottom\n", "gap  ", "\n", "top\n\nbottom\n"),
    ] {
        applied(&root, "level.txt", before, old, new)??;
        assert_eq!(
            text(&root, "level.txt")?,
            after,
            "{old:?} to {new:?} in {before:?}"
        );
    }
    Ok(())
}

/// An anchor that sits nowhere is refused: an empty or all-blank one, and a
/// block with only one of its ends in the file.
#[test]
fn sim_an_anchor_that_matches_nothing_is_refused() -> TestResult {
    let root = scratch("sim-edit-no-match")?;
    let before = "alpha\n\nbeta\ngamma\n";
    for old in [
        "",
        "  \t",
        "delta",
        "alpha\nBETA\ndelta",
        "zeta\nbeta\ngamma",
    ] {
        match applied(&root, "none.txt", before, old, "new")? {
            Err(EditError::NoMatch { path }) => assert_eq!(path, "none.txt"),
            other => return Err(format!("{old:?}: wrong outcome: {other:?}").into()),
        }
        assert_eq!(text(&root, "none.txt")?, before, "{old:?}: unchanged");
    }
    Ok(())
}

/// An anchor that sits in more than one place is refused at the first level
/// that finds it, naming at most sixteen candidate lines.
#[test]
fn sim_an_ambiguous_anchor_is_refused_naming_its_lines() -> TestResult {
    let root = scratch("sim-edit-ambiguous")?;
    let many = "same\n".repeat(20);
    let indented = "  same\n".repeat(20);
    let sixteen: Vec<usize> = (1..=16).collect();
    for (before, old, lines) in [
        (
            "one\nrepeat\ntwo\nrepeat\nthree\n",
            "repeat",
            vec![2_usize, 4],
        ),
        ("aaa\n", "aa", vec![1, 1]),
        ("é\néé\n", "é", vec![1, 2, 2]),
        ("  x = 1\ny\n\tx = 1\n", "x = 1 ", vec![1, 3]),
        ("a\nm\nz\na\nn\nz\n", "a\nother\nz", vec![1, 4]),
        (many.as_str(), "same", sixteen.clone()),
        (indented.as_str(), "same  ", sixteen),
    ] {
        match applied(&root, "twice.txt", before, old, "once")? {
            Err(EditError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, lines, "{old:?} in {before:?}");
            }
            other => return Err(format!("{old:?}: wrong outcome: {other:?}").into()),
        }
        assert_eq!(text(&root, "twice.txt")?, before, "{old:?}: unchanged");
    }
    Ok(())
}

/// An edit keeps the file's mode, so a script stays executable, leaves no
/// temporary file and leaves a hard link of the old file as it was.
#[cfg(unix)]
#[test]
fn sim_an_edit_keeps_the_mode_and_replaces_the_file_whole() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let root = snapshot("sim-edit-mode", &[("run.sh", b"echo one\n")])?;
    let script = root.join("run.sh");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o751))?;
    std::fs::hard_link(&script, root.join("old.sh"))?;
    apply_edit(&root, &edit("run.sh", "one", "two", "echo one\n"))?;
    assert_eq!(
        std::fs::metadata(&script)?.permissions().mode() & 0o777,
        0o751
    );
    assert_eq!(text(&root, "run.sh")?, "echo two\n");
    assert_eq!(text(&root, "old.sh")?, "echo one\n");
    assert_eq!(std::fs::read_dir(&*root)?.count(), 2, "no temporary file");
    Ok(())
}

/// Two edits that saw one digest never both land: one that arrives while the
/// other holds the file is refused, and so is one whose file was replaced
/// between its open and its hold.
#[test]
fn sim_two_edits_that_saw_one_digest_never_both_land() -> TestResult {
    let root = snapshot("sim-edit-held", &[("one.txt", b"alpha\n")])?;
    let second = edit("one.txt", "alpha", "second", "alpha\n");
    let mut during = None;
    apply_edit_with(
        &root,
        &edit("one.txt", "alpha", "first", "alpha\n"),
        &mut |window| {
            if window == Window::Decided {
                during = Some(apply_edit(&root, &second));
            }
            Ok(())
        },
    )?;
    assert!(
        matches!(during, Some(Err(EditError::Busy { .. }))),
        "{during:?}"
    );
    assert_eq!(text(&root, "one.txt")?, "first\n");
    let early = edit("one.txt", "first", "early", "first\n");
    let late = apply_edit_with(
        &root,
        &edit("one.txt", "first", "late", "first\n"),
        &mut |window| {
            if window == Window::Opened {
                apply_edit(&root, &early).map_err(std::io::Error::other)?;
            }
            Ok(())
        },
    );
    assert!(matches!(late, Err(EditError::Busy { .. })), "{late:?}");
    assert_eq!(text(&root, "one.txt")?, "early\n");
    Ok(())
}

/// A hundred edits at once on one digest apply exactly one; every other is
/// refused as busy or stale, and the file holds the winner's text.
#[test]
fn sim_a_hundred_edits_at_once_on_one_digest_apply_one() -> TestResult {
    let root = scratch("sim-edit-race")?;
    for seed in 0..8 {
        let path = format!("race-{seed}.txt");
        std::fs::write(root.join(&path), "alpha\n")?;
        let outcomes = std::thread::scope(|owner| {
            // Every editor is started before the first is joined, so they overlap.
            let mut editors = Vec::new();
            for editor in 0..100 {
                let change = edit(&path, "alpha", &format!("editor-{editor}"), "alpha\n");
                let root = &root;
                editors.push(owner.spawn(move || apply_edit(root, &change)));
            }
            editors
                .into_iter()
                .map(std::thread::ScopedJoinHandle::join)
                .collect::<Vec<_>>()
        });
        let mut winners = Vec::new();
        for outcome in outcomes {
            match outcome.map_err(|panic| format!("seed {seed}: {panic:?}"))? {
                Ok(receipt) => winners.push(receipt),
                Err(EditError::Busy { .. } | EditError::StaleDigest { .. }) => {}
                Err(other) => return Err(format!("seed {seed}: {other}").into()),
            }
        }
        assert_eq!(winners.len(), 1, "seed {seed}: one edit applied");
        let kept = digest_of(&text(&root, &path)?);
        assert_eq!(
            winners.first().map(|receipt| receipt.after_digest.clone()),
            Some(kept),
            "seed {seed}: the file holds the winner's text"
        );
    }
    Ok(())
}

/// A directory swapped for a link out of the tree before the replace receives
/// nothing: the edit fails and the file out there is as it was.
#[cfg(unix)]
#[test]
fn sim_a_directory_swapped_for_a_link_before_the_replace_writes_nothing() -> TestResult {
    let outside = snapshot("sim-edit-swap-outside", &[("f.txt", b"outside\n")])?;
    let root = scratch("sim-edit-swap")?;
    std::fs::create_dir(root.join("dir"))?;
    std::fs::write(root.join("dir/f.txt"), "alpha\n")?;
    let swapped = apply_edit_with(
        &root,
        &edit("dir/f.txt", "alpha", "beta", "alpha\n"),
        &mut |window| {
            if window == Window::Decided {
                std::fs::rename(root.join("dir"), root.join("moved"))?;
                std::os::unix::fs::symlink(&*outside, root.join("dir"))?;
            }
            Ok(())
        },
    );
    assert!(matches!(swapped, Err(EditError::Io { .. })), "{swapped:?}");
    assert_eq!(text(&outside, "f.txt")?, "outside\n");
    assert_eq!(
        std::fs::read_dir(&*outside)?.count(),
        1,
        "nothing added outside"
    );
    assert_eq!(text(&root, "moved/f.txt")?, "alpha\n");
    Ok(())
}
