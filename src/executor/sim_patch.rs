//! Seeded simulations of a session's patch (larry#624): a patch applied then
//! reverted gives the original bytes, a corrupted patch is refused with a typed
//! error and nothing written, every file needs the digest the model read, and a
//! write that fails puts every file back. Source: openai/codex
//! `apply_patch.lark` (the shape a trained model writes), CWE-20 (improper input
//! validation) and CWE-367 (a check another writer can pass between).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::mem::discriminant;
use std::path::Path;

use lgwks_std::seeded::Seeded;

use crate::executor::edit::{Edit, EditError, apply_edit, read_file};
use crate::executor::patch::{FileReceipt, Patch, apply_patch, apply_patch_with};
use crate::executor::sim_tree::{Scratch, files_beneath, scratch, tree_digest};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each generated simulation runs.
const SEEDS: u64 = 256;

/// Every file of a tree, with its text.
type Files = BTreeMap<String, String>;

/// The BLAKE3 hex of `text`.
fn digest(text: &str) -> String {
    lgwks_std::hash::blake3(text.as_bytes()).to_hex()
}

/// One patch around `sections`.
fn wrapped(sections: &str) -> String {
    format!("*** Begin Patch\n{sections}*** End Patch\n")
}

/// The patch of `sections` naming the digest of each of `seen`.
fn patch(sections: &str, seen: &[(&str, &str)]) -> Patch {
    Patch {
        text: wrapped(sections),
        expect_digests: seen
            .iter()
            .map(|&(path, text)| (path.to_owned(), digest(text)))
            .collect(),
    }
}

/// `files` as a tree.
fn files_of(files: &[(&str, &str)]) -> Files {
    files
        .iter()
        .map(|&(path, text)| (path.to_owned(), text.to_owned()))
        .collect()
}

/// A scratch tree holding `files`, with their directories.
fn tree(label: &str, files: &Files) -> Result<Scratch, SimError> {
    let root = scratch(label)?;
    for (path, text) in files {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, text)?;
    }
    Ok(root)
}

/// Every file under `root` as it is now.
fn held(root: &Path) -> Result<Files, SimError> {
    let mut files = Files::new();
    for path in files_beneath(root)? {
        let text = String::from_utf8(std::fs::read(root.join(&path))?)?;
        files.insert(path, text);
    }
    Ok(files)
}

/// One generated change in both directions, and the tree on either side of it.
#[derive(Default)]
struct Case {
    /// The files before the patch.
    before: Files,
    /// The files after it.
    after: Files,
    /// The sections that take `before` to `after`.
    forward: Vec<String>,
    /// The sections that take `after` back to `before`.
    back: Vec<String>,
}

/// The patch of `sections` over `files`, naming the digest of each file it
/// changes or removes.
fn over(sections: &str, files: &Files) -> Patch {
    let named = |path: &String| {
        sections.contains(&format!("*** Update File: {path}\n"))
            || sections.contains(&format!("*** Delete File: {path}\n"))
    };
    Patch {
        text: wrapped(sections),
        expect_digests: files
            .iter()
            .filter(|&(path, _)| named(path))
            .map(|(path, text)| (path.clone(), digest(text)))
            .collect(),
    }
}

/// `lines` joined the way a file of this kind ends them.
fn file_of(lines: &[String], newline: &str, terminated: bool) -> String {
    let mut text = lines.join(newline);
    if terminated {
        text.push_str(newline);
    }
    text
}

/// `lines`, each marked with `mark`, as patch lines.
fn marked(mark: char, lines: &[String]) -> String {
    lines.iter().map(|line| format!("{mark}{line}\n")).collect()
}

