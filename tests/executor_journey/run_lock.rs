//! A leftover run lock that cannot be opened (larry#619), split from
//! `tests/executor_journey.rs` so each file stays one responsibility. Source: CWE-755 (one
//! handled-wrong fault stops every later run), POSIX process groups (`kill(-pgid, 0)` probes
//! one) and open(2) (`EISDIR`: a directory cannot be opened for writing). Requirement proved:
//! an unknown liveness is never read as dead (larry#334), and one run's leftover never stops a
//! later run.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::exec_cli::deadline::await_group_exit;
use crate::exec_cli::{TempDir, TestResult, fixture, report_of, run_exec, stderr_of, task_json};
use crate::held_group::{ledger_rows, spawn_held_command};

/// The one run lock a killed executor left under `state`.
fn left_lock(state: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut locks = Vec::new();
    for entry in fs::read_dir(state.join("journals"))? {
        let path = entry?.path();
        if path.extension() == Some(OsStr::new("lock")) {
            locks.push(path);
        }
    }
    match *locks.as_slice() {
        [ref lock] => Ok(lock.clone()),
        _ => Err(format!("the killed run left one lock: {locks:?}").into()),
    }
}

/// A lock replaced by a directory, as a root-owned file left by a killed `sudo larry exec` would
/// be just as unopenable: the next start reports it, keeps the journal, the recorded group and
/// its row, and runs its task; once the lock can be opened, the start after that reaps the group
/// and recovers the journal.
#[test]
fn a_run_lock_that_cannot_be_opened_never_stops_a_later_run() -> TestResult {
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let (mut child, pid, _hold) = spawn_held_command(state.path(), origin.path(), scratch.path())?;
    child.kill()?;
    child.wait()?;
    let lock = left_lock(state.path())?;
    fs::remove_file(&lock)?;
    fs::create_dir(&lock)?;
    let db = state.path().join("executor.db");

    let next = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let output = run_exec(state.path(), &next, None)?;
    report_of(&output)?;
    let stderr = stderr_of(&output);
    for event in [r#""reap":"kept""#, r#""recovery":"kept""#] {
        let reported = stderr.lines().filter(|line| line.contains(event)).count();
        assert_eq!(reported, 1, "one {event} line: {stderr}");
    }
    assert!(
        lgwks_std::process::process_group_exists(pid)?,
        "an unknown liveness signals no group"
    );
    assert_eq!(ledger_rows(&db)?, 1, "the row stays for a later start");
    assert!(
        lock.with_extension("journal").is_file(),
        "the journal stays a journal"
    );

    fs::remove_dir(&lock)?;
    report_of(&run_exec(state.path(), &next, None)?)?;
    await_group_exit(pid)?;
    assert_eq!(ledger_rows(&db)?, 0);
    assert!(lock.with_extension("recovered").is_file());
    Ok(())
}
