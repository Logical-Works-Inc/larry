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

#[path = "support/exec_cli.rs"]
mod exec_cli;

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use exec_cli::{
    POLL_LIMIT, TempDir, TestResult, assert_refused, denial_lines, fixture, flag_of,
    live_group_pid, make_fifo, millis, number_of, percentile, report_of, result_at, run_exec,
    spawn_exec, spawn_larry, task_json, text_of,
};
use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};
use lgwks_std::json::{self, Value};

/// Runs the latency case samples.
const SAMPLES: usize = 32;

/// The stdout cap the executor applies to each command, in bytes.
const STDOUT_CAP: usize = 1_048_576;

/// Collects every file and symlink under `dir` as (path relative to `root`, bytes).
fn collect(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
        let kind = fs::symlink_metadata(&path)?.file_type();
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            out.push((relative, format!("link:{}", target.display()).into_bytes()));
        } else if kind.is_dir() {
            collect(root, &path, out)?;
        } else {
            out.push((relative, fs::read(&path)?));
        }
    }
    Ok(())
}

/// BLAKE3 over every path and byte under `dir`, in sorted path order.
fn tree_digest(dir: &Path) -> Result<String, Box<dyn Error>> {
    let mut entries = Vec::new();
    collect(dir, dir, &mut entries)?;
    entries.sort();
    let mut hasher = lgwks_std::hash::Hasher::new();
    for entry in &entries {
        hasher.update(entry.0.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(&entry.1);
    }
    Ok(hasher.finalize().to_hex())
}

/// Whether any member of process group `pgid` is still alive.
fn group_alive(pgid: i32) -> Result<bool, Box<dyn Error>> {
    Ok(lgwks_std::process::process_group_exists(pgid)?)
}

/// Polls until process group `pgid` is gone; killed members may linger briefly as zombies.
fn wait_for_group_exit(pgid: i32) -> Result<bool, Box<dyn Error>> {
    let mut attempts: u32 = 0;
    while group_alive(pgid)? {
        attempts = attempts.saturating_add(1);
        if attempts >= POLL_LIMIT {
            return Ok(false);
        }
        std::thread::yield_now();
    }
    Ok(true)
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
    let ready = scratch.path().join("ready");
    let hold = scratch.path().join("hold");
    make_fifo(&ready)?;
    make_fifo(&hold)?;
    let acceptance = format!(
        r#"[["sh","-c","echo up > {}; cat < {}"]]"#,
        ready.display(),
        hold.display()
    );
    let task = task_json(origin.path(), "HEAD", &acceptance);
    let mut child = spawn_exec(Some(state.path()), None, &task)?;
    // This open blocks until the acceptance command opens the fifo, so the command is running.
    let _ready_open = fs::File::open(&ready)?;
    let db = state.path().join("executor.db");
    let pid = live_group_pid(&db)?;
    child.kill()?;
    child.wait()?;
    assert!(
        group_alive(pid)?,
        "the killed executor's group {pid} must still be alive before the next start"
    );

    let next = task_json(origin.path(), "HEAD", "[]");
    report_of(&run_exec(state.path(), &next, None)?)?;
    assert!(
        wait_for_group_exit(pid)?,
        "the next start must reap process group {pid}"
    );
    let connection = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let rows: i64 =
        connection.query_row("SELECT COUNT(*) FROM live_groups", [], |row| row.get(0))?;
    assert_eq!(
        rows, 0,
        "the live_groups table must be empty after the reap"
    );
    Ok(())
}

#[test]
fn missing_state_dir_non_repo_and_bad_base_each_fail_typed() -> TestResult {
    // Three refusals, each one typed line on stderr and exit 1; nothing is sealed.
    // A model-command task is a denial, sealed and never accepted: tests/receipt_journey.rs.
    let (state, origin) = fixture()?;
    let empty = TempDir::new()?;
    let valid = task_json(origin.path(), "HEAD", "[]");

    let output = spawn_exec(None, None, &valid)?.wait_with_output()?;
    assert_refused(&output, "LARRY_STATE_DIR");

    let output = run_exec(state.path(), &task_json(empty.path(), "HEAD", "[]"), None)?;
    assert_refused(&output, "could not clone");

    let output = run_exec(
        state.path(),
        &task_json(origin.path(), "no-such-ref", "[]"),
        None,
    )?;
    assert_refused(&output, "could not check out base no-such-ref");
    Ok(())
}

#[test]
fn a_noisy_command_is_truncated_not_blocked() -> TestResult {
    // Three mebibytes of output are cut to the cap; the command still completes.
    let (state, origin) = fixture()?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","yes x | dd bs=1024 count=3072 2>/dev/null"]]"#,
    );
    let report = report_of(&run_exec(state.path(), &task, None)?)?;
    let result = result_at(&report, 0)?;
    assert!(
        flag_of(result, "truncated")?,
        "output past the cap must be flagged truncated"
    );
    assert_eq!(
        text_of(result, "stdout")?.len(),
        STDOUT_CAP,
        "stdout must be cut to exactly the cap"
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
    assert!(!first_pwd.is_empty(), "the first pwd must report a path");
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

// ── the admission gate (larry#164) ───────────────────────────────────────────

/// A task document with a brief: `goal` and `files` the model writes before
/// the acceptance commands run.
fn brief_json(repo: &Path, goal: &str, files: &str, acceptance: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"HEAD","goal":"{goal}","files":{files},"acceptance":{acceptance}}}"#,
        repo.display()
    )
}