/// A hunk replacing a seeded run of lines of one seeded file: LF or CRLF, with
/// or without its last newline, in the middle or pinned to the end. Returns the
/// two files and the forward and inverse hunks.
fn rewrite(rng: &mut Seeded, tag: &str) -> Result<[String; 4], SimError> {
    let newline = if rng.below(2)? == 0 { "\n" } else { "\r\n" };
    let terminated = rng.below(2)? == 0;
    let count = rng.index(8)?.saturating_add(3);
    let lines: Vec<String> = (0..count).map(|line| format!("  {tag} {line}")).collect();
    let first = rng.index(count.saturating_sub(1))?.saturating_add(1);
    let past = first.saturating_add(rng.index(count.saturating_sub(first).saturating_add(1))?);
    let fresh: Vec<String> = (0..rng.index(3)?.saturating_add(1))
        .map(|line| format!("\t{tag} new {line}"))
        .collect();
    let part = |from: usize, to: usize| {
        lines
            .iter()
            .take(to)
            .skip(from)
            .cloned()
            .collect::<Vec<_>>()
    };
    let above = part(first.saturating_sub(1), first);
    let old = part(first, past);
    let below = part(past, past.saturating_add(1));
    let changed = [part(0, first), fresh.clone(), part(past, count)].concat();
    let end = if below.is_empty() {
        "*** End of File\n"
    } else {
        ""
    };
    let hunk = |from: &[String], to: &[String]| {
        format!(
            "@@\n{}{}{}{}{end}",
            marked(' ', &above),
            marked('-', from),
            marked('+', to),
            marked(' ', &below)
        )
    };
    Ok([
        file_of(&lines, newline, terminated),
        file_of(&changed, newline, terminated),
        hunk(&old, &fresh),
        hunk(&fresh, &old),
    ])
}

/// Up to four seeded lines, none repeated.
fn few(rng: &mut Seeded, tag: &str) -> Result<Vec<String>, SimError> {
    Ok((0..rng.index(4)?.saturating_add(1))
        .map(|line| format!("{tag} {line}"))
        .collect())
}

/// A seeded patch over any of: an update, an addition in new directories, a
/// deletion and a move that carries a hunk.
fn case(seed: u64) -> Result<Case, SimError> {
    let mut rng = Seeded::from_seed(seed);
    let mut made = Case::default();
    let kinds = rng.below(15)?.saturating_add(1);
    if kinds & 1 != 0 {
        let [before, after, forward, back] = rewrite(&mut rng, "up")?;
        let head = "*** Update File: up.txt\n";
        made.forward.push(format!("{head}{forward}"));
        made.back.push(format!("{head}{back}"));
        made.before.insert(String::from("up.txt"), before);
        made.after.insert(String::from("up.txt"), after);
    }
    if kinds & 2 != 0 {
        let path = format!("made/deep/{seed}.txt");
        let lines = few(&mut rng, "added")?;
        made.forward
            .push(format!("*** Add File: {path}\n{}", marked('+', &lines)));
        made.back.push(format!("*** Delete File: {path}\n"));
        made.after.insert(path, file_of(&lines, "\n", true));
    }
    if kinds & 4 != 0 {
        let lines = few(&mut rng, "gone")?;
        made.forward
            .push(String::from("*** Delete File: gone.txt\n"));
        made.back
            .push(format!("*** Add File: gone.txt\n{}", marked('+', &lines)));
        made.before
            .insert(String::from("gone.txt"), file_of(&lines, "\n", true));
    }
    if kinds & 8 != 0 {
        let [before, after, forward, back] = rewrite(&mut rng, "mv")?;
        let to = format!("moved/{seed}.txt");
        made.forward.push(format!(
            "*** Update File: from.txt\n*** Move to: {to}\n{forward}"
        ));
        made.back.push(format!(
            "*** Update File: {to}\n*** Move to: from.txt\n{back}"
        ));
        made.before.insert(String::from("from.txt"), before);
        made.after.insert(to, after);
    }
    Ok(made)
}

