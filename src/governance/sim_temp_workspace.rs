//! Seeded controls of the temp-path rule's workspace read (larry#311): the member list comes from
//! the root `Cargo.toml`, a missing member root is a typed refusal naming it, and `#[path]` and
//! `include!` modules are followed and judged as the test code that names them.
//! Source: the Cargo Book on workspaces and the Rust Reference on modules and `include!`, and
//! CWE-59 for the symlink the follow never takes.

use lgwks_std::seeded::Seeded;

use super::error::GovernanceError;
use super::sim_support::{Scratch, SimError, SimResult};
use super::sim_temp_paths::{
    CLEAN, LEAK, MANIFEST, assert_all, assert_none, assert_one, assert_reason, named, package,
    refusal, refusals, stopped,
};
use super::temp_paths;

/// A scratch tree that is a workspace of `members`, each a package with `src/lib.rs` and
/// `tests/keep.rs`.
fn workspace(tag: &str, members: &[&str]) -> Result<Scratch, SimError> {
    let tree = Scratch::new(tag)?;
    let quoted: Vec<String> = members.iter().map(|name| format!("\"{name}\"")).collect();
    named(
        &tree,
        "Cargo.toml",
        &format!(
            "[workspace]\nmembers = [{}]\nresolver = \"3\"\n",
            quoted.join(", ")
        ),
    )?;
    for member in members {
        add_package(&tree, member)?;
    }
    Ok(tree)
}

/// Writes `member` as a package: its manifest, `src/lib.rs` and `tests/keep.rs`.
fn add_package(tree: &Scratch, member: &str) -> Result<(), SimError> {
    let name = member.replace('/', "-");
    named(
        tree,
        &format!("{member}/Cargo.toml"),
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )?;
    named(
        tree,
        &format!("{member}/src/lib.rs"),
        "pub fn present() {}\n",
    )?;
    named(tree, &format!("{member}/tests/keep.rs"), "fn keep() {}\n")
}

/// larry#311: a comment before the member array ends at its newline, and a quoted escape is read
/// as TOML spells it, so neither hides the members the manifest declares.
#[test]
fn sim_a_comment_and_an_escaped_string_keep_the_members_readable() -> SimResult {
    let tree = Scratch::new("temp-members-comment")?;
    named(
        &tree,
        "Cargo.toml",
        r#"[workspace]
members = [ # the crates
  "crates/one"
]
"#,
    )?;
    add_package(&tree, "crates/one")?;
    named(&tree, "crates/one/tests/leak.rs", LEAK)?;
    assert_one(
        &tree,
        "crates/one/tests/leak.rs",
        2,
        "a comment before the array does not hide a member",
    )?;
    let escaped = Scratch::new("temp-members-escape")?;
    named(
        &escaped,
        "Cargo.toml",
        r#"[workspace]
members = ["\\"]
"#,
    )?;
    match temp_paths::errors(escaped.path()) {
        Err(GovernanceError::MissingRoot { ref relative }) => {
            assert_eq!(
                relative, "\\/src",
                "the escaped member is read as TOML spells it"
            );
        }
        other => {
            return Err(SimError::Absent(format!("MissingRoot, got {other:?}")).into());
        }
    }
    Ok(())
}

/// larry#311: the rule reads the member list from the root manifest, so a pid temp path under a
/// member crate is refused, in the crate's `tests` or its in-crate `sim_` module, and a directory
/// the manifest does not name is never read.
#[test]
fn sim_a_workspace_member_and_its_crate_are_both_scanned() -> SimResult {
    let cases = ["crates/alpha/tests/leak.rs", "crates/beta/src/sim_leak.rs"];
    for seed in [0x311_u64, 0x312] {
        let tree = workspace("temp-members", &["crates/alpha", "crates/beta"])?;
        let mut rng = Seeded::from_seed(seed);
        let chosen = rng.index(cases.len())?;
        let chosen_case = *cases.get(chosen).ok_or("a chosen member case")?;
        for (slot, name) in cases.iter().enumerate() {
            let source = if slot == chosen { LEAK } else { CLEAN };
            named(&tree, name, source)?;
        }
        named(&tree, "crates/gamma/tests/leak.rs", LEAK)?;
        assert_all(
            &tree,
            &[refusal(chosen_case, 2)],
            "only the member the manifest names is refused",
        )?;
    }
    Ok(())
}

