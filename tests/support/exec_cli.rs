//! Fixtures for the journeys that drive the shipped `larry exec`, `larry verify`
//! and `larry key` commands (larry#85, larry#86): a temp state directory, an
//! origin repository, the child process with a cleared environment, and the
//! readers of its sealed JSON report. Both `tests/executor_journey.rs` and
//! `tests/receipt_journey.rs` call every helper here. The temp directory and
//! the origin repository live in `tests/support/origin.rs`, shared with the
//! MCP journey (larry#123).

use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::time::Duration;

use lgwks_deps::storefront::rusqlite::{Connection, OpenFlags};
use lgwks_std::json::Value;

/// What every test here returns.
pub type TestResult = Result<(), Box<dyn Error>>;

/// Polls a condition at most this many times before the journey gives up.
pub const POLL_LIMIT: u32 = 100_000;

#[path = "origin.rs"]
mod origin;
pub use origin::{TempDir, fixture};

#[path = "larry_cli.rs"]
mod larry_cli;
pub use larry_cli::{denial_lines, flag_of, report_of, spawn_larry, stderr_of};

/// Creates a named pipe at `path`.
pub fn make_fifo(path: &Path) -> Result<(), Box<dyn Error>> {
    let status = Command::new("mkfifo").arg(path).status()?;
    assert!(status.success(), "mkfifo {} must succeed", path.display());
    Ok(())
}

/// A task document naming `repo`, `base` and the acceptance commands as raw JSON.
pub fn task_json(repo: &Path, base: &str, acceptance: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"{}","acceptance":{}}}"#,
        repo.display(),
        base,
        acceptance
    )
}

/// Starts `larry exec` with a cleared environment, pipes `task` in and closes stdin.
/// `state` sets `LARRY_STATE_DIR` when given; `timeout_ms` sets the command timeout.
pub fn spawn_exec(
    state: Option<&Path>,
    timeout_ms: Option<u64>,
    task: &str,
) -> Result<Child, Box<dyn Error>> {
    let timeout = timeout_ms.map(|ms| ms.to_string());
    let mut env: Vec<(&str, &OsStr)> = Vec::new();
    if let Some(dir) = state {
        env.push(("LARRY_STATE_DIR", dir.as_os_str()));
    }
    if let Some(ref ms) = timeout {
        env.push(("LARRY_EXEC_TIMEOUT_MS", OsStr::new(ms)));
    }
    spawn_larry("exec", &env, task.as_bytes())
}

/// Runs `larry exec` to its end with `task` on stdin and returns its output.
pub fn run_exec(
    state: &Path,
    task: &str,
    timeout_ms: Option<u64>,
) -> Result<Output, Box<dyn Error>> {
    Ok(spawn_exec(Some(state), timeout_ms, task)?.wait_with_output()?)
}

/// The member `key` of `value` as text.
pub fn text_of<'a>(value: &'a Value, key: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("report member {key} is not text").into())
}

/// The member `key` of `value` as an integer.
pub fn number_of(value: &Value, key: &str) -> Result<i64, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("report member {key} is not an integer").into())
}

/// The `index`-th entry of the report's `results` array.
pub fn result_at(report: &Value, index: usize) -> Result<&Value, Box<dyn Error>> {
    report
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.get(index))
        .ok_or_else(|| format!("report has no results[{index}]").into())
}

/// Asserts a typed refusal: exit 1, exactly one stderr line, and `needle` named in it.
pub fn assert_refused(output: &Output, needle: &str) {
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
pub fn percentile(sorted: &[Duration], per_cent: usize) -> Duration {
    let rank = sorted.len().saturating_mul(per_cent).div_ceil(100);
    match sorted.get(rank.saturating_sub(1)) {
        Some(&sample) => sample,
        None => Duration::ZERO,
    }
}

/// A duration in fractional milliseconds.
pub fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Opens the executor's ledger read-only and polls it until a live group is recorded;
/// returns that group's pid.
pub fn live_group_pid(db: &Path) -> Result<i32, Box<dyn Error>> {
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