/// Whether each receipt names its own file once, with the digest on either side.
fn receipts_name(receipts: &[FileReceipt], before: &Files, after: &Files) -> bool {
    let named: BTreeSet<&String> = receipts.iter().map(|receipt| &receipt.path).collect();
    let touched: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    named.len() == receipts.len()
        && named == touched
        && receipts.iter().all(|receipt| {
            receipt.before_digest == before.get(&receipt.path).map(|text| digest(text))
                && receipt.after_digest == after.get(&receipt.path).map(|text| digest(text))
        })
}

/// For every seed, a patch lands with a receipt per file and its inverse gives
/// back the original bytes of every file.
#[test]
fn sim_a_patch_applied_then_reverted_gives_the_original_bytes() -> TestResult {
    for seed in 0..SEEDS {
        let made = case(seed)?;
        let root = tree("sim-patch-revert", &made.before)?;
        for (sections, from, to) in [
            (&made.forward, &made.before, &made.after),
            (&made.back, &made.after, &made.before),
        ] {
            let sent = over(&sections.concat(), from);
            let receipts = apply_patch(&root, &sent)
                .map_err(|cause| format!("seed {seed}: {:?} refused: {cause}", sent.text))?;
            assert_eq!(&held(&root)?, to, "seed {seed}: {:?}", sent.text);
            assert!(
                receipts_name(&receipts, from, to),
                "seed {seed}: {receipts:?}"
            );
        }
    }
    Ok(())
}

/// Lines a corruption drops into a patch.
const NOISE: [&str; 8] = [
    "*** Add File: up.txt",
    "*** Update File: ../out",
    "*** Move to: gone.txt",
    "*** End of File",
    "*** Begin Patch",
    "@@ nowhere at all",
    "unmarked line",
    "",
];

/// `text` with one seeded corruption: cut short, a line dropped, doubled,
/// inserted or losing its first character.
fn corrupted(rng: &mut Seeded, text: &str) -> Result<String, SimError> {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let at = rng.index(lines.len())?;
    let line = lines.get(at).ok_or(SimError::Missing("line"))?.clone();
    match rng.below(5)? {
        0 => return Ok(text.chars().take(rng.index(text.len())?).collect()),
        1 => {
            lines.drain(at..=at);
        }
        2 => lines.insert(at, line),
        3 => {
            let noise = NOISE
                .get(rng.index(NOISE.len())?)
                .ok_or(SimError::Missing("noise"))?;
            lines.insert(at, (*noise).to_owned());
        }
        _ => {
            lines.splice(at..=at, [line.chars().skip(1).collect()]);
        }
    }
    Ok(lines.join("\n"))
}

/// A seeded corpus of corrupted patches: each is refused with a typed error
/// that leaves every file and directory as it was, or is still a patch.
#[test]
fn sim_a_corrupted_patch_is_refused_typed_and_writes_nothing() -> TestResult {
    let mut refusals = Vec::new();
    for seed in 0..SEEDS.saturating_mul(4) {
        let made = case(seed % SEEDS)?;
        let root = tree("sim-patch-corrupt", &made.before)?;
        let mut sent = over(&made.forward.concat(), &made.before);
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 32));
        sent.text = corrupted(&mut rng, &sent.text)?;
        let before = tree_digest(&root, false)?;
        if let Err(refused) = apply_patch(&root, &sent) {
            assert_eq!(
                tree_digest(&root, false)?,
                before,
                "seed {seed}: {:?} refused ({refused}) and wrote",
                sent.text
            );
            refusals.push(refused);
        }
    }
    let kinds: HashSet<_> = refusals.iter().map(discriminant).collect();
    assert!(refusals.len() > 256, "only {} refusals", refusals.len());
    assert!(kinds.len() >= 4, "only {} kinds of refusal", kinds.len());
    Ok(())
}