/// larry#311: a member with no `src` or `tests` directory is a typed refusal naming the missing
/// root, never a pass over zero files.
#[test]
fn sim_a_missing_member_root_is_a_typed_refusal() -> SimResult {
    let tree = Scratch::new("temp-missing-member")?;
    named(
        &tree,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/alpha\", \"crates/beta\"]\n",
    )?;
    add_package(&tree, "crates/alpha")?;
    named(&tree, "crates/beta/Cargo.toml", MANIFEST)?;
    named(&tree, "crates/beta/src/lib.rs", "pub fn present() {}\n")?;
    named(&tree, "crates/beta/src/keep.rs", "fn keep() {}\n")?;
    match temp_paths::errors(tree.path()) {
        Err(GovernanceError::MissingRoot { ref relative }) => {
            assert_eq!(relative, "crates/beta/tests", "the missing root is named");
        }
        other => return Err(SimError::Absent(format!("MissingRoot, got {other:?}")).into()),
    }
    let root_package = Scratch::new("temp-missing-root")?;
    named(&root_package, "Cargo.toml", MANIFEST)?;
    match temp_paths::errors(root_package.path()) {
        Err(GovernanceError::MissingRoot { ref relative }) => {
            assert_eq!(relative, "src", "the root package is a member too");
        }
        other => return Err(SimError::Absent(format!("MissingRoot, got {other:?}")).into()),
    }
    Ok(())
}

/// larry#311: a `#[path]` module and an `include!` file outside the walk are followed, inherit the
/// test code that names them, and are refused at their own file and line.
#[test]
fn sim_followed_path_and_include_modules_are_scanned() -> SimResult {
    let tree = package("temp-follow")?;
    named(
        &tree,
        "tests/entry.rs",
        "#[path = \"../modules/entry_mod.rs\"]\nmod entry_mod;\n",
    )?;
    named(
        &tree,
        "modules/entry_mod.rs",
        r#"#[path = "alt.rs"]
mod alt;

include!("inc.rs");
"#,
    )?;
    named(&tree, "modules/alt.rs", LEAK)?;
    named(&tree, "modules/inc.rs", LEAK)?;
    assert_all(
        &tree,
        &[refusal("modules/alt.rs", 2), refusal("modules/inc.rs", 2)],
        "a followed module is judged as the test code that names it",
    )
}

/// larry#311: a `#[path]` attribute inside an inline module is rooted through that module's name,
/// as rustc roots it, so the live tree's `journal/durable.rs` guard is found and read, not refused.
#[test]
fn sim_an_inline_module_roots_a_followed_path_below_it() -> SimResult {
    let tree = package("temp-inline-follow")?;
    named(
        &tree,
        "tests/entry.rs",
        r#"mod inner {
    #[path = "../../modules/inline.rs"]
    mod inline;
}
"#,
    )?;
    named(&tree, "modules/inline.rs", LEAK)?;
    assert_one(
        &tree,
        "modules/inline.rs",
        2,
        "the inline module's followed file is judged as the test code above it",
    )
}

