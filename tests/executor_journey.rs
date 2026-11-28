//! Journeys against the shipped `larry exec` command (larry#85), as an operator
//! lives them: pipe a task document on stdin, read one JSON report on stdout.
//!
//! Source: POSIX process conventions (exit status, stdout for the report, stderr
//! for one diagnostic line), POSIX process groups and signals (SIGKILL reaches the
//! whole group, `kill(-pgid, 0)` probes it), the git object model (a clone is
//! isolated from its origin's `.git`) and SQLite's read-only open. Requirement
//! proved: the shipped binary runs acceptance commands in its own workspace and
//! reports exit codes, output and the staged diff; it never changes the origin;
//! it kills a timed-out group; it reaps the groups a killed executor left behind;
//! it truncates noisy output; it refuses bad input with a typed one-line error;
//! and two concurrent tasks never share a workspace. Each child runs with a
//! cleared environment, so the developer's own variables never reach a test.
//! The exit status is 0 for an accepted run and 2 for a sealed rejected one
//! (larry#86); `report_of` checks that it agrees with the sealed verdict.
//! The fixture origin runs no background git maintenance, so nothing but the
//! executor can change the tree a journey hashes (git-maintenance, larry#138).
//! The busy-ledger sim proves the ledger reader's retry on its own, once, in
//! this binary (larry#309).

#[path = "executor_journey/ceiling.rs"]
mod ceiling;
#[path = "executor_journey/confine.rs"]
mod confine;
#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/exec_cli.rs"]
mod exec_cli;
#[path = "executor_journey/gate.rs"]
mod gate;
#[path = "support/held_group.rs"]
mod held_group;
#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/larry_guard.rs"]
mod larry_guard;
#[path = "executor_journey/lock.rs"]
mod lock;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/executor_residency_journey.rs"]
mod residency;
#[path = "support/tree_digest.rs"]
mod tree_digest;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use exec_cli::deadline::{await_fifo_writer, await_group_exit};
use exec_cli::{
    TempDir, TestResult, assert_refused, fixture, flag_of, live_group_pid_through, make_fifo,
    millis, number_of, percentile, report_of, result_at, run_exec, spawn_exec, spawn_larry_args,
    task_json, text_of,
};
use gate::brief_json;
use held_group::{ledger_rows, spawn_held_command};
use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};
use lgwks_std::seeded::Seeded;
use tree_digest::tree_digest;

/// Runs the latency case samples.
const SAMPLES: usize = 32;

/// The stdout cap the executor applies to each command, in bytes.
const STDOUT_CAP: usize = 1_048_576;

/// Seeds the busy-ledger simulation sweeps (larry#309).
const BUSY_SEEDS: u64 = 8;

/// A kill -9 of the shipped executor after its command's group is recorded leaves no live
/// group after the next start and no ledger row once it has reaped. The kill point is the
/// fixture's own seam: `spawn_held_command` returns only once the command holds its group
/// open on a fifo and the ledger names that group, so the kill lands inside the recorded
/// run with no count of yields. Honest limit (ADR 0042): the window between spawn and the
/// row is not closed and cannot be stopped from outside the process, so this proves the
/// reap after the row, not a hit inside that window.
#[test]
fn a_kill_after_the_recorded_group_leaves_no_group_after_restart() -> TestResult {
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let (mut child, pid, _hold) = spawn_held_command(state.path(), origin.path(), scratch.path())?;
    child.kill()?;
    child.wait()?;
    let next = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    report_of(&run_exec(state.path(), &next, None)?)?;
    await_group_exit(pid)?;
    let rows = ledger_rows(&state.path().join("executor.db"))?;
    assert_eq!(rows, 0, "the ledger holds no row after the reap");
    Ok(())
}

