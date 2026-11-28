//! Adversarial controls of the temp-path rule's manifest read and source walk (larry#311,
//! larry#554): hostile manifests, member forms and trees, each refused by its exact text.
//! Source: the Cargo Book on workspaces, TOML 1.0.0, CWE-59 (link following) and CWE-400
//! (uncontrolled resource consumption). Requirement: a tree the rule cannot read whole is a
//! typed refusal naming the path, never a pass over fewer files.

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::Path;

use lgwks_ast::MAX_SOURCE_BYTES;

use super::super::sim_support::{Scratch, SimError, SimResult};
use super::{
    EntryError, MAX_DEPTH, Workspace, array_items, declared_members, entry_name, expand,
    lane_count, may_follow, may_refuse, rule_errors, section, sort_by_file_and_line, string_items,
};

/// A package tree: a manifest, `src/lib.rs` and `tests/keep.rs`.
fn package(tag: &str) -> Result<Scratch, SimError> {
    let tree = Scratch::new(tag)?;
    tree.write("Cargo.toml", b"[package]\nname = \"f\"\n")?;
    tree.write("src/lib.rs", b"pub fn present() {}\n")?;
    tree.write("tests/keep.rs", b"fn keep() {}\n")?;
    Ok(tree)
}

/// The text of the refusal that stopped the manifest read of `root`.
fn manifest_refusal(root: &Path) -> Result<String, SimError> {
    match Workspace::read(root) {
        Ok(workspace) => Err(SimError::Absent(format!(
            "a manifest refusal, but the members are {:?}",
            workspace.members()
        ))),
        Err(cause) => Ok(cause.to_string()),
    }
}

/// The text of the refusal that stopped the walk of `root` under `workspace`.
fn walk_refusal(root: &Path, workspace: &Workspace) -> Result<String, SimError> {
    match rule_errors(root, workspace) {
        Ok(errors) => Err(SimError::Absent(format!(
            "a walk refusal, but the rule answered {errors:?}"
        ))),
        Err(cause) => Ok(cause.to_string()),
    }
}

/// Sets the mode of `path`.
fn chmod(path: &Path, mode: u32) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// A manifest that is a link, a directory, unreadable, not UTF-8 or whose member array never
/// closes is refused by name; none of them reads as a tree with no member.
#[test]
fn sim_a_hostile_manifest_is_refused_by_its_exact_text() -> SimResult {
    let tree = package("manifest-hostile")?;
    let manifest = tree.path().join("Cargo.toml");
    assert_eq!(Workspace::read(tree.path())?.members(), [""]);

    tree.write(
        "Cargo.toml",
        b"[workspace]\nmembers = [\"b\", \"a\", \"b/\"]\n",
    )?;
    assert_eq!(
        Workspace::read(tree.path())?.members(),
        ["a", "b"],
        "members are sorted and named once"
    );
    for broken in [
        "[workspace]\nmembers = [\"a\", \n",
        "[workspace]\nmembers = \"a\"\n",
        "[workspace]\nmembers = ] [\"a\"\n",
    ] {
        tree.write("Cargo.toml", broken.as_bytes())?;
        assert_eq!(
            manifest_refusal(tree.path())?,
            "workspace refused: the members array does not close",
            "{broken:?}"
        );
    }
    tree.write("Cargo.toml", &[0xff])?;
    let refused = manifest_refusal(tree.path())?;
    assert!(
        refused.starts_with("workspace refused: Cargo.toml is not UTF-8: invalid utf-8"),
        "{refused}"
    );

    fs::remove_file(&manifest)?;
    fs::create_dir(&manifest)?;
    assert_eq!(
        manifest_refusal(tree.path())?,
        "workspace refused: Cargo.toml is not a file"
    );
    fs::remove_dir(&manifest)?;

    tree.write("elsewhere.toml", b"[package]\n")?;
    symlink(tree.path().join("elsewhere.toml"), &manifest)?;
    assert_eq!(
        manifest_refusal(tree.path())?,
        "workspace refused: Cargo.toml is a symbolic link"
    );
    fs::remove_file(&manifest)?;

    tree.write("Cargo.toml", b"[package]\n")?;
    chmod(&manifest, 0o000)?;
    let refused = manifest_refusal(tree.path());
    chmod(&manifest, 0o644)?;
    let refused = refused?;
    assert!(refused.starts_with("cannot read Cargo.toml: "), "{refused}");

    // A root that is a file cannot be looked into, which is not the same as no manifest.
    let refused = manifest_refusal(&tree.path().join("src/lib.rs"))?;
    assert!(refused.starts_with("cannot read Cargo.toml: "), "{refused}");
    Ok(())
}

