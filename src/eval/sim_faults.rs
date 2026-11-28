//! Seeded fault simulations of `larry eval`'s infrastructure paths (larry#87, larry#399,
//! larry#576). Sources: POSIX `open` and `unlink` errors (EISDIR, ENOTDIR), a PATH that
//! lost its tools, and a non-UTF-8 state directory. Requirement: a fault that is not the
//! arm's doing is a typed error naming the task or the step, never a score, and an origin
//! the run could not finish is removed or left for the next sweep.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};

use lgwks_std::seeded::Seeded;

use crate::eval::{
    EvalError, EvalPath, EvalTask, Origin, Prices, run, run_in, session_acceptance_passed,
    session_outcome,
};
use crate::executor::{ExecError, Settings, TaskError, origin_lock_path};
use crate::sim_error::SimError;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Rounds each seeded sweep runs; each round spawns real processes.
const ROUNDS: u64 = 4;

/// Settings over `state` whose PATH is `path` and which holds the `extra` variables.
fn settings(state: &Path, path: &str, extra: &[(&str, &str)]) -> Result<Settings, ExecError> {
    let state = state.to_string_lossy().into_owned();
    let path = path.to_owned();
    let extra: Vec<(String, String)> = extra
        .iter()
        .map(|&(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(state.clone()),
        "PATH" => Some(path.clone()),
        other => extra
            .iter()
            .find(|pair| pair.0 == other)
            .map(|pair| pair.1.clone()),
    })
}

/// A command task named `id` over one origin file.
fn task(id: &str) -> EvalTask {
    EvalTask {
        id: id.to_owned(),
        files: vec![(String::from("README.md"), String::from("seed\n"))],
        goal: String::from("write o.txt"),
        path: EvalPath::Command,
        write: vec![String::from("o.txt")],
        acceptance: vec![String::from("test -s README.md")],
    }
}

/// An origin handle over `dir` that no drop cleans up.
fn unowned(state: &Path, dir: PathBuf) -> Result<Origin, io::Error> {
    let lock = state.join("unowned.lock");
    Ok(Origin {
        dir,
        _held: fs::File::create(&lock)?,
        lock,
        cleaned: true,
    })
}

/// The id and cause of an executor failure, or the error it was instead.
fn exec_failure<T>(result: Result<T, EvalError>) -> Result<(String, ExecError), SimError> {
    match result {
        Err(EvalError::Exec { id, cause }) => Ok((id, cause)),
        Err(_) => Err(SimError::Missing("an executor failure, not another error")),
        Ok(_) => Err(SimError::Missing("the fault, which went unreported")),
    }
}

/// The name `Origin::create` gives origin `index` of this process.
fn origin_name(index: usize) -> String {
    format!("origin-{}-{index}", std::process::id())
}