/// Each way a patch can break its shape is refused naming its line.
#[test]
fn sim_each_shape_fault_names_its_patch_line() -> TestResult {
    let files = files_of(&[("kept.txt", "kept\n")]);
    let root = tree("sim-patch-shape", &files)?;
    let update = "*** Update File: kept.txt\n";
    let delete = "*** Delete File: kept.txt\n";
    for (text, wanted) in [
        (String::from("no opening\n"), 1),
        (format!("*** Begin Patch\n{delete}"), 2),
        (wrapped(""), 2),
        (wrapped("*** Frobnicate File: kept.txt\n"), 2),
        (wrapped("\n"), 2),
        (wrapped(update), 2),
        (wrapped(&format!("{update}*** End of File\n")), 3),
        (wrapped(&format!("{update}@@ kept\n")), 3),
        (wrapped(&format!("{update}@@\nunmarked\n")), 4),
        (wrapped("*** Add File: other.txt\n"), 2),
        (wrapped(&format!("{delete}{delete}")), 3),
        (wrapped(&format!("{update}*** Move to: kept.txt\n")), 2),
        (format!("{}more\n", wrapped(delete)), 4),
    ] {
        let sent = Patch {
            text: text.clone(),
            expect_digests: BTreeMap::from([(String::from("kept.txt"), digest("kept\n"))]),
        };
        match apply_patch(&root, &sent) {
            Err(EditError::PatchSyntax { line, .. }) => assert_eq!(line, wanted, "{text:?}"),
            other => return Err(format!("{text:?}: wrong outcome: {other:?}").into()),
        }
    }
    assert_eq!(held(&root)?, files);
    Ok(())
}

/// A file the patch changes or removes needs the digest it still has, one it
/// creates must not be there, a digest of any other file is refused, and a path
/// out of the tree or not a file is refused: nothing is written.
#[cfg(unix)]
#[test]
fn sim_a_patch_needs_the_digest_of_every_file_it_changes() -> TestResult {
    let files = files_of(&[("a.txt", "alpha\n"), ("b.txt", "beta\n")]);
    let root = tree("sim-patch-digest", &files)?;
    let outside = tree("sim-patch-outside", &files)?;
    std::os::unix::fs::symlink(&*outside, root.join("link"))?;
    std::fs::create_dir(root.join("folder"))?;
    let before = tree_digest(&root, false)?;
    let change = "*** Update File: a.txt\n@@\n-alpha\n+ALPHA\n";
    let both = format!("{change}*** Delete File: b.txt\n");
    let moved = "*** Update File: a.txt\n*** Move to: b.txt\n".to_owned();
    let add = |path: &str| format!("*** Add File: {path}\n+x\n");
    let long = format!("*** Update File: a.txt\n@@\n+{}\n", "y".repeat(262_144));
    let seen = [("a.txt", "alpha\n")];
    let stale = format!(
        "edit file b.txt changed since it was read: its digest is {}",
        digest("beta\n")
    );
    for (sections, digests, wanted) in [
        (
            both.clone(),
            &seen[..],
            "patch changes b.txt but expect_digests names no digest for it",
        ),
        (
            both,
            &[("a.txt", "alpha\n"), ("b.txt", "stale\n")][..],
            stale.as_str(),
        ),
        (
            add("new.txt"),
            &seen[..],
            "patch names a digest for a.txt, which it does not change",
        ),
        (
            add("b.txt"),
            &[],
            "patch creates b.txt, which already exists",
        ),
        (
            moved,
            &seen[..],
            "patch creates b.txt, which already exists",
        ),
        (
            add("../escape.txt"),
            &[],
            "edit path ../escape.txt escapes the snapshot",
        ),
        (
            add("link/new.txt"),
            &[],
            "edit path link/new.txt escapes the snapshot",
        ),
        (add("folder"), &[], "edit path folder is not a regular file"),
        (
            format!("{change}*** Delete File: absent.txt\n"),
            &[("a.txt", "alpha\n"), ("absent.txt", "")][..],
            "edit file absent.txt could not be read or written",
        ),
        (
            add("a.txt/under"),
            &[],
            "edit file a.txt/under could not be",
        ),
        (long, &seen[..], "edit file a.txt is 262151 bytes"),
    ] {
        match apply_patch(&root, &patch(&sections, digests)) {
            Err(refused) => assert!(
                refused.to_string().starts_with(wanted),
                "{refused} is not {wanted:?}"
            ),
            Ok(receipts) => return Err(format!("{wanted}: landed {receipts:?}").into()),
        }
        assert_eq!(tree_digest(&root, false)?, before, "{wanted}");
    }
    assert_eq!(held(&outside)?, files, "nothing was written outside");
    Ok(())
}

