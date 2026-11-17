//! The temp-path rule (larry#267, larry#293): in test code a temp directory may be named with the
//! process id only inside the one guard that empties the directory before use and removes it on
//! drop. A fixture named `<label>-<pid>` that is never removed lands a later run in a dead run's
//! tree when the system reuses that id: larry#243 measured 3,566 such directories in the system
//! temporary directory, and larry#267 names six fixtures whose stale contents failed the tests
//! that followed them.
//!
//! The rule reads a syntax tree, not text and not a lexer of its own (larry#293, ADR 0061): every
//! Rust source is parsed by the estate reader `lgwks_ast`, and the rule judges one function at a
//! time. A function that reaches a temp source and names `std::process::id`, as a call or as a
//! value, through any `use` path, alias, `self` import, glob or `::std` prefix, is refused at its
//! first use. Patterns, closure arguments, method calls and a shadowing local are not the process
//! id. The rule reads the root's `src` and `tests` (larry#311 widens this to every workspace
//! member and follows `#[path]` and `include!` modules). The directory walk never follows a
//! symlink (CWE-59).
//!
//! Source: The Rust Reference on path expressions, use declarations and macros-by-example; POSIX
//! `mkstemp(3)` (a temporary name must be unique and owned by its maker); CWE-377 (insecure
//! temporary file) and CWE-59 (link following).

mod syntax;
mod uses;

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};

use lgwks_ast::{Language, MAX_SOURCE_BYTES, Parsed, try_parse};

use super::error::GovernanceError;
use syntax::{is_test_code, verdicts};

/// The one guard of the crate's tests and the one guard of the integration tests. Each empties its
/// directory before use and removes it on drop (`executor::sim_tree::scratch`, `TempDir` behind
/// `support/temp_dir.rs`), so the process id there names a directory its owner replaces and then
/// removes, never a leftover. Every other temp path in test code reaches its directory through one
/// of them.
const GUARDS: [&str; 2] = ["src/executor/sim_tree.rs", "tests/support/temp_dir.rs"];

/// The directories the rule reads, in order.
const ROOTS: [&str; 2] = ["src", "tests"];

/// The deepest nesting the walk follows, so a pathological tree ends in a refusal, never a stack
/// overflow.
const MAX_DEPTH: usize = 64;

/// The most lanes one check scans its sources on. A check reads one source per file, and the
/// parse is the cost, so a few lanes cut a whole-tree check without holding more than this many.
const LANES: usize = 4;

/// A source the rule reads: its path on disk and its root-relative name.
type Source = (PathBuf, String);

/// What reading one source file produced.
enum SourceRead {
    /// The file's bytes.
    Contents(Vec<u8>),
    /// The file is past the reader's byte bound, and this many bytes.
    TooLarge(usize),
}

/// Every refusal the rule raises over the repository at `root`, sorted, in path and line order.
pub(crate) fn errors(root: &Path) -> Result<Vec<String>, GovernanceError> {
    let mut errors = Vec::new();
    let mut queue: VecDeque<Source> = VecDeque::new();
    for directory in ROOTS {
        let relative = String::from(directory);
        let path = root.join(directory);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(GovernanceError::Unreadable {
                    relative,
                    cause: io::Error::other("a symbolic link is never followed"),
                });
            }
            Ok(metadata) if metadata.is_dir() => {
                collect(&path, &relative, 0, &mut queue, &mut errors)?;
            }
            Ok(_) => {
                return Err(GovernanceError::Unreadable {
                    relative,
                    cause: io::Error::other("not a directory"),
                });
            }
            // A fixture tree of documents alone holds no source root; larry#311 makes a missing
            // workspace-member root a typed refusal.
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => return Err(GovernanceError::Unreadable { relative, cause }),
        }
    }
    let mut seen = BTreeSet::new();
    let pending: Vec<Source> = queue
        .into_iter()
        .filter(|entry| {
            let relative = &entry.1;
            seen.insert(relative.clone()) && !GUARDS.contains(&relative.as_str())
        })
        .collect();
    for scanned in scan_all(&pending) {
        errors.extend(scanned);
    }
    sort_by_file_and_line(&mut errors);
    errors.dedup();
    Ok(errors)
}

