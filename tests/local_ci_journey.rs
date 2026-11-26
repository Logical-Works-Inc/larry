//! `cargo local-ci` names where its receipt goes before it runs anything (larry#440, ADR 0123).
//! Source: POSIX.1-2024 "Environment Variables" (a value is a byte string, so a path a program
//! cannot read as text must be refused, not guessed); CWE-20, improper input validation; the
//! 2026-11 worktree runs, where one fixed `$HOME/wwfd/state` held receipts from separate
//! checkouts. Requirement: a bad receipt directory from the flag or `LARRY_STATE_DIR` exits 1
//! naming its source, before git runs and before any lane, and writes nothing.

#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use temp_dir::TempDir;

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The shipped `larry-local-ci` with `args` and `state` as `LARRY_STATE_DIR`, run from `dir`, which
/// is not a git checkout: a run that got as far as git would fail there with another error. Only
/// `PATH` and the test runner's own names pass, so no `GIT_DIR` a commit hook exported leaks in.
fn local_ci(dir: &Path, args: &[&str], state: Option<&OsStr>) -> Check<Output> {
    let runner = std::env::vars_os().filter(|pair| {
        let name = pair.0.to_string_lossy();
        name == "PATH" || name.starts_with("NEXTEST_") || name.starts_with("LGWKS_MUTANT")
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry-local-ci"));
    command.args(args).current_dir(dir).env_clear().envs(runner);
    if let Some(state) = state {
        command.env("LARRY_STATE_DIR", state);
    }
    Ok(command.output()?)
}

/// Exit 1, the refusal on stderr, and nothing written in the directory the run started from.
fn refused(dir: &Path, output: &Output, names: &str) -> Check {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains(names),
        "the refusal names {names}: {stderr}"
    );
    assert!(
        !stderr.contains("git could not report"),
        "the directory is refused before git runs: {stderr}"
    );
    assert!(
        fs::read_dir(dir)?.next().is_none(),
        "a refused run wrote nothing"
    );
    Ok(())
}

/// A `LARRY_STATE_DIR` that is not UTF-8 exits 1 naming the variable, before git or any lane.
#[cfg(unix)]
#[test]
fn a_non_unicode_state_dir_exits_one_naming_the_variable() -> Check {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new()?;
    let output = local_ci(&dir, &[], Some(OsStr::from_bytes(b"state\xff")))?;
    refused(&dir, &output, "LARRY_STATE_DIR is not valid UTF-8")
}

/// An empty `LARRY_STATE_DIR` and an empty `--receipts` each exit 1 naming their source.
#[test]
fn an_empty_receipt_directory_exits_one_naming_its_source() -> Check {
    let dir = TempDir::new()?;
    let output = local_ci(&dir, &[], Some(OsStr::new("")))?;
    refused(&dir, &output, "LARRY_STATE_DIR is empty")?;
    let output = local_ci(&dir, &["--receipts", ""], Some(OsStr::new("state")))?;
    refused(&dir, &output, "--receipts is empty")
}

/// `--receipts` with no directory, or with a second word after it, exits 1 with the usage.
#[test]
fn a_receipts_flag_without_one_directory_exits_one_with_the_usage() -> Check {
    let dir = TempDir::new()?;
    let output = local_ci(&dir, &["--receipts"], None)?;
    refused(&dir, &output, "--receipts needs a directory")?;
    let output = local_ci(&dir, &["--receipts", "a", "b"], None)?;
    refused(
        &dir,
        &output,
        "usage: larry-local-ci [--list | --receipts <dir>]",
    )
}
