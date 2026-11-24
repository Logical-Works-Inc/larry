//! Every error type takes its `Display`, `Error` and `From` from the estate derive (larry#435).
//! Requirement proved: no source of the three workspace members holds a hand-written
//! `std::error::Error` impl or a hand-written `Display` for a type named `*Error`, except a form
//! the derive refuses, which is named in [`REFUSED`] with the reason; a mixed derive and manual
//! impl does not compile, so this is the whole acceptance.
//! Source: lgwks_macros 2.1.4 `Error`, expanded by lgwks_ast 6.1.1 `error_derive` (`Display`,
//! `core::error::Error::source` and `From` for every `#[from]` field).

use std::error::Error;
use std::path::{Path, PathBuf};

/// The members whose sources are read: the root package and the two crates.
const MEMBERS: [&str; 3] = ["", "crates/larry-contracts", "crates/larry-runtime"];

/// The source directories read in each member.
const ROOTS: [&str; 2] = ["src", "tests"];

/// The deepest directory the walk enters, so a link cycle or a runaway tree ends.
const MAX_DEPTH: usize = 16;

/// The most sources one run reads.
const MAX_FILES: usize = 4096;

/// The largest source read, in bytes.
const MAX_BYTES: u64 = 1 << 21;

/// The files and types the derive refuses, with the line of its parser that refuses them. An
/// entry is never a way to keep an impl the derive accepts: the first two hold a source as an
/// `Option<..>`, which the derive wraps in `Some` and unwraps only for a `Box` (lgwks_ast 6.1.1
/// `error_derive/expand.rs:402-443`, `parse.rs:243`); the third names the `io::Error` inside a
/// field that is not itself an `Error`, which a derive source (the whole field) cannot reach.
const REFUSED: &[(&str, &str, &str)] = &[
    (
        "src/mutants.rs",
        "Fault",
        "`Read.cause` is a `ReadFault`, no `Error`; its source is the `io::Error` inside",
    ),
    (
        "src/model/route.rs",
        "RouteError",
        "`Damaged.cause` is `Option<Box<RouteError>>`",
    ),
    (
        "src/rerank.rs",
        "RerankError",
        "`ReceiptPath` holds an `Option<io::Error>` source",
    ),
];

/// Every `.rs` file under `directory`, in path order, never following a link.
fn walk(directory: &Path, depth: usize, found: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    if depth > MAX_DEPTH {
        return Err(format!("{} nests deeper than {MAX_DEPTH}", directory.display()).into());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        entries.push(entry?.path());
    }
    entries.sort();
    for path in entries {
        let kind = std::fs::symlink_metadata(&path)?.file_type();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(&path, depth.saturating_add(1), found)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
        if found.len() > MAX_FILES {
            return Err(format!(
                "more than {MAX_FILES} sources under {}",
                directory.display()
            )
            .into());
        }
    }
    Ok(())
}

/// The type a line's `impl .. for <Type>` names, when the line opens an impl of `trait_name`.
fn implemented<'line>(line: &'line str, trait_name: &str) -> Option<&'line str> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("impl") {
        return None;
    }
    let marker = format!("{trait_name} for ");
    let after = trimmed.get(trimmed.find(&marker)?.saturating_add(marker.len())..)?;
    after
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .next()
}

/// Every hand-written impl the derive replaces, as `path:line: type`.
fn hand_written() -> Result<Vec<String>, Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for member in MEMBERS {
        for directory in ROOTS {
            walk(&root.join(member).join(directory), 0, &mut found)?;
        }
    }
    let mut left = Vec::new();
    for path in found {
        if std::fs::metadata(&path)?.len() > MAX_BYTES {
            return Err(format!("{} is past {MAX_BYTES} bytes", path.display()).into());
        }
        let text = std::fs::read_to_string(&path)?;
        let relative = path.strip_prefix(root)?.display().to_string();
        for (index, line) in text.lines().enumerate() {
            let manual_error = implemented(line, "Error");
            let manual_display =
                implemented(line, "Display").filter(|name| name.ends_with("Error"));
            for name in manual_error.into_iter().chain(manual_display) {
                let refused = REFUSED
                    .iter()
                    .any(|&(file, kind, _)| file == relative && kind == name);
                if !refused {
                    left.push(format!("{relative}:{}: {name}", index.saturating_add(1)));
                }
            }
        }
    }
    Ok(left)
}

/// No error type of the workspace keeps a hand-written `Error` or `Display` impl: each is
/// `#[derive(lgwks_macros::Error)]`, so a variant added later cannot forget its `source`.
#[test]
fn sim_no_error_enum_keeps_a_hand_written_impl() -> Result<(), Box<dyn Error>> {
    let left = hand_written()?;
    assert!(
        left.is_empty(),
        "{} hand-written error impls remain:\n{}",
        left.len(),
        left.join("\n")
    );
    Ok(())
}

/// The reader finds an impl written every way the workspace writes one, and no line that only
/// names an error type: the guard cannot pass by missing a spelling.
#[test]
fn sim_the_guard_reads_every_spelling_of_an_impl() {
    let spellings = [
        (
            "impl std::error::Error for IdError {",
            "Error",
            Some("IdError"),
        ),
        ("impl Error for ExecError {", "Error", Some("ExecError")),
        (
            "    impl<'a> core::error::Error for Wrapped<'a> {",
            "Error",
            Some("Wrapped"),
        ),
        (
            "impl fmt::Display for ServeError {",
            "Display",
            Some("ServeError"),
        ),
        ("impl From<io::Error> for TuiError {", "Error", None),
        ("// impl std::error::Error for Commented {", "Error", None),
        ("let impl_error = Error for;", "Error", None),
    ];
    for (line, trait_name, wanted) in spellings {
        assert_eq!(implemented(line, trait_name), wanted, "{line}");
    }
}