/// Scans every pending source and returns each file's result in the order the walk found it, so a
/// report never depends on which lane finished first.
fn scan_all(pending: &[Source]) -> Vec<Vec<String>> {
    let lanes = match std::thread::available_parallelism() {
        Ok(count) => count.get(),
        Err(_) => 1,
    }
    .min(LANES)
    .min(pending.len());
    if lanes <= 1 {
        return pending
            .iter()
            .map(|source| scan(&source.0, &source.1))
            .collect();
    }
    let chunk = pending.len().div_ceil(lanes);
    std::thread::scope(|scope| {
        let mut starts: Vec<(usize, usize)> = Vec::with_capacity(lanes);
        let mut start: usize = 0;
        let mut handles = Vec::with_capacity(lanes);
        for slice in pending.chunks(chunk) {
            starts.push((start, slice.len()));
            start = start.saturating_add(slice.len());
            handles.push(scope.spawn(move || {
                slice
                    .iter()
                    .map(|source| scan(&source.0, &source.1))
                    .collect::<Vec<Vec<String>>>()
            }));
        }
        let mut scanned = Vec::with_capacity(pending.len());
        for (handle, &(start, len)) in handles.into_iter().zip(&starts) {
            match handle.join() {
                Ok(lane) => scanned.extend(lane),
                Err(_) => {
                    for source in &pending[start..start.saturating_add(len)] {
                        let relative = &source.1;
                        scanned.push(vec![format!(
                            "a source scan stopped without answering: {relative}"
                        )]);
                    }
                }
            }
        }
        scanned
    })
}

/// The order the rule reports in: by the file, then by the line inside it, so a report reads in
/// the order a person opens the files.
fn sort_by_file_and_line(errors: &mut [String]) {
    errors.sort_by_key(|error| match error.rsplit_once(':') {
        Some((file, line)) => match line.parse::<usize>() {
            Ok(line) => (String::from(file), line),
            Err(_) => (error.clone(), 0),
        },
        None => (error.clone(), 0),
    });
}

/// Every `.rs` file beneath `path`, as a root-relative path with `/`. A symlink is never followed,
/// whatever it points at, and nesting past [`MAX_DEPTH`] is refused by name.
fn collect(
    path: &Path,
    relative: &str,
    depth: usize,
    found: &mut VecDeque<Source>,
    errors: &mut Vec<String>,
) -> Result<(), GovernanceError> {
    if depth > MAX_DEPTH {
        return Err(GovernanceError::Unreadable {
            relative: String::from(relative),
            cause: io::Error::other(format!("source tree deeper than {MAX_DEPTH}")),
        });
    }
    let mut entries: Vec<(PathBuf, bool, bool)> = Vec::new();
    let unreadable = |cause: io::Error| GovernanceError::Unreadable {
        relative: String::from(relative),
        cause,
    };
    for entry in std::fs::read_dir(path).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let file_type = entry.file_type().map_err(unreadable)?;
        entries.push((entry.path(), file_type.is_dir(), file_type.is_symlink()));
    }
    entries.sort();
    for (entry, is_dir, is_symlink) in entries {
        if is_symlink {
            continue;
        }
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            errors.push(format!(
                "source tree names an entry that is not UTF-8: {relative}"
            ));
            continue;
        };
        let child = format!("{relative}/{name}");
        if is_dir {
            collect(&entry, &child, depth.saturating_add(1), found, errors)?;
        } else if name.ends_with(".rs") {
            found.push_back((entry, child));
        }
    }
    Ok(())
}

/// Reads, parses and judges one source file; a reader refusal is a message naming the file, never
/// a pass.
fn scan(path: &Path, relative: &str) -> Vec<String> {
    let contents = match read_source(path) {
        Ok(SourceRead::Contents(contents)) => contents,
        Ok(SourceRead::TooLarge(bytes)) => {
            return vec![
                GovernanceError::SourceTooLarge {
                    relative: String::from(relative),
                    bytes,
                }
                .to_string(),
            ];
        }
        Err(cause) => {
            return vec![
                GovernanceError::Unreadable {
                    relative: String::from(relative),
                    cause,
                }
                .to_string(),
            ];
        }
    };
    let text = match std::str::from_utf8(&contents) {
        Ok(text) => text,
        Err(_) => {
            return vec![
                GovernanceError::SourceNotUtf8 {
                    relative: String::from(relative),
                }
                .to_string(),
            ];
        }
    };
    let tree: Parsed = match try_parse(text, Language::Rust) {
        Ok(tree) => tree,
        Err(cause) => {
            return vec![
                GovernanceError::Source {
                    relative: String::from(relative),
                    cause,
                }
                .to_string(),
            ];
        }
    };
    if is_test_code(relative, &tree) {
        verdicts(relative, text, &tree)
    } else {
        Vec::new()
    }
}

/// Reads one source file within the reader's byte bound.
fn read_source(path: &Path) -> io::Result<SourceRead> {
    let metadata = std::fs::metadata(path)?;
    let bytes = match usize::try_from(metadata.len()) {
        Ok(bytes) => bytes,
        Err(_) => usize::MAX,
    };
    if bytes > MAX_SOURCE_BYTES {
        return Ok(SourceRead::TooLarge(bytes));
    }
    std::fs::read(path).map(SourceRead::Contents)
}
