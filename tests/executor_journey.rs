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

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};
use lgwks_std::json::{self, Value};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Runs the latency case samples.
const SAMPLES: usize = 32;

/// Polls a condition at most this many times before the journey gives up.
const POLL_LIMIT: u32 = 100_000;

/// The stdout cap the executor applies to each command, in bytes.
const STDOUT_CAP: usize = 1_048_576;

/// A fresh directory under the system temp dir, removed when dropped.
struct TempDir {
    /// Where the directory lives.
    path: PathBuf,
}

impl TempDir {
    /// Creates an empty directory whose name no other test in this process uses.
    fn new() -> io::Result<TempDir> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let run = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("larry-exec-{}-{run}", std::process::id()));
        fs::remove_dir_all(&path).ok();
        fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }

    /// The directory path.
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Cleanup is best effort: a leftover temp dir must never fail a journey.
        fs::remove_dir_all(&self.path).ok();
    }
}

/// A state dir for the executor and a fresh origin repository, in that order.
fn fixture() -> Result<(TempDir, TempDir), Box<dyn Error>> {
    let state = TempDir::new()?;
    let origin = TempDir::new()?;
    origin_repo(origin.path())?;
    Ok((state, origin))
}

/// Runs `git -C dir <args>` and fails with git's stderr when it exits non-zero.
fn git(dir: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {args:?} in {} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}

/// Makes `dir` a git repository with one commit holding `README` and `src/lib.rs`.
fn origin_repo(dir: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join("README"), "origin readme\n")?;
    fs::write(dir.join("src").join("lib.rs"), "pub fn origin() {}\n")?;
    git(dir, &["init", "-q"])?;
    git(dir, &["add", "-A"])?;
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    )?;
    Ok(())
}

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

/// Creates a named pipe at `path`.
fn make_fifo(path: &Path) -> Result<(), Box<dyn Error>> {
    let status = Command::new("mkfifo").arg(path).status()?;
    assert!(status.success(), "mkfifo {} must succeed", path.display());
    Ok(())
}

/// A task document naming `repo`, `base` and the acceptance commands as raw JSON.
fn task_json(repo: &Path, base: &str, acceptance: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"{}","acceptance":{}}}"#,
        repo.display(),
        base,
        acceptance
    )
}

/// Starts `larry exec` with a cleared environment, pipes `task` in and closes stdin.
/// `state` sets `LARRY_STATE_DIR` when given; `timeout_ms` sets the command timeout.
fn spawn_exec(
    state: Option<&Path>,
    timeout_ms: Option<u64>,
    task: &str,
) -> Result<Child, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg("exec")
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = state {
        command.env("LARRY_STATE_DIR", dir);
    }
    if let Some(ms) = timeout_ms {
        command.env("LARRY_EXEC_TIMEOUT_MS", ms.to_string());
    }
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A refused task may exit before it reads stdin; the broken pipe is the refusal itself.
        stdin.write_all(task.as_bytes()).ok();
    }
    Ok(child)
}

/// Runs `larry exec` to its end with `task` on stdin and returns its output.
fn run_exec(state: &Path, task: &str, timeout_ms: Option<u64>) -> Result<Output, Box<dyn Error>> {
    Ok(spawn_exec(Some(state), timeout_ms, task)?.wait_with_output()?)
}

/// The child's stderr as text.
fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Parses a successful run's stdout as the JSON report.
fn report_of(output: &Output) -> Result<Value, Box<dyn Error>> {
    assert!(
        output.status.success(),
        "larry exec must exit 0, got {:?}; stderr: {}",
        output.status.code(),
        stderr_of(output)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(json::from_str::<Value>(&text)?)
}

/// The member `key` of `value` as text.
fn text_of<'a>(value: &'a Value, key: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("report member {key} is not text").into())
}

/// The member `key` of `value` as a boolean.
fn flag_of(value: &Value, key: &str) -> Result<bool, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("report member {key} is not a boolean").into())
}

/// The member `key` of `value` as an integer.
fn number_of(value: &Value, key: &str) -> Result<i64, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("report member {key} is not an integer").into())
}

/// The `index`-th entry of the report's `results` array.
fn result_at(report: &Value, index: usize) -> Result<&Value, Box<dyn Error>> {
    report
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.get(index))
        .ok_or_else(|| format!("report has no results[{index}]").into())
}

/// Asserts a typed refusal: exit 1, exactly one stderr line, and `needle` named in it.
fn assert_refused(output: &Output, needle: &str) {
    let stderr = stderr_of(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a refusal must exit 1; stderr: {stderr:?}"
    );
    assert_eq!(
        stderr.lines().count(),
        1,
        "a refusal must print exactly one stderr line, got {stderr:?}"
    );
    assert!(
        stderr.contains(needle),
        "stderr {stderr:?} must name {needle}"
    );
}

/// The sample at percentile `per_cent` of `sorted`, nearest rank; zero when empty.
fn percentile(sorted: &[Duration], per_cent: usize) -> Duration {
    let rank = sorted.len().saturating_mul(per_cent).div_ceil(100);
    match sorted.get(rank.saturating_sub(1)) {
        Some(&sample) => sample,
        None => Duration::ZERO,
    }
}

/// A duration in fractional milliseconds.
fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Opens the executor's ledger read-only and polls it until a live group is recorded;
/// returns that group's pid.
fn live_group_pid(db: &Path) -> Result<i32, Box<dyn Error>> {
    let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut attempts: u32 = 0;
    loop {
        match connection.query_row("SELECT pid FROM live_groups", [], |row| {
            row.get::<_, i64>(0)
        }) {
            Ok(pid) => return Ok(i32::try_from(pid)?),
            Err(error) => {
                attempts = attempts.saturating_add(1);
                if attempts >= POLL_LIMIT {
                    return Err(error.into());
                }
                std::thread::yield_now();
            }
        }
    }
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
    let report = report_of(&run_exec(state.path(), &task, None)?)?;
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
    // Four refusals, each one typed line on stderr and exit 1.
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

    let output = run_exec(
        state.path(),
        &format!(
            r#"{{"repo":"{}","base":"HEAD","acceptance":[],"model_commands":[]}}"#,
            origin.path().display()
        ),
        None,
    )?;
    assert_refused(&output, "isolation");
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