/// larry#311: a followed module that is absolute, leaves the checked tree, is a symbolic link or is
/// missing is named in a refusal and never read.
#[test]
fn sim_a_followed_module_outside_the_tree_is_refused() -> SimResult {
    let tree = package("temp-escape")?;
    named(
        &tree,
        "tests/escape.rs",
        r#"#[path = "../../outside.rs"]
mod outside;

#[path = "/tmp/absolute.rs"]
mod absolute;

#[path = "gone.rs"]
mod gone;

#[path = "link.rs"]
mod link;
"#,
    )?;
    std::os::unix::fs::symlink(
        tree.path().join("src/lib.rs"),
        tree.path().join("tests/link.rs"),
    )?;
    let errors = refusals(&tree)?;
    assert_reason(&errors, "tests/escape.rs", "leaves the checked tree");
    assert_reason(&errors, "tests/escape.rs", "names an absolute path");
    assert_reason(&errors, "tests/escape.rs", "names no readable source");
    assert_reason(
        &errors,
        "tests/escape.rs",
        "symbolic link is never followed",
    );
    assert!(
        errors
            .iter()
            .all(|error| !error.contains("names a temp directory")),
        "no escaped module is read: {errors:?}"
    );
    Ok(())
}

/// larry#311: a manifest with `[package]` alone scans the root package's `src` and `tests`, and a
/// document-only tree with no manifest names no member to read.
#[test]
fn sim_the_manifest_decides_the_roots() -> SimResult {
    assert!(
        MANIFEST.contains("[package]"),
        "the fixture manifest declares a package"
    );
    let tree = package("temp-package-roots")?;
    named(&tree, "src/sim_leak.rs", LEAK)?;
    assert_one(
        &tree,
        "src/sim_leak.rs",
        2,
        "the root package is a workspace member",
    )?;
    let documents = Scratch::new("temp-no-manifest")?;
    named(&documents, "tests/leak.rs", LEAK)?;
    assert_none(&documents, "a tree with no manifest names no member")
}

/// larry#311: `members = ["crates/*"]` is expanded to the directories that hold a manifest, so a
/// plain directory under `crates/` is not a member.
#[test]
fn sim_a_glob_member_expands_to_the_packages_it_names() -> SimResult {
    let tree = Scratch::new("temp-glob-members")?;
    named(
        &tree,
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
resolver = "3"
"#,
    )?;
    add_package(&tree, "crates/one")?;
    add_package(&tree, "crates/two")?;
    named(&tree, "crates/one/tests/leak.rs", LEAK)?;
    let plain = Scratch::new("temp-glob-plain")?;
    named(
        &plain,
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
"#,
    )?;
    add_package(&plain, "crates/one")?;
    named(&plain, "crates/plain/tests/leak.rs", LEAK)?;
    assert_one(
        &tree,
        "crates/one/tests/leak.rs",
        2,
        "a glob member's package is scanned",
    )?;
    assert_none(&plain, "a directory with no manifest is not a member")
}

/// larry#311: the manifest is read as the repository's own file, so a symlink, bytes that are not
/// UTF-8 or an unclosed member array is a typed refusal naming `Cargo.toml`.
#[test]
fn sim_a_manifest_refusal_names_the_file() -> SimResult {
    let tree = Scratch::new("temp-manifest-link")?;
    named(&tree, "elsewhere.toml", MANIFEST)?;
    std::os::unix::fs::symlink(
        tree.path().join("elsewhere.toml"),
        tree.path().join("Cargo.toml"),
    )?;
    let message = stopped(&tree)?;
    assert!(
        message.contains("Cargo.toml") && message.contains("symbolic link"),
        "a symlinked manifest is refused: {message}"
    );
    let broken = Scratch::new("temp-manifest-bytes")?;
    broken.write("Cargo.toml", b"[workspace]\nmembers = [\"\xff\"]\n")?;
    let message = stopped(&broken)?;
    assert!(
        message.contains("Cargo.toml") && message.contains("UTF-8"),
        "a manifest that is not UTF-8 is refused: {message}"
    );
    let unclosed = Scratch::new("temp-manifest-open")?;
    named(
        &unclosed,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/one\"\n",
    )?;
    let message = stopped(&unclosed)?;
    assert!(
        message.contains("members"),
        "an unclosed member array is refused: {message}"
    );
    Ok(())
}