/// A ledger read refused with `SQLITE_BUSY` twice is retried inside the deadline and answers
/// the pid once the writer releases its lock, for every seed: a row is read after the lock is
/// released, never instead of it (larry#294). The reader takes no busy timeout, so each refusal
/// answers at once instead of after rusqlite's 5 s default, and the deadline alone bounds the
/// wait; the sim lives here so it runs once, not once per binary that includes the fixture
/// (larry#309).
#[test]
fn sim_live_group_pid_answers_after_two_busy_reads() -> TestResult {
    for seed in 0..BUSY_SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let pid = i32::try_from(rng.below(60_000)?.saturating_add(1))?;
        let dir = TempDir::new()?;
        let db = dir.join("ledger.db");
        // The writer holds the database exclusively, so the reader's own query is refused with
        // SQLITE_BUSY until the third attempt releases it.
        let writer = Connection::open(&db)?;
        writer.execute_batch(&format!(
            "CREATE TABLE live_groups (pid INTEGER); INSERT INTO live_groups VALUES ({pid}); BEGIN EXCLUSIVE;"
        ))?;
        let reader = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        reader.busy_timeout(Duration::ZERO)?;
        let mut attempts = 0_u64;
        let mut first_refusal = String::new();
        let answered = live_group_pid_through("a live group was recorded in the ledger", || {
            attempts = attempts.saturating_add(1);
            if attempts == 3 {
                writer.execute_batch("ROLLBACK")?;
            }
            match reader.query_row("SELECT pid FROM live_groups", [], |row| {
                row.get::<_, i64>(0)
            }) {
                Ok(pid) => Ok(Some(i32::try_from(pid)?)),
                Err(lgwks_deps::storefront::rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(cause) => {
                    if attempts == 1 {
                        first_refusal = cause.to_string();
                    }
                    Err(cause.into())
                }
            }
        })?;
        assert!(
            first_refusal.contains("database is locked"),
            "seed {seed}: the first read is refused because the writer holds the lock: {first_refusal}"
        );
        assert_eq!(answered, pid, "seed {seed}: the pid the row holds");
        assert_eq!(
            attempts, 3,
            "seed {seed}: two busy reads are retried, then the released row answers"
        );
    }
    Ok(())
}

/// Whether any member of process group `pgid` is still alive.
fn group_alive(pgid: i32) -> Result<bool, Box<dyn Error>> {
    Ok(lgwks_std::process::process_group_exists(pgid)?)
}

#[test]
fn a_passing_and_a_failing_command_report_their_exit_codes() -> TestResult {
    // A passing and a failing acceptance command report their own exit codes and output.
    let (state, origin) = fixture()?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["true"],["sh","-c","echo hi; echo err >&2; exit 3"]]"#,
    );
    let output = run_exec(state.path(), &task, None)?;
    let report = report_of(&output)?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "a run with a failing command is sealed as rejected"
    );
    let passing = result_at(&report, 0)?;
    let failing = result_at(&report, 1)?;
    assert_eq!(number_of(passing, "exit_code")?, 0, "`true` must exit 0");
    assert_eq!(
        number_of(failing, "exit_code")?,
        3,
        "the failing command must exit 3"
    );
    assert_eq!(
        text_of(failing, "stdout")?,
        "hi\n",
        "stdout must be captured exactly"
    );
    assert_eq!(
        text_of(failing, "stderr")?,
        "err\n",
        "stderr must be captured exactly"
    );
    assert_eq!(
        text_of(&report, "diff")?,
        "",
        "an unchanged workspace has an empty diff"
    );

    // The same task, timed: 32 runs, nearest-rank percentiles on stderr.
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let output = run_exec(state.path(), &task, None)?;
        samples.push(started.elapsed());
        let timed = report_of(&output)?;
        assert_eq!(
            number_of(result_at(&timed, 1)?, "exit_code")?,
            3,
            "every timed run must report exit 3"
        );
    }
    samples.sort();
    assert_eq!(
        journals(state.path(), "journal")?.len(),
        0,
        "a returned task removes its journal (larry#75)"
    );
    let mut err = io::stderr().lock();
    writeln!(
        err,
        "exec p50={:.3} p95={:.3} p99={:.3} n={SAMPLES}",
        millis(percentile(&samples, 50)),
        millis(percentile(&samples, 95)),
        millis(percentile(&samples, 99)),
    )?;
    Ok(())
}

#[test]
fn a_fixture_origin_starts_no_background_maintenance() -> TestResult {
    // A commit in the fixture origin, traced: git prints a run_command line for every
    // child it starts, and `maintenance run --auto --detach` would be one of them.
    let (_state, origin) = fixture()?;
    let output = Command::new("git")
        .arg("-C")
        .arg(origin.path())
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--allow-empty",
            "-qm",
            "probe",
        ])
        // The probe commit belongs to the fixture origin. A hook that exports GIT_DIR, GIT_WORK_TREE,
        // GIT_INDEX_FILE or GIT_COMMON_DIR would send it into the repository under test (larry#366).
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env("GIT_TRACE", "1")
        .output()?;
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the probe commit failed: {trace}");
    assert!(
        trace.contains("built-in: git commit"),
        "GIT_TRACE must reach git for the probe to mean anything: {trace}"
    );
    assert!(
        !trace.contains("maintenance run"),
        "a fixture commit must not fork background maintenance: {trace}"
    );
    Ok(())
}