/// The five writes the midway simulations make, in write order.
const WRITTEN: [&str; 5] = ["a.txt", "new/deep/c.txt", "b.txt", "other/e.txt", "d.txt"];

/// The tree those writes change, and the patch of them: an update, an addition
/// in new directories, a deletion and a move.
fn five(label: &str) -> Result<(Scratch, Patch), SimError> {
    let seen = [
        ("a.txt", "alpha\n"),
        ("b.txt", "beta\n"),
        ("d.txt", "delta\n"),
    ];
    let sections = "*** Update File: a.txt\n@@\n-alpha\n+ALPHA\n*** Add File: new/deep/c.txt\n+gamma\n*** Delete File: b.txt\n*** Update File: d.txt\n*** Move to: other/e.txt\n@@\n-delta\n+DELTA\n";
    Ok((tree(label, &files_of(&seen))?, patch(sections, &seen)))
}

/// A write that fails at any of the changes puts back every file already
/// written and removes every directory the patch made.
#[test]
fn sim_a_patch_failing_at_any_write_puts_every_file_back() -> TestResult {
    for (failing, path) in WRITTEN.iter().enumerate() {
        let (root, sent) = five("sim-patch-midway")?;
        let before = tree_digest(&root, false)?;
        let outcome = apply_patch_with(&root, &sent, &mut |at| {
            if at == failing {
                return Err(io::Error::other("injected"));
            }
            Ok(())
        });
        match outcome {
            Err(EditError::Io { path: named, .. }) => assert_eq!(&named, path),
            other => return Err(format!("write {failing}: wrong outcome: {other:?}").into()),
        }
        assert_eq!(tree_digest(&root, false)?, before, "write {failing}");
    }
    let (root, sent) = five("sim-patch-whole")?;
    let receipts = apply_patch(&root, &sent)?;
    let named: Vec<&str> = receipts.iter().map(|receipt| &*receipt.path).collect();
    assert_eq!(named, WRITTEN);
    Ok(())
}

/// A file that appears at a path the patch creates is not overwritten and not
/// removed by the rollback: the patch fails and puts its own writes back.
#[test]
fn sim_a_file_that_appears_before_its_creation_is_left_alone() -> TestResult {
    let (root, sent) = five("sim-patch-appears")?;
    let target = root.join("new/deep/c.txt");
    let outcome = apply_patch_with(&root, &sent, &mut |at| {
        if at == 1 {
            std::fs::create_dir_all(root.join("new/deep"))?;
            std::fs::write(&target, "theirs\n")?;
        }
        Ok(())
    });
    assert!(matches!(outcome, Err(EditError::Io { .. })), "{outcome:?}");
    assert_eq!(std::fs::read_to_string(&target)?, "theirs\n");
    assert_eq!(std::fs::read_to_string(root.join("a.txt"))?, "alpha\n");
    Ok(())
}

