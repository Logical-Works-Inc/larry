//! The temp-path rule (larry#267): in test code a temp directory may be named with the process id
//! only inside the one guard that empties the directory before use and removes it on drop. A
//! fixture named `<label>-<pid>` that is never removed lands a later run in a dead run's tree when
//! the system reuses that id: larry#243 measured 3,566 such directories in `$TMPDIR`, and larry#267
//! names six fixtures whose stale contents failed the tests that followed them.
//!
//! Source: POSIX `mkstemp(3)` (a temporary name must be unique and owned by its maker) and
//! CWE-377 (insecure temporary file). The rule is a check over source text, never a runtime probe.

use std::io;
use std::path::{Path, PathBuf};

/// The one guard of the crate's tests and the one guard of the integration tests. Each empties its
/// directory before use and removes it on drop (`executor::sim_tree::scratch`, `TempDir` behind
/// `support/temp_dir.rs`), so the process id there names a directory its owner replaces and then
/// removes, never a leftover. Every other temp path in test code reaches its directory through one
/// of them.
const GUARDS: [&str; 2] = ["src/executor/sim_tree.rs", "tests/support/temp_dir.rs"];

/// The identifier that makes a temp directory's name age with the process.
const PID: &str = "std::process::id()";

/// How a temp directory is built.
const TEMP: &str = "env::temp_dir()";

/// The largest source file the rule reads, in bytes.
const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

/// The directories the rule reads, in order.
const ROOTS: [&str; 2] = ["src", "tests"];

/// Every refusal the rule raises over the repository at `root`, in path and line order.
pub(crate) fn errors(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let mut sources = Vec::new();
    for directory in ROOTS {
        match collect(&root.join(directory), directory, &mut sources) {
            Ok(()) => {}
            // A fixture tree holds the documents alone; no source directory is not a refusal.
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => errors.push(format!("unreadable source tree: {directory}: {cause}")),
        }
    }
    sources.sort();
    for relative in sources {
        if GUARDS.contains(&relative.as_str()) {
            continue;
        }
        collect_errors(root, &relative, &mut errors);
    }
    errors
}

/// The refusals one source file raises, pushed onto `errors`.
fn collect_errors(root: &Path, relative: &str, errors: &mut Vec<String>) {
    let bytes = match std::fs::read(root.join(relative)) {
        Ok(bytes) => bytes,
        Err(cause) => {
            errors.push(format!("unreadable source file: {relative}: {cause}"));
            return;
        }
    };
    if bytes.len() > MAX_SOURCE_BYTES {
        errors.push(format!("source file over 2 MiB: {relative}"));
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    if is_test_code(relative, &text) {
        errors.extend(pid_temp_paths(relative, &text));
    }
}

/// Every `.rs` file beneath `path`, as a root-relative path with `/`.
fn collect(path: &Path, relative: &str, found: &mut Vec<String>) -> io::Result<()> {
    let mut names: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        names.push(entry?.path());
    }
    names.sort();
    for entry in names {
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let child = format!("{relative}/{name}");
        if entry.is_dir() {
            collect(&entry, &child, found)?;
        } else if name.ends_with(".rs") {
            found.push(child);
        }
    }
    Ok(())
}

/// Whether `relative` is test code: every file under `tests/`, and an in-crate file that is a
/// simulation module, a tests module or holds one behind `#[cfg(test)]`.
fn is_test_code(relative: &str, text: &str) -> bool {
    if relative.starts_with("tests/") {
        return true;
    }
    let Some(stem) = Path::new(relative)
        .file_stem()
        .and_then(|stem| stem.to_str())
    else {
        return text.contains("#[cfg(test)]");
    };
    stem.starts_with("sim_")
        || stem.ends_with("_tests")
        || stem == "tests"
        || text.contains("#[cfg(test)]")
}

/// Every line of `relative` whose code names a temp directory with a process id.
fn pid_temp_paths(relative: &str, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut errors = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (index, line) in lines.iter().enumerate() {
        let code = code_matches(line, &mut quoted, &mut escaped);
        if code.is_empty() {
            continue;
        }
        if statement(&lines, index)
            .iter()
            .any(|line| line.contains(TEMP))
        {
            errors.push(format!(
                "a process id names a temp directory outside the guard: {relative}:{}",
                index.saturating_add(1)
            ));
        }
    }
    errors
}

/// The byte positions of [`PID`] on `line` that lie in code, carrying the quote state across lines
/// so a fixture's own source text, written as a string, is data and not code.
fn code_matches(line: &str, quoted: &mut bool, escaped: &mut bool) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut index = 0_usize;
    while index < line.len() {
        if line.get(index..).is_some_and(|rest| rest.starts_with(PID)) {
            if !*quoted {
                positions.push(index);
            }
            index = index.saturating_add(PID.len());
            continue;
        }
        let Some(character) = line.get(index..).and_then(|rest| rest.chars().next()) else {
            break;
        };
        if *escaped {
            *escaped = false;
        } else if *quoted && character == '\\' {
            *escaped = true;
        } else if character == '"' {
            *quoted = !*quoted;
        }
        index = index.saturating_add(character.len_utf8());
    }
    positions
}

/// The lines of the statement that holds `line`, so a `format!` split over lines keeps the
/// `temp_dir()` call that belongs to its process id.
fn statement<'a>(lines: &'a [&'a str], at: usize) -> &'a [&'a str] {
    let mut start = at;
    while start > 0 && !ends_a_statement(lines[start.saturating_sub(1)]) {
        start = start.saturating_sub(1);
    }
    let mut end = at;
    while end.saturating_add(1) < lines.len() && !ends_a_statement(lines[end]) {
        end = end.saturating_add(1);
    }
    match lines.get(start..=end) {
        Some(window) => window,
        None => &[],
    }
}

/// Whether `line` ends a statement: a call, a block that opens or a block that closes.
fn ends_a_statement(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.ends_with(';') || trimmed.ends_with('{') || trimmed.ends_with('}')
}