#[test]
fn a_hostile_command_leaves_the_origin_byte_identical() -> TestResult {
    // A command that deletes and rewrites the workspace cannot touch the origin.
    let (state, origin) = fixture()?;
    let before = tree_digest(origin.path())?;
    let hostile = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","rm -rf ./* .git; echo pwned > README"]]"#,
    );
    run_exec(state.path(), &hostile, None)?;
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "a hostile command must leave the origin byte-identical"
    );
    let leftovers = fs::read_dir(state.path().join("ws"))?.count();
    assert_eq!(
        leftovers, 0,
        "the ws directory must hold no entries afterwards"
    );

    // A command that re-points the workspace's git at the origin: the executor's git
    // stays bound to its own git dir, so the diff is still the workspace's.
    let redirect = format!(
        "rm -rf .git; ln -s {}/.git .git; echo pwned > README",
        origin.path().display()
    );
    let task = task_json(
        origin.path(),
        "HEAD",
        &format!(r#"[["sh","-c","{redirect}"]]"#),
    );
    let report = report_of(&run_exec(state.path(), &task, None)?)?;
    assert_eq!(
        number_of(result_at(&report, 0)?, "exit_code")?,
        0,
        "the redirect command must exit 0"
    );
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "a git redirect must leave the origin byte-identical"
    );
    assert!(
        text_of(&report, "diff")?.contains("pwned"),
        "the diff must show the workspace's change"
    );
    Ok(())
}

#[test]
fn a_timed_out_command_has_its_whole_group_killed() -> TestResult {
    // A command blocked on a fifo nobody writes is killed with its whole group.
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let never = scratch.path().join("never");
    make_fifo(&never)?;
    let acceptance = format!(
        r#"[["sh","-c","cat < {} & cat < {}"]]"#,
        never.display(),
        never.display()
    );
    let task = task_json(origin.path(), "HEAD", &acceptance);
    let report = report_of(&run_exec(state.path(), &task, Some(300))?)?;
    let result = result_at(&report, 0)?;
    assert!(
        flag_of(result, "timed_out")?,
        "the blocked command must be reported timed out"
    );
    assert!(
        !flag_of(result, "interrupted")?,
        "a timeout is never also an interrupt (larry#242)"
    );
    assert!(
        !flag_of(&report, "accepted")?,
        "a timed-out run is never accepted"
    );
    let pgid = i32::try_from(number_of(result, "pgid")?)?;
    assert!(
        !group_alive(pgid)?,
        "process group {pgid} must not survive its timeout"
    );
    Ok(())
}

#[test]
fn a_kill_9_of_the_executor_is_reaped_by_the_next_start() -> TestResult {
    // SIGKILL the executor while its command runs; the next start reaps the orphaned group.
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let (mut child, pid, _ready_open) =
        spawn_held_command(state.path(), origin.path(), scratch.path())?;
    child.kill()?;
    child.wait()?;
    assert!(
        group_alive(pid)?,
        "the killed executor's group {pid} must still be alive before the next start"
    );

    let next = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    report_of(&run_exec(state.path(), &next, None)?)?;
    await_group_exit(pid)?;
    let rows = ledger_rows(&state.path().join("executor.db"))?;
    assert_eq!(
        rows, 0,
        "the live_groups table must be empty after the reap"
    );
    Ok(())
}

/// The files under `<state>/journals` with extension `ext`, sorted.
fn journals(state: &Path, ext: &str) -> Result<Vec<std::path::PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    let listing = match fs::read_dir(state.join("journals")) {
        Ok(listing) => listing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error.into()),
    };
    for entry in listing {
        let path = entry?.path();
        if path.extension() == Some(OsStr::new(ext)) {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn a_kill_9_mid_task_leaves_a_journal_the_next_start_restores_without_rerunning() -> TestResult {
    // larry#75: SIGKILL inside the task body, then a torn append on top of the
    // file. The next start takes the torn tail back, restores the dead run,
    // settles it as unknown and keeps the file; the body never runs again.
    // Source: write-ahead-log recovery (replay, truncate the torn tail) and the
    // RequiresHuman rule that an ambiguous reach is never retried blind.
    use larry_contracts::state::RunState;
    use larry_runtime::journal::Journal;
    use larry_runtime::projection::{FoldProjection, ProjectionBackend};
    use larry_runtime::runtime::replay;

    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let hold = scratch.path().join("hold");
    let marker = scratch.path().join("ran");
    make_fifo(&hold)?;
    let acceptance = format!(
        r#"[["sh","-c","echo ran >> {}; cat < {}"]]"#,
        marker.display(),
        hold.display()
    );
    let task = task_json(origin.path(), "HEAD", &acceptance);
    let mut child = spawn_exec(Some(state.path()), None, &task)?;
    // The command opens the fifo's read end only after it writes the marker, and the write end
    // opens only once that read end is open: the marker is on disk when this returns (larry#268).
    let _hold = await_fifo_writer(&hold)?;
    child.kill()?;
    child.wait()?;

    let left = journals(state.path(), "journal")?;
    if left.len() != 1 {
        return Err(format!("the killed task leaves one journal, found {left:?}").into());
    }
    let journal = left.first().ok_or("one journal")?;
    let whole = fs::read(journal)?;
    assert!(!whole.is_empty(), "the killed task's facts are on disk");
    let torn = whole.get(..7).ok_or("the journal holds a frame")?;
    fs::OpenOptions::new()
        .append(true)
        .open(journal)?
        .write_all(torn)?;

    let next = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    report_of(&run_exec(state.path(), &next, None)?)?;
    assert_eq!(
        journals(state.path(), "journal")?,
        Vec::<std::path::PathBuf>::new(),
        "no journal is left running: the dead one is recovered, the new one removed"
    );
    let recovered = journal.with_extension("recovered");
    assert_eq!(
        journals(state.path(), "recovered")?,
        vec![recovered.clone()],
        "the dead task's journal is kept as recovered"
    );
    assert_eq!(
        fs::read_to_string(&marker)?,
        "ran\n",
        "the recovered task's body never runs again"
    );

    // Opening it again replays cleanly: the recovery took the torn tail back.
    let kept = Journal::open(&recovered, 4096)?;
    let mut folded = FoldProjection::default();
    replay(&mut folded, &kept)?;
    let runs = folded.run_ids();
    assert_eq!(runs.len(), 1, "the dead task's one run is restored");
    for id in runs {
        assert_eq!(
            folded.run_state(id),
            Some(RunState::OutcomeUnknown),
            "the dead run settles as unknown, never as done or retried"
        );
    }
    Ok(())
}

#[test]
fn missing_state_dir_non_repo_and_bad_base_each_fail_typed() -> TestResult {
    // Three refusals, each one typed line on stderr and exit 1; nothing is sealed.
    // A model-command task is a denial, sealed and never accepted: tests/receipt_journey.rs.
    let (state, origin) = fixture()?;
    let empty = TempDir::new()?;
    let quick = r#"[["true"]]"#;
    let valid = task_json(origin.path(), "HEAD", quick);

    let output = spawn_exec(None, None, &valid)?.wait_with_output()?;
    assert_refused(&output, "LARRY_STATE_DIR");

    let output = run_exec(state.path(), &task_json(empty.path(), "HEAD", quick), None)?;
    assert_refused(&output, "not in the tenant allowlist");

    let output = run_exec(
        state.path(),
        &task_json(origin.path(), "no-such-ref", quick),
        None,
    )?;
    assert_refused(&output, "could not check out base no-such-ref");
    Ok(())
}

#[test]
fn a_noisy_command_is_truncated_not_blocked() -> TestResult {
    // Three mebibytes of output are cut to the cap; the command still completes.
    //
    // `dd count=N` counts blocks read, not bytes: a read that returns fewer than `bs` bytes is
    // one whole block, so `yes x | dd bs=1024 count=3072` writes fewer than 3 MiB whenever the
    // shell's own pipe hands it a partial block, and the exact count below then missed by that
    // much (larry#319: 650 bytes short under load; a 10 KiB target fed in small writes measured
    // 212 to 1322 bytes on both hosts). `/dev/zero` always fills a read, so this command writes
    // 3,145,728 bytes on every host.
    let (state, origin) = fixture()?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","dd if=/dev/zero bs=1024 count=3072 2>/dev/null"]]"#,
    );
    let report = report_of(&run_exec(state.path(), &task, None)?)?;
    let result = result_at(&report, 0)?;
    assert!(
        flag_of(result, "truncated")?,
        "output past the cap must be flagged truncated"
    );
    // The head and the tail together keep the cap; a marker line says what fell between them
    // (larry#206). 3 MiB less the cap is dropped, exactly.
    let kept = text_of(result, "stdout")?.len();
    assert!(
        (STDOUT_CAP..=STDOUT_CAP.saturating_add(64)).contains(&kept),
        "stdout must keep the cap plus one marker line, got {kept}"
    );
    assert_eq!(
        number_of(result, "stdout_dropped")?,
        2_097_152,
        "the dropped count is exact: 3145728 bytes less the 1048576 kept"
    );
    // The report states the bytes the stream yielded, so kept plus dropped is checked against
    // the stream's own length and no byte can be silently unaccounted (larry#319).
    let yielded = number_of(result, "stdout_bytes")?;
    assert_eq!(
        yielded, 3_145_728,
        "the command wrote 3145728 bytes and the report says how many it read"
    );
    let marker = format!(
        "\n[larry: {} bytes dropped]\n",
        number_of(result, "stdout_dropped")?
    );
    assert_eq!(
        number_of(result, "stdout_dropped")? + i64::try_from(kept.saturating_sub(marker.len()))?,
        yielded,
        "kept plus dropped is every byte the command wrote"
    );
    assert_eq!(
        number_of(result, "exit_code")?,
        0,
        "the noisy command must still exit 0"
    );
    Ok(())
}

#[test]
fn two_tasks_at_once_never_share_a_workspace() -> TestResult {
    // Two executors start together on one origin and one state dir.
    let (state, origin) = fixture()?;
    let before = tree_digest(origin.path())?;
    let task = task_json(origin.path(), "HEAD", r#"[["pwd"]]"#);
    let first_child = spawn_exec(Some(state.path()), None, &task)?;
    let second_child = spawn_exec(Some(state.path()), None, &task)?;
    let first = report_of(&first_child.wait_with_output()?)?;
    let second = report_of(&second_child.wait_with_output()?)?;
    let first_pwd = text_of(result_at(&first, 0)?, "stdout")?.trim_end();
    let second_pwd = text_of(result_at(&second, 0)?, "stdout")?.trim_end();
    assert!(
        !first_pwd.is_empty(),
        "the first pwd must report a path: {}",
        result_at(&first, 0)?
    );
    assert_ne!(
        first_pwd, second_pwd,
        "two concurrent tasks must run in different workspaces"
    );
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "two concurrent tasks must leave the origin byte-identical"
    );
    Ok(())
}

#[test]
fn a_bad_model_timeout_does_not_fail_a_task_with_no_brief() -> TestResult {
    // larry#213: model settings are parsed only when a task reaches a model, so a typo in
    // one of them fails nothing a task with no brief uses.
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let env: [(&str, &OsStr); 5] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        ("LARRY_MODEL_URL", OsStr::new("http://127.0.0.1:9/v1")),
        ("LARRY_MODEL", OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", OsStr::new("test-key")),
        ("LARRY_MODEL_TIMEOUT_MS", OsStr::new("abc")),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    let report = report_of(&output)?;
    assert!(
        flag_of(&report, "accepted")?,
        "a task with no brief never reads the model's settings"
    );
    Ok(())
}

#[test]
fn exec_refuses_an_unset_path_typed() -> TestResult {
    // larry#213: an unset PATH is a typed refusal, never the empty string every command
    // would then be looked up under. The child gets no PATH at all. The task is the child's
    // stdin as a file, never a pipe: `larry exec` refuses before it reads, and a pipe write
    // would race that exit into a BrokenPipe (larry#448).
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let task_file = state.path().join("task.json");
    std::fs::write(&task_file, task.as_bytes())?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg("exec")
        .env_clear()
        .env("LARRY_STATE_DIR", state.path())
        .stdin(std::fs::File::open(&task_file)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    coverage::forward(&mut command);
    let output = command.output()?;
    assert_refused(&output, "PATH is unset");
    assert!(output.stdout.is_empty(), "a refused run seals no report");
    Ok(())
}