/// Every file of a patch is held from its check to the patch's end: an edit of
/// one already written, or of one still to be written, is refused as busy.
#[test]
fn sim_a_file_of_a_patch_stays_held_until_the_patch_ends() -> TestResult {
    let (root, sent) = five("sim-patch-held")?;
    let mut during = Vec::new();
    apply_patch_with(&root, &sent, &mut |at| {
        if at == 2 {
            for (path, old) in [
                ("a.txt", "ALPHA\n"),
                ("new/deep/c.txt", "gamma\n"),
                ("d.txt", "delta\n"),
            ] {
                let change = Edit {
                    path: path.to_owned(),
                    old: old.to_owned(),
                    new: String::from("theirs\n"),
                    expect_digest: digest(old),
                };
                during.push((path, apply_edit(&root, &change)));
            }
        }
        Ok(())
    })?;
    assert_eq!(during.len(), 3);
    for (path, outcome) in during {
        assert!(
            matches!(outcome, Err(EditError::Busy { .. })),
            "{path}: {outcome:?}"
        );
    }
    assert_eq!(read_file(&root, "a.txt")?.text, "ALPHA\n");
    Ok(())
}

/// A rollback that cannot restore a file says which, with the first failure.
#[test]
fn sim_a_rollback_that_cannot_restore_names_what_stayed() -> TestResult {
    let (root, sent) = five("sim-patch-partly")?;
    let first = root.join("a.txt");
    let outcome = apply_patch_with(&root, &sent, &mut |at| {
        if at == 1 {
            std::fs::remove_file(&first)?;
            std::fs::create_dir(&first)?;
            return Err(io::Error::other("injected"));
        }
        Ok(())
    });
    match outcome {
        Err(EditError::PartlyApplied { left, cause }) => {
            assert_eq!(left, vec![String::from("a.txt")]);
            assert!(matches!(*cause, EditError::Io { .. }), "{cause}");
        }
        other => return Err(format!("wrong outcome: {other:?}").into()),
    }
    Ok(())
}

/// A hunk sits where its lines are: a repeated one is refused until an `@@`
/// anchor picks the one after it, additions alone land at the end, whitespace
/// differences keep the file's own context bytes, a trailing blank line is not
/// part of the file, and `*** End of File` pins a hunk to the end.
#[test]
fn sim_a_hunk_sits_in_the_one_place_its_lines_name() -> TestResult {
    let before = "fn a\n x\nfn b\n x\n";
    let files = files_of(&[("p.txt", before)]);
    let sent = |hunks: &str| {
        patch(
            &format!("*** Update File: p.txt\n{hunks}"),
            &[("p.txt", before)],
        )
    };
    for (hunks, after) in [
        ("@@ fn b\n- x\n+ y\n", "fn a\n x\nfn b\n y\n"),
        ("@@ fn a\n- x\n+ y\n", "fn a\n y\nfn b\n x\n"),
        (
            "@@ fn a\n@@ fn b\n- x\n+ y\n@@\n+tail\n",
            "fn a\n x\nfn b\n y\ntail\n",
        ),
        ("@@\n fn b  \n-x\n+y\n", "fn a\n x\nfn b\ny\n"),
        ("@@\n- x\n+ z\n*** End of File\n", "fn a\n x\nfn b\n z\n"),
        ("@@\n-fn a\n+fn c\n\n", "fn c\n x\nfn b\n x\n"),
        ("@@\n+tail\n\n", "fn a\n x\nfn b\n x\ntail\n"),
        ("@@\n fn a\n x\n+mid\n", "fn a\n x\nmid\nfn b\n x\n"),
    ] {
        let root = tree("sim-patch-place", &files)?;
        apply_patch(&root, &sent(hunks)).map_err(|cause| format!("{hunks:?}: {cause}"))?;
        assert_eq!(read_file(&root, "p.txt")?.text, after, "{hunks:?}");
    }
    let root = tree("sim-patch-misplace", &files)?;
    match apply_patch(&root, &sent("@@\n- x\n+ y\n")) {
        Err(EditError::Ambiguous { candidates, .. }) => assert_eq!(candidates, vec![2, 4]),
        other => return Err(format!("wrong outcome: {other:?}").into()),
    }
    for hunks in [
        "@@ fn c\n- x\n+ y\n",
        "@@\n-absent\n+y\n",
        "@@\n-fn a\n+y\n*** End of File\n",
        "@@ fn b\n-fn a\n+y\n",
        "@@\n- x\n-fn b\n- x\n-more\n+y\n",
    ] {
        match apply_patch(&root, &sent(hunks)) {
            Err(EditError::PatchMismatch { line: 3, .. }) => {}
            other => return Err(format!("{hunks:?}: wrong outcome: {other:?}").into()),
        }
    }
    assert_eq!(held(&root)?, files);
    Ok(())
}

