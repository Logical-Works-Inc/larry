//! `cargo local-ci` names where its receipt goes before it runs anything (larry#440, ADR 0123).
//! Source: POSIX.1-2024 "Environment Variables" (a value is a byte string, so a path a program
//! cannot read as text must be refused, not guessed); CWE-20, improper input validation; the
//! 2026-11 worktree runs, where one fixed `$HOME/wwfd/state` held receipts from separate
//! checkouts. Requirement: a bad receipt directory from the flag or `LARRY_STATE_DIR`, or a word
//! outside the command line, exits 1 naming its source, before git runs and before any lane, and
//! writes nothing. The modules drive the rest of the run: `lanes` the gates and the receipt,
//! `faults` what happens when git, the log, the receipt or the process fails.

#[cfg(unix)]
#[path = "local_ci_journey/faults.rs"]
mod faults;
#[cfg(unix)]
#[path = "local_ci_journey/lanes.rs"]
mod lanes;
#[cfg(unix)]
#[path = "local_ci_journey/rig.rs"]
mod rig;

#[path = "support/origin.rs"]
mod origin;

#[path = "support/coverage.rs"]
mod coverage;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use origin::TempDir;

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The usage every command-line refusal ends with.
const USAGE: &str = "usage: larry-local-ci [--list | --receipts <dir>]";

/// The shipped `larry-local-ci` with `args` and `state` as `LARRY_STATE_DIR`, run from `dir`, which
/// is not a git checkout: a run that got as far as git would fail there with another error. Only
/// `PATH` and the test runner's own names pass, so no `GIT_DIR` a commit hook exported leaks in.
fn local_ci(dir: &Path, args: &[&OsStr], state: Option<&OsStr>) -> Check<Output> {
    let runner = std::env::vars_os().filter(|pair| {
        let name = pair.0.to_string_lossy();
        name == "PATH" || name.starts_with("NEXTEST_") || name.starts_with("LGWKS_MUTANT")
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry-local-ci"));
    command.args(args).current_dir(dir).env_clear().envs(runner);
    if let Some(state) = state {
        command.env("LARRY_STATE_DIR", state);
    }
    coverage::forward(&mut command);
    Ok(command.output()?)
}

/// Exit 1, exactly this refusal on stderr, an empty log, and nothing written in the directory the
/// run started from.
fn refused(dir: &Path, output: &Output, refusal: &str) -> Check {
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("local-ci: {refusal}\n"),
        "the refusal"
    );
    assert_eq!(output.status.code(), Some(1), "exit status");
    assert_eq!(output.stdout, b"", "nothing was logged, so nothing ran");
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
    let args = [OsStr::new("--receipts"), OsStr::new("")];
    let output = local_ci(&dir, &args, Some(OsStr::new("state")))?;
    refused(&dir, &output, "--receipts is empty")
}

/// `--receipts` with no directory, or with a second word after it, exits 1 with the usage.
#[test]
fn a_receipts_flag_without_one_directory_exits_one_with_the_usage() -> Check {
    let dir = TempDir::new()?;
    let output = local_ci(&dir, &[OsStr::new("--receipts")], None)?;
    let missing = format!("--receipts needs a directory; {USAGE}");
    refused(&dir, &output, &missing)?;
    let args = ["--receipts", "a", "b"].map(OsStr::new);
    let output = local_ci(&dir, &args, None)?;
    refused(&dir, &output, &format!("unknown argument \"b\"; {USAGE}"))
}

/// A word outside the command line is refused by name (CWE-88): an unknown flag, `--list` with
/// anything after it, a near miss, a path where a flag belongs, an empty word, and bytes that are
/// not UTF-8.
#[cfg(unix)]
#[test]
fn a_word_outside_the_command_line_is_refused_by_name() -> Check {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new()?;
    for (args, named) in [
        (&["--nope"][..], "--nope"),
        (&["--list", "--receipts"][..], "--list"),
        (&["--receipts=state"][..], "--receipts=state"),
        (&["state"][..], "state"),
        (&[""][..], ""),
    ] {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let output = local_ci(&dir, &args, None)?;
        let refusal = format!("unknown argument {named:?}; {USAGE}");
        refused(&dir, &output, &refusal)?;
    }
    let bytes = OsStr::from_bytes(b"--list\xff");
    let output = local_ci(&dir, &[bytes], None)?;
    let refusal = format!("unknown argument {bytes:?}; {USAGE}");
    refused(&dir, &output, &refusal)
}