/// The manifest is lexed as TOML spells it: section headers with comments, lookalike keys,
/// both quote forms, escapes, torn strings and comments (TOML 1.0.0, larry#611).
#[test]
fn sim_the_manifest_lexer_reads_only_what_the_manifest_declares() -> SimResult {
    assert_eq!(
        section("[package] # c\nname = \"f\"\n[other]\nx = 1\n", "package"),
        Some("name = \"f\"\n")
    );
    assert_eq!(
        section("  [workspace]\nmembers = []\n", "workspace"),
        Some("members = []\n")
    );
    assert_eq!(section("[package.metadata]\na = 1\n", "package"), None);
    assert_eq!(section("# [package]\n", "package"), None);

    let none: [&str; 0] = [];
    for lookalike in [
        "membersx = [\"a\"]\n",
        "members [\"a\"]\n",
        "default-members = [\"a\"]\n",
        "members\n",
        "",
    ] {
        assert_eq!(declared_members(lookalike)?, none, "{lookalike:?}");
    }
    assert_eq!(declared_members("members=[\"a\"]")?, ["a"]);

    assert_eq!(array_items("['a', \"b]\"] # ] c\n")?, ["a", "b]"]);
    assert_eq!(array_items("[ # ]\n \"a\"\n]")?, ["a"]);
    assert_eq!(array_items("[\"q\\\"]\"]")?, ["q\"]"]);
    for torn in ["[\"a\\", "[\"unterminated\"", "[", "# [\"a\"]"] {
        let refused = array_items(torn).err().map(|cause| cause.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("workspace refused: the members array does not close"),
            "{torn:?}"
        );
    }

    assert_eq!(string_items("\"a\" 'b'"), ["a", "b"]);
    assert_eq!(string_items("\"a\\n\\tb\\\\\""), ["a\n\tb\\"]);
    assert_eq!(string_items("'a\\n'"), ["a\\n"], "no literal escape");
    assert_eq!(string_items("\"a\\"), none);
    assert_eq!(string_items("\"open"), none);
    assert_eq!(string_items("# \"x\"\n\"a#b\", # 'y'\n'c'"), ["a#b", "c"]);
    assert_eq!(string_items("\"a\" # no newline \"b\""), ["a"]);
    Ok(())
}

/// A member entry is a literal path or a trailing `*` glob over the directories that hold a
/// manifest; every other metacharacter and an unreadable parent are refused.
#[test]
fn sim_a_member_glob_names_only_packages_or_refuses() -> SimResult {
    let tree = Scratch::new("manifest-glob")?;
    tree.write("crates/a/Cargo.toml", b"[package]\n")?;
    tree.write("crates/NOTE", b"not a member\n")?;
    fs::create_dir_all(tree.path().join("crates/bare"))?;
    tree.write("pkg/Cargo.toml", b"[package]\n")?;
    let root = tree.path();
    assert_eq!(expand(root, "crates/a")?, ["crates/a"]);
    assert_eq!(expand(root, "crates/a/")?, ["crates/a"]);
    assert_eq!(expand(root, "crates/*")?, ["crates/a"]);
    assert_eq!(expand(root, "/*")?, ["pkg"]);
    for glob in ["crates/*/deep", "cr*", "*"] {
        let refused = expand(root, glob).err().map(|cause| cause.to_string());
        assert_eq!(
            refused,
            Some(format!(
                "workspace refused: unsupported member glob: {glob}"
            )),
        );
    }
    let missing = expand(root, "missing/*")
        .err()
        .map(|cause| cause.to_string());
    assert!(
        missing
            .as_deref()
            .is_some_and(|text| text.starts_with("cannot read missing: ")),
        "{missing:?}"
    );
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let hostile = std::ffi::OsStr::from_bytes(b"h-\xff");
        let hostile = root.join("crates").join(hostile);
        fs::create_dir(&hostile)?;
        fs::write(hostile.join("Cargo.toml"), b"[package]\n")?;
        assert_eq!(expand(root, "crates/*")?, ["crates/a"]);
    }
    Ok(())
}

/// A member root that is a link, a file or gone, and a tree whose root is gone, stop the rule
/// with the path named before any source is read.
#[test]
fn sim_a_member_root_that_is_not_a_directory_stops_the_walk() -> SimResult {
    let tree = package("walk-roots")?;
    let workspace = Workspace::read(tree.path())?;
    assert_eq!(rule_errors(tree.path(), &workspace)?, Vec::<String>::new());

    let src = tree.path().join("src");
    fs::remove_dir_all(&src)?;
    symlink(tree.path().join("tests"), &src)?;
    assert_eq!(
        walk_refusal(tree.path(), &workspace)?,
        "cannot read src: a symbolic link is never followed"
    );
    fs::remove_file(&src)?;
    fs::write(&src, b"a file")?;
    assert_eq!(
        walk_refusal(tree.path(), &workspace)?,
        "cannot read src: not a directory"
    );
    fs::remove_file(&src)?;
    assert_eq!(
        walk_refusal(tree.path(), &workspace)?,
        "source tree is missing: src"
    );
    // A root that is a file: looking up `src` under it fails with a cause of its own.
    let refused = walk_refusal(&tree.path().join("tests/keep.rs"), &workspace)?;
    assert!(refused.starts_with("cannot read src: "), "{refused}");
    // A root that is gone names no member and fails its canonicalization.
    let gone = tree.path().join("never-there");
    let empty = Workspace::read(&gone)?;
    assert_eq!(empty.members().len(), 0);
    let refused = walk_refusal(&gone, &empty)?;
    assert!(refused.starts_with("cannot read .: "), "{refused}");
    Ok(())
}