/// A patch takes at most sixteen files, so one report record stays bounded.
#[test]
fn sim_a_patch_of_too_many_files_is_refused() -> TestResult {
    let adds = |count: usize| -> String {
        (0..count)
            .map(|file| format!("*** Add File: many/{file}\n+x\n"))
            .collect()
    };
    let root = scratch("sim-patch-bounds")?;
    assert!(matches!(
        apply_patch(&root, &patch(&adds(17), &[])),
        Err(EditError::PatchSyntax { line: 34, .. })
    ));
    assert!(held(&root)?.is_empty(), "a refused patch writes nothing");
    assert_eq!(apply_patch(&root, &patch(&adds(16), &[]))?.len(), 16);
    Ok(())
}

/// An update and a move keep the file's mode, and no temporary file is left.
#[cfg(unix)]
#[test]
fn sim_a_patch_keeps_the_mode_of_each_file() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let seen = [("run.sh", "echo one\n"), ("old.sh", "echo old\n")];
    let root = tree("sim-patch-mode", &files_of(&seen))?;
    for (path, _) in seen {
        std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(0o751))?;
    }
    let sections = "*** Update File: run.sh\n@@\n-echo one\n+echo two\n*** Update File: old.sh\n*** Move to: bin/new.sh\n";
    apply_patch(&root, &patch(sections, &seen))?;
    for path in ["run.sh", "bin/new.sh"] {
        let mode = std::fs::metadata(root.join(path))?.permissions().mode();
        assert_eq!(mode & 0o777, 0o751, "{path}");
    }
    assert_eq!(files_beneath(&root)?, ["bin/new.sh", "run.sh"]);
    Ok(())
}

/// Thirty-two patches at once on the same digests apply exactly one; every other
/// is refused and the winner's text stays.
#[test]
fn sim_patches_at_once_on_one_digest_apply_one() -> TestResult {
    let seen = [("a.txt", "alpha\n"), ("b.txt", "beta\n")];
    for seed in 0..8 {
        let root = tree("sim-patch-race", &files_of(&seen))?;
        let outcomes = std::thread::scope(|owner| {
            // Every writer is started before the first is joined, so they overlap.
            let mut writers = Vec::new();
            for writer in 0..32 {
                let sections = format!(
                    "*** Update File: a.txt\n@@\n-alpha\n+a-{writer}\n*** Update File: b.txt\n@@\n-beta\n+b-{writer}\n"
                );
                let sent = patch(&sections, &seen);
                let root = &root;
                writers.push(owner.spawn(move || apply_patch(root, &sent).map(|_| writer)));
            }
            writers
                .into_iter()
                .map(std::thread::ScopedJoinHandle::join)
                .collect::<Vec<_>>()
        });
        let mut winners = Vec::new();
        for outcome in outcomes {
            match outcome.map_err(|panic| format!("seed {seed}: {panic:?}"))? {
                Ok(writer) => winners.push(writer),
                Err(EditError::Busy { .. } | EditError::StaleDigest { .. }) => {}
                Err(other) => return Err(format!("seed {seed}: {other}").into()),
            }
        }
        let &[winner] = winners.as_slice() else {
            return Err(format!("seed {seed}: winners {winners:?}").into());
        };
        assert_eq!(read_file(&root, "a.txt")?.text, format!("a-{winner}\n"));
        assert_eq!(read_file(&root, "b.txt")?.text, format!("b-{winner}\n"));
    }
    Ok(())
}