/// Whether any entry under `root`, at any depth, is named `name`.
fn has_entry_named(root: &Path, name: &str) -> Result<bool, Box<dyn Error>> {
    let mut found = false;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            found = found || path.ends_with(name);
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(found)
}

/// The verdict `larry verify` printed for `report` bytes, trusting `state`'s key.
fn verified_verdict(state: &Path, report: &[u8]) -> Result<Value, Box<dyn Error>> {
    let output = spawn_larry("verify", &[("LARRY_STATE_DIR", state.as_os_str())], report)?
        .wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "a genuine denied receipt verifies as rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(json::from_str::<Value>(&String::from_utf8_lossy(
        &output.stdout,
    ))?)
}

#[test]
fn exec_denies_a_brief_task_when_the_policy_lacks_file_write() -> TestResult {
    // Source: OWASP ASVS 4.0.3 §4.1.3 (deny by default) and §4.1.1 (the
    // privilege check happens where the action is served). The policy admits
    // read, spawn and model invoke but not file write, so a brief that needs a
    // file written is denied: nothing runs, no connection reaches the model
    // endpoint, and the sealed report is genuine but rejected (larry#164).
    let (state, origin) = fixture()?;
    // The endpoint the brief's model would call is bound but never answered.
    // A connection attempt completes the TCP handshake into this listener's
    // backlog, so one accepted connection is proof a request was attempted;
    // none may be.
    let endpoint = TcpListener::bind(("127.0.0.1", 0))?;
    endpoint.set_nonblocking(true)?;
    let url = format!("http://127.0.0.1:{}/v1", endpoint.local_addr()?.port());
    let task = brief_json(
        origin.path(),
        "write the note",
        r#"["note.txt"]"#,
        r#"[["true"]]"#,
    );
    let env: [(&str, &OsStr); 4] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        (
            "LARRY_POLICY",
            OsStr::new("file.read,process.spawn,model.invoke"),
        ),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_URL", OsStr::new(url.as_str())),
    ];
    let output = spawn_larry("exec", &env, task.as_bytes())?.wait_with_output()?;
    assert!(
        endpoint.accept().is_err(),
        "the gate denies before the model step, so no connection may reach the endpoint"
    );
    assert_eq!(output.status.code(), Some(2), "a gate denial exits 2");
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "a gate denial is never accepted"
    );
    assert_eq!(
        denial_lines(&report)?,
        vec![String::from("gate: policy_refuses: file.write")],
        "the denial names the first scope the policy refuses"
    );
    assert_eq!(
        report.get("diff").map(Value::is_null),
        Some(true),
        "a denied run seals no diff"
    );
    assert!(
        result_at(&report, 0).is_err(),
        "no command result exists after the denial"
    );
    let verdict = verified_verdict(state.path(), &output.stdout)?;
    assert!(flag_of(&verdict, "verified")?, "the receipt is genuine");
    assert!(
        !flag_of(&verdict, "accepted")?,
        "the verdict says not accepted"
    );
    assert_eq!(
        text_of(&verdict, "task_id")?,
        text_of(&report, "task_id")?,
        "the verdict names the denied task"
    );
    Ok(())
}

#[test]
fn exec_denies_every_command_when_the_policy_is_empty() -> TestResult {
    // Source: OWASP ASVS 4.0.3 §4.1.3 (deny by default). An empty policy
    // admits nothing, so the first required scope in Scope order (file.read)
    // is named, the acceptance command that would create the marker never
    // runs, and neither the state directory nor the origin changes (larry#164).
    let (state, origin) = fixture()?;
    let before = tree_digest(origin.path())?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","echo pwned > marker"]]"#,
    );
    let env: [(&str, &OsStr); 2] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        ("LARRY_POLICY", OsStr::new("")),
    ];
    let output = spawn_larry("exec", &env, task.as_bytes())?.wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "an empty policy denies with a sealed report"
    );
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "an empty policy accepts nothing"
    );
    assert_eq!(
        denial_lines(&report)?,
        vec![String::from("gate: policy_refuses: file.read")],
        "the first required scope in Scope order is named"
    );
    assert!(
        !has_entry_named(state.path(), "marker")?,
        "no marker file may exist anywhere under the state directory"
    );
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "an empty policy must leave the origin byte-identical"
    );
    Ok(())
}

#[test]
fn exec_refuses_an_unknown_policy_name() -> TestResult {
    // Source: OWASP ASVS 4.0.3 §4.1.1. A policy name the executor does not
    // know is refused before anything is read or run: one typed line naming
    // it, exit 1, nothing sealed (larry#164).
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", "[]");
    let env: [(&str, &OsStr); 2] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        ("LARRY_POLICY", OsStr::new("file.read,root")),
    ];
    let output = spawn_larry("exec", &env, task.as_bytes())?.wait_with_output()?;
    assert_refused(&output, "root");
    Ok(())
}