/// A source past the reader bound, one that is not UTF-8, a FIFO named like a source and one the
/// reader may not open are each refused by name, never skipped.
#[test]
fn sim_an_unreadable_source_is_named_never_passed() -> SimResult {
    let tree = package("walk-sources")?;
    let workspace = Workspace::read(tree.path())?;
    let over = MAX_SOURCE_BYTES.saturating_add(1);
    tree.write("tests/big.rs", &vec![b' '; over])?;
    assert_eq!(
        rule_errors(tree.path(), &workspace)?,
        [format!(
            "source file exceeds byte limit: tests/big.rs ({over} bytes)"
        )]
    );
    fs::remove_file(tree.path().join("tests/big.rs"))?;

    tree.write("tests/latin.rs", b"fn f() {} // \xe9\n")?;
    assert_eq!(
        rule_errors(tree.path(), &workspace)?,
        ["source file is not UTF-8: tests/latin.rs"]
    );
    fs::remove_file(tree.path().join("tests/latin.rs"))?;

    let status = std::process::Command::new("mkfifo")
        .arg(tree.path().join("tests/pipe.rs"))
        .status()?;
    assert!(status.success(), "mkfifo refused: {status}");
    assert_eq!(
        rule_errors(tree.path(), &workspace)?,
        ["cannot read tests/pipe.rs: not a regular file"]
    );
    fs::remove_file(tree.path().join("tests/pipe.rs"))?;

    let secret = tree.path().join("src/secret.rs");
    tree.write("src/secret.rs", b"fn hidden() {}\n")?;
    chmod(&secret, 0o000)?;
    let refused = rule_errors(tree.path(), &workspace);
    chmod(&secret, 0o644)?;
    let refused = refused?;
    assert!(
        matches!(refused.as_slice(), [one] if one.starts_with("cannot read src/secret.rs: ")),
        "{refused:?}"
    );
    Ok(())
}

/// A directory the walk may not list, and nesting one level past the bound, stop the rule; the
/// deepest admitted level is still read.
#[test]
fn sim_a_locked_or_bottomless_tree_stops_the_walk() -> SimResult {
    let tree = package("walk-deep")?;
    let workspace = Workspace::read(tree.path())?;
    let locked = tree.path().join("src/locked");
    fs::create_dir(&locked)?;
    chmod(&locked, 0o000)?;
    let refused = walk_refusal(tree.path(), &workspace);
    chmod(&locked, 0o755)?;
    let refused = refused?;
    assert!(refused.starts_with("cannot read src/locked: "), "{refused}");

    let mut deepest = String::from("src");
    for _ in 0..MAX_DEPTH {
        deepest.push_str("/d");
    }
    fs::create_dir_all(tree.path().join(&deepest))?;
    assert_eq!(rule_errors(tree.path(), &workspace)?, Vec::<String>::new());
    deepest.push_str("/d");
    fs::create_dir(tree.path().join(&deepest))?;
    assert_eq!(
        walk_refusal(tree.path(), &workspace)?,
        format!("cannot read {deepest}: source tree deeper than {MAX_DEPTH}")
    );
    Ok(())
}

/// The lane count, the two text gates, the entry name and the report order are exact.
#[test]
fn sim_the_walk_helpers_answer_exactly() {
    assert_eq!(lane_count(0), 1);
    assert_eq!(lane_count(1), 1);
    assert!((1..=64).contains(&lane_count(64)));
    assert!(may_refuse("process tmp"));
    assert!(may_refuse("process temp_dir"));
    assert!(may_refuse("process TMPDIR"));
    assert!(!may_refuse("process"));
    assert!(!may_refuse("tmp"));
    assert!(may_follow("#[path = \"a.rs\"]"));
    assert!(may_follow("include!(\"a.rs\")"));
    assert!(!may_follow("path"));
    assert_eq!(entry_name(Path::new("a/ok.rs")), Ok("ok.rs"));
    assert_eq!(entry_name(Path::new("/")), Err(EntryError::NotUtf8));

    let mut errors: Vec<String> = ["nocolon", "b.rs:x", "a.rs:10", "a.rs:2"]
        .iter()
        .map(|error| String::from(*error))
        .collect();
    sort_by_file_and_line(&mut errors);
    assert_eq!(errors, ["a.rs:2", "a.rs:10", "b.rs:x", "nocolon"]);
}
