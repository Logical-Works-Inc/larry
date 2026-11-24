//! Larry names `lgwks_bot` only through the surface that carries its compatibility promise.
//!
//! Source: the `lgwks_bot` 10.0.2 crate docs and `src/kernel.rs` (logicalworks-crates#526, #845,
//! #1016): `script!`, `app` and `kernel` are the supported surface, and every other module is
//! `#[doc(hidden)]` support code with no promise, so a minor release may move it. The deps audit
//! of 2026-11-24 found 20 sources that named the support paths of `kernel`'s own modules.
//! Requirement: no Rust source of any workspace member names a `kernel` module by its support
//! path, and the members this test walks are the members the root manifest declares.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use lgwks_std::fs::{WalkOptions, walk_dir};

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The modules `lgwks_bot::kernel` re-exports. Each support path beside it carries no promise.
const KERNEL_MODULES: [&str; 9] = [
    "broker",
    "cap",
    "clock",
    "dist_lease",
    "effect",
    "gate",
    "journal",
    "retry",
    "skew",
];

/// The workspace members, the root package first, as the root manifest declares them.
const MEMBERS: [&str; 3] = ["", "crates/larry-contracts", "crates/larry-runtime"];

/// The repository root.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The support-path spellings, built here so this file never names one.
fn support_paths() -> Vec<String> {
    KERNEL_MODULES
        .iter()
        .map(|module| format!("lgwks_bot::{module}::"))
        .collect()
}

/// Every `name:line` of `text` that names one of `spellings`.
fn named_in(name: &str, text: &str, spellings: &[String]) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|&(_, line)| {
            spellings
                .iter()
                .any(|spelling| line.contains(spelling.as_str()))
        })
        .map(|(index, _)| format!("{name}:{}", index.saturating_add(1)))
        .collect()
}

/// Every `file:line` of a Rust source under `bases` that names one of `spellings`.
fn named_lines(bases: &[PathBuf], spellings: &[String]) -> Result<Vec<String>, Box<dyn Error>> {
    let mut found = Vec::new();
    for base in bases {
        for path in walk_dir(base, &WalkOptions::default())? {
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            found.extend(named_in(&path.display().to_string(), &text, spellings));
        }
    }
    Ok(found)
}

#[test]
fn larry_names_lgwks_bot_only_through_its_supported_paths() -> TestResult {
    let manifest = fs::read_to_string(root().join("Cargo.toml"))?;
    let declared = r#"members = ["crates/larry-contracts", "crates/larry-runtime"]"#;
    assert!(
        manifest.contains(declared),
        "the root manifest declares other members than this test walks"
    );
    let bases: Vec<PathBuf> = MEMBERS
        .iter()
        .flat_map(|member| ["src", "tests"].map(|dir| root().join(member).join(dir)))
        .collect();
    let found = named_lines(&bases, &support_paths())?;
    assert!(
        found.is_empty(),
        "name these through lgwks_bot::kernel: {found:?}"
    );
    Ok(())
}

#[test]
fn a_support_path_is_found_and_a_kernel_or_app_path_is_not() {
    let spellings = support_paths();
    let support = format!("use std::fmt;\nuse lgwks_bot::{}::Clock;\n", "clock");
    assert_eq!(
        named_in("a.rs", &support, &spellings),
        ["a.rs:2"],
        "the support path is found on its own line"
    );
    let supported = "use lgwks_bot::kernel::clock::Clock;\nuse lgwks_bot::app::App;\n";
    assert!(
        named_in("b.rs", supported, &spellings).is_empty(),
        "a kernel or app path is the supported surface"
    );
    let every: String = KERNEL_MODULES
        .iter()
        .map(|module| format!("lgwks_bot::{module}::Item\n"))
        .collect();
    assert_eq!(
        named_in("c.rs", &every, &spellings).len(),
        KERNEL_MODULES.len(),
        "every kernel module's support path is found"
    );
}
