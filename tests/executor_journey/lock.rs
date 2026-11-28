//! A run lock nobody can open (larry#591): the shipped `larry exec` is killed while its
//! command runs, then its `<task>.lock` is spoiled, and the next start still runs.
//!
//! Source: `open(2)` (`EISDIR` for a directory opened to write, `EACCES` for a file its mode
//! closes, `ENXIO` for a FIFO opened to write with no reader under `O_NONBLOCK`, and a wait
//! with no end without it) and POSIX process groups (`kill(-pgid, 0)` probes one).
//! Requirement proved: an unopenable lock proves nothing. The reap signals none of that
//! run's groups and keeps its ledger row, recovery keeps its journal and says so on one
//! typed line, the new run seals its report, and once the fault is gone the next start
//! reaps and recovers the dead run.

use std::error::Error;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;

use crate::exec_cli::deadline::{await_event, await_group_exit};
use crate::exec_cli::{
    TempDir, TestResult, fixture, make_fifo, report_of, spawn_exec, stderr_of, task_json,
};
use crate::held_group::{ledger_rows, spawn_held_command};

/// What stands where the run lock was.
#[derive(Debug, Clone, Copy)]
enum Fault {
    /// A directory in the lock's place.
    Directory,
    /// The lock file with every mode bit cleared, as another user's file reads.
    NoMode,
    /// A FIFO in the lock's place, which a plain open to write waits on for ever.
    Fifo,
}

/// Puts `fault` where `lock` is.
fn spoil(lock: &Path, fault: Fault) -> Result<(), Box<dyn Error>> {
    match fault {
        Fault::Directory => {
            fs::remove_file(lock)?;
            fs::create_dir(lock)?;
        }
        Fault::NoMode => fs::set_permissions(lock, fs::Permissions::from_mode(0o000))?,
        Fault::Fifo => {
            fs::remove_file(lock)?;
            make_fifo(lock)?;
        }
    }
    Ok(())
}

/// Takes `fault` away again, as the person who found the file would.
fn heal(lock: &Path, fault: Fault) -> Result<(), Box<dyn Error>> {
    match fault {
        Fault::Directory => fs::remove_dir(lock)?,
        Fault::NoMode => fs::set_permissions(lock, fs::Permissions::from_mode(0o600))?,
        Fault::Fifo => fs::remove_file(lock)?,
    }
    Ok(())
}

/// The one running journal under `state`.
fn only_journal(state: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(state.join("journals"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "journal") {
            found.push(path);
        }
    }
    match (found.pop(), found.is_empty()) {
        (Some(path), true) => Ok(path),
        other => Err(format!("the killed run left one journal, found {other:?}").into()),
    }
}

/// Runs one passing task to its end inside the failure deadline, so a start that waits on a
/// lock for ever fails by name.
fn next_run(state: &Path, origin: &Path) -> Result<Output, Box<dyn Error>> {
    let task = task_json(origin, "HEAD", r#"[["true"]]"#);
    let mut child = spawn_exec(Some(state), None, &task)?;
    await_event("the next larry exec ends", || Ok(child.try_wait()?))?;
    Ok(child.wait_with_output()?)
}

/// The kept-journal lines of one run's standard error.
fn kept_lines(output: &Output) -> Vec<String> {
    stderr_of(output)
        .lines()
        .filter(|line| line.contains(r#""recovery":"kept""#))
        .map(str::to_owned)
        .collect()
}

#[test]
fn an_unopenable_run_lock_never_refuses_the_next_run() -> TestResult {
    for fault in [Fault::Directory, Fault::NoMode, Fault::Fifo] {
        let (state, origin) = fixture()?;
        let scratch = TempDir::new()?;
        let (mut child, pid, hold) =
            spawn_held_command(state.path(), origin.path(), scratch.path())?;
        child.kill()?;
        child.wait()?;
        let journal = only_journal(state.path())?;
        let lock = journal.with_extension("lock");
        spoil(&lock, fault)?;

        let first = next_run(state.path(), origin.path())?;
        report_of(&first)?;
        let kept = kept_lines(&first);
        let named = journal.file_name().and_then(|name| name.to_str());
        assert!(
            kept.len() == 1
                && named.is_some_and(|name| kept.iter().all(|line| line.contains(name))),
            "{fault:?}: one typed line names the kept journal, got {:?}",
            stderr_of(&first)
        );
        assert!(
            kept.iter()
                .all(|line| line.contains("run lock cannot be opened")),
            "{fault:?}: the line says why, got {kept:?}"
        );
        assert!(journal.exists(), "{fault:?}: it stays a journal");
        assert!(
            lgwks_std::process::process_group_exists(pid)?,
            "{fault:?}: the unprovable run's group is not signalled"
        );
        let db = state.path().join("executor.db");
        assert_eq!(ledger_rows(&db)?, 1, "{fault:?}: its ledger row stays");

        heal(&lock, fault)?;
        let healed = next_run(state.path(), origin.path())?;
        report_of(&healed)?;
        assert_eq!(
            kept_lines(&healed),
            Vec::<String>::new(),
            "{fault:?}: a healed lock is not reported again"
        );
        await_group_exit(pid)?;
        assert_eq!(ledger_rows(&db)?, 0, "{fault:?}: the healed run is reaped");
        assert!(
            journal.with_extension("recovered").exists(),
            "{fault:?}: the healed run's journal is recovered"
        );
        drop(hold);
    }
    Ok(())
}