/// broken-machine: for seeded task ids and indexes, a lock path that is a directory, an
/// origin path that is a file and a PATH with no `git` are each the typed failure of that
/// step naming that task, and a stale origin a dead run left is replaced whole. Catches a
/// fault scored as the arm's, an origin built over another run's files, and a partial
/// origin left behind after the seed commit failed.
#[test]
fn sim_eval_every_origin_fault_is_typed_and_names_its_task() -> TestResult {
    let host = std::env::var("PATH")?;
    for seed in 0..ROUNDS {
        let mut rng = Seeded::from_seed(seed);
        let index = rng.index(64)?;
        let id = format!("task-{:08x}", rng.below(1 << 32)?);
        let state = crate::executor::sim_tree::scratch("larry-eval-faults")?;
        let eval = state.join("eval");
        let name = origin_name(index);
        let dir = eval.join(&name);
        let lock = origin_lock_path(&eval, &name);
        let wanted = task(&id);
        let healthy = settings(&state, &host, &[])?;

        fs::create_dir_all(&lock)?;
        let (named, cause) = exec_failure(Origin::create(&healthy, index, &wanted))?;
        assert_eq!(named, id, "seed {seed}: the lock fault names its task");
        assert!(
            matches!(cause, ExecError::Workspace(ref why) if why.kind() == io::ErrorKind::IsADirectory),
            "seed {seed}: {cause}"
        );
        fs::remove_dir(&lock)?;

        fs::write(&dir, b"a file where the origin goes")?;
        match Origin::create(&healthy, index, &wanted) {
            Err(EvalError::Origin { id: named, cause }) => {
                assert_eq!(named, id, "seed {seed}: the origin fault names its task");
                assert_eq!(cause.kind(), io::ErrorKind::NotADirectory, "seed {seed}");
            }
            Err(other) => return Err(format!("seed {seed}: {other}").into()),
            Ok(_) => return Err(format!("seed {seed}: an origin was built over a file").into()),
        }
        fs::remove_file(&dir)?;

        let bare = state.join("no-tools");
        fs::create_dir_all(&bare)?;
        let blind = settings(&state, &bare.to_string_lossy(), &[])?;
        let (named, cause) = exec_failure(Origin::create(&blind, index, &wanted))?;
        assert_eq!(named, id, "seed {seed}: the seed fault names its task");
        assert!(cause.to_string().contains("git"), "seed {seed}: {cause}");
        assert!(!dir.exists(), "seed {seed}: the partial origin is removed");
        assert!(!lock.exists(), "seed {seed}: its lock is removed");

        fs::create_dir_all(&dir)?;
        fs::write(dir.join("stale.txt"), b"a dead run's file")?;
        let origin = Origin::create(&healthy, index, &wanted)?;
        assert!(!dir.join("stale.txt").exists(), "seed {seed}: stale gone");
        assert_eq!(fs::read(dir.join("README.md"))?, b"seed\n", "seed {seed}");
        origin.remove()?;
        assert!(!dir.exists() && !lock.exists(), "seed {seed}: all removed");
    }
    Ok(())
}

/// broken-machine (larry#343): an origin dropped while its tree cannot be removed keeps the
/// entry for the next sweep and still removes the lock, so the sweep sees it unheld.
/// Catches a drop that skips the lock once the tree failed.
#[test]
fn sim_eval_a_dropped_origin_it_cannot_remove_keeps_the_tree_and_frees_the_lock() -> TestResult {
    let state = crate::executor::sim_tree::scratch("larry-eval-drop")?;
    let dir = state.join("not-a-directory");
    fs::write(&dir, b"x")?;
    let mut origin = unowned(&state, dir.clone())?;
    origin.cleaned = false;
    let lock = origin.lock.clone();
    drop(origin);
    assert!(dir.exists(), "the entry is left for the next sweep");
    assert!(!lock.exists(), "the lock is released and removed");
    Ok(())
}

/// Puts a directory where `state` keeps its database, replacing any database there.
fn break_store(state: &Path) -> io::Result<()> {
    let database = state.join("executor.db");
    if database.try_exists()? {
        fs::remove_file(&database)?;
    }
    fs::create_dir_all(&database)
}

/// Whether `cause` is the key store refusing a directory where its database goes.
fn key_store_is_a_directory(cause: &ExecError) -> bool {
    matches!(cause, ExecError::KeyStore(why) if why.kind() == io::ErrorKind::IsADirectory)
}

/// broken-machine (larry#217): a session verifier whose origin is gone, so `sh` cannot start
/// in it, or whose ledger cannot open, is an infrastructure failure naming the task, never a
/// rejected session. The PATH that lacks `sh` is larry#598.
#[test]
fn sim_eval_a_verifier_the_host_cannot_run_is_never_a_rejection() -> TestResult {
    let host = std::env::var("PATH")?;
    let state = crate::executor::sim_tree::scratch("larry-eval-verifier")?;
    let wanted = task("verify");
    let configured = settings(&state, &host, &[])?;

    let gone = unowned(&state, state.join("never-made"))?;
    let (named, cause) = exec_failure(session_acceptance_passed(&gone, &configured, &wanted))?;
    assert_eq!(named, "verify");
    assert!(matches!(cause, ExecError::Spawn { .. }), "{cause}");

    let work = state.join("work");
    fs::create_dir_all(&work)?;
    let origin = unowned(&state, work)?;
    let broken = state.join("broken");
    let unopenable = settings(&broken, &host, &[])?;
    break_store(&broken)?;
    let (named, cause) = exec_failure(session_acceptance_passed(&origin, &unopenable, &wanted))?;
    assert_eq!(named, "verify");
    assert!(key_store_is_a_directory(&cause), "{cause}");
    Ok(())
}

/// fat-finger and hostile-path: a session whose origin is gone, a command task with no
/// acceptance command and a state directory that is not UTF-8 (CWE-176) each stop with
/// their typed error before any model call. Catches an unrunnable task scored as a
/// refusal or a rejection.
#[test]
fn sim_eval_a_task_that_cannot_start_stops_the_arm_typed() -> TestResult {
    let host = std::env::var("PATH")?;
    let state = crate::executor::sim_tree::scratch("larry-eval-unstartable")?;
    let configured = settings(&state, &host, &[])?;

    let missing = unowned(&state, state.join("never-made"))?;
    let mut session = task("gone");
    session.path = EvalPath::Session;
    let (named, cause) = exec_failure(session_outcome(&missing, &configured, &session))?;
    assert_eq!(named, "gone");
    assert!(matches!(cause, ExecError::Session(_)), "{cause}");

    let work = unowned(&state, state.join("work"))?;
    let mut unchecked = task("unchecked");
    unchecked.acceptance = Vec::new();
    let (named, cause) = exec_failure(run_in(&work, &configured, &unchecked))?;
    assert_eq!(named, "unchecked");
    assert!(
        matches!(cause, ExecError::Task(TaskError::CommandCount { count: 0 })),
        "{cause}"
    );

    let raw = PathBuf::from(OsString::from_vec(b"/nowhere/\xff".to_vec()));
    let unspeakable = unowned(&state, raw.clone())?;
    match run_in(&unspeakable, &configured, &task("bytes")) {
        Err(EvalError::StatePath(path)) => assert_eq!(path, raw),
        Err(other) => return Err(format!("not a state path refusal: {other}").into()),
        Ok(outcome) => return Err(format!("a non-UTF-8 origin ran: {outcome:?}").into()),
    }
    Ok(())
}

/// broken-machine: an arm whose route ledger cannot open stops as `(route)`, and one whose
/// origin area is a file stops as `(sweep)`; neither scores a card. Catches a broken state
/// directory reported as an unconfigured model.
#[test]
fn sim_eval_a_broken_state_directory_stops_before_the_first_task() -> TestResult {
    let host = std::env::var("PATH")?;
    let state = crate::executor::sim_tree::scratch("larry-eval-state")?;
    let set = br#"{"name":"s","tasks":[{"id":"one","files":{"README.md":"x"},"goal":"g","write":["o.txt"],"acceptance":["true"]}]}"#;
    let route = [
        ("LARRY_MODEL_URL", "http://127.0.0.1:1/v1"),
        ("LARRY_MODEL", "sim-model"),
        ("LARRY_MODEL_KEY", "k"),
    ];
    let prices = || Ok(Prices::micro(1, 1));

    let broken = state.join("route");
    let configured = settings(&broken, &host, &route)?;
    break_store(&broken)?;
    let (named, cause) = exec_failure(run(set, &configured, prices()))?;
    assert_eq!(named, "(route)");
    assert!(key_store_is_a_directory(&cause), "{cause}");

    let swept = state.join("sweep");
    fs::create_dir_all(&swept)?;
    fs::write(swept.join("eval"), b"a file where origins go")?;
    let (named, cause) = exec_failure(run(set, &settings(&swept, &host, &route)?, prices()))?;
    assert_eq!(named, "(sweep)");
    assert!(
        cause.to_string().to_lowercase().contains("not a directory"),
        "{cause}"
    );
    Ok(())
}
