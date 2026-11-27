//! The task ceilings of `larry exec` (larry#439, ADR 0111), split from
//! `tests/executor_journey.rs` so each file stays one responsibility. Source: POSIX `sh -c`
//! (the command string, then its positional parameters), POSIX process groups (a killed group
//! leaves no member) and OWASP ASVS 4.0.3 §1.4.1 (a limit is enforced at the trusted boundary,
//! before the action). Requirement proved: a task names its shell, base, per-command timeout and
//! whole-task deadline; each is bounded by the tenant's ceiling in the executor settings; a value
//! past a ceiling is one typed stderr line with nothing cloned; a task past its deadline ends
//! typed with its command's group gone.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

use lgwks_std::json::Value;

use crate::exec_cli::deadline::await_group_exit;
use crate::exec_cli::{
    TempDir, TestResult, assert_refused, fixture, flag_of, report_of, spawn_larry_args, stderr_of,
};
use crate::held_group::ledger_rows;

/// What a helper here returns.
type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

/// How many workspaces a state directory holds; none before its first clone.
fn workspaces(state: &Path) -> Outcome<usize> {
    match fs::read_dir(state.join("ws")) {
        Ok(entries) => Ok(entries.count()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

/// Runs `larry exec` over `state` with the tenant settings `extra`, `task` on stdin.
fn exec_with(state: &Path, extra: &[(&str, &str)], task: &str) -> Outcome<Output> {
    let mut env: Vec<(&str, &OsStr)> = vec![("LARRY_STATE_DIR", state.as_os_str())];
    env.extend(extra.iter().map(|&(name, value)| (name, OsStr::new(value))));
    Ok(spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?)
}

/// A task document over `origin` with the raw JSON members `members` appended.
fn task_with(origin: &Path, base: &str, acceptance: &str, members: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"{base}","acceptance":{acceptance}{members}}}"#,
        origin.display()
    )
}

/// The argv the report says ran at `index`.
fn argv_at(report: &Value, index: usize) -> Option<Vec<String>> {
    let argv = report
        .get("results")?
        .as_array()?
        .get(index)?
        .get("argv")?
        .as_array()?;
    argv.iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// Two tenants, two state directories with different ceilings, submit one task whose
/// command timeout is 5 s: the tenant whose ceiling is 1 s is refused with nothing cloned,
/// and the tenant whose ceiling is 60 s runs it and seals it accepted.
#[test]
fn two_tenants_with_different_ceilings_admit_one_and_refuse_one() -> TestResult {
    let (strict, origin) = fixture()?;
    let roomy = TempDir::new()?;
    let task = task_with(
        origin.path(),
        "HEAD",
        r#"[["true"]]"#,
        r#","timeout_ms":5000"#,
    );
    let refused = exec_with(
        strict.path(),
        &[("LARRY_EXEC_MAX_TIMEOUT_MS", "1000")],
        &task,
    )?;
    assert_refused(&refused, "timeout_ms");
    assert_eq!(
        workspaces(strict.path())?,
        0,
        "a refused task clones nothing"
    );
    let admitted = exec_with(
        roomy.path(),
        &[("LARRY_EXEC_MAX_TIMEOUT_MS", "60000")],
        &task,
    )?;
    report_of(&admitted)?;
    assert_eq!(
        admitted.status.code(),
        Some(0),
        "the roomy tenant runs the task, accepted"
    );
    Ok(())
}

/// A shell, a base or a deadline the tenant's ceiling refuses is one typed line naming it,
/// with nothing cloned; an allowed shell runs each command as `<shell> -c <command>`.
#[test]
fn a_shell_base_or_deadline_past_the_ceiling_clones_nothing() -> TestResult {
    let (state, origin) = fixture()?;
    let tenant = [
        ("LARRY_EXEC_SHELLS", "sh"),
        ("LARRY_EXEC_BASES", "HEAD,v*"),
        ("LARRY_EXEC_MAX_DEADLINE_MS", "60000"),
    ];
    let bash = task_with(
        origin.path(),
        "HEAD",
        r#"[["exit 0"]]"#,
        r#","shell":"bash""#,
    );
    assert_refused(&exec_with(state.path(), &tenant, &bash)?, "shell");
    let main = task_with(origin.path(), "main", r#"[["true"]]"#, "");
    assert_refused(&exec_with(state.path(), &tenant, &main)?, "base");
    let long = task_with(
        origin.path(),
        "HEAD",
        r#"[["true"]]"#,
        r#","deadline_ms":60001"#,
    );
    assert_refused(&exec_with(state.path(), &tenant, &long)?, "deadline_ms");
    assert_eq!(
        workspaces(state.path())?,
        0,
        "no refused task cloned anything"
    );
    let allowed = task_with(origin.path(), "HEAD", r#"[["exit 0"]]"#, r#","shell":"sh""#);
    let output = exec_with(state.path(), &tenant, &allowed)?;
    let report = report_of(&output)?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "an allowed shell and base run, accepted"
    );
    assert_eq!(
        argv_at(&report, 0),
        Some(vec![
            String::from("sh"),
            String::from("-c"),
            String::from("exit 0")
        ]),
        "the command ran as `sh -c <command>` and the report names that argv"
    );
    Ok(())
}

/// A task past its whole-task deadline seals a rejected report: the running command's group is
/// killed and gone, the cut command's result stays with `timed_out: true`, the sealed report
/// carries `deadline_passed` beside its results, `accepted` is false, exit 2, `larry verify`
/// re-derives the rejection, the ledger holds no row, and the command after it never starts.
#[test]
fn a_task_past_its_deadline_seals_rejected_with_its_group_reaped() -> TestResult {
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let pid_file = scratch.join("pid");
    let second = scratch.join("second");
    let script = format!("echo $$ > {}; exec sleep 30", pid_file.display());
    let acceptance = format!(
        r#"[["sh","-c","{script}"],["sh","-c","touch {}"]]"#,
        second.display()
    );
    let task = task_with(origin.path(), "HEAD", &acceptance, r#","deadline_ms":1500"#);
    let started = Instant::now();
    let output = exec_with(state.path(), &[], &task)?;
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(1500),
        "the run ends at its deadline, not before: {elapsed:?}"
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a spent deadline seals a rejected report, exit 2; stderr: {:?}",
        stderr_of(&output)
    );
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "a spent deadline is never accepted"
    );
    assert_eq!(
        report.get("deadline_passed"),
        Some(&lgwks_std::json::json!({"deadline_ms": 1500})),
        "the seal names the spent deadline beside its results"
    );
    assert_eq!(
        argv_at(&report, 0),
        Some(vec![String::from("sh"), String::from("-c"), script,]),
        "the cut command stays as the report's one result"
    );
    let results = report
        .get("results")
        .and_then(Value::as_array)
        .ok_or("the report lists its results")?;
    assert_eq!(
        results.len(),
        1,
        "the cut command stays and the second never starts"
    );
    let first = results.first().ok_or("the cut command stays")?;
    assert_eq!(
        first.get("timed_out").and_then(Value::as_bool),
        Some(true),
        "the cut command timed out"
    );
    assert_eq!(
        first.get("exit_code"),
        Some(&Value::Null),
        "the killed command names no exit code"
    );
    let state_env = [("LARRY_STATE_DIR", state.path().as_os_str())];
    let verified = spawn_larry_args(&["verify"], &state_env, &output.stdout)?.wait_with_output()?;
    assert_eq!(
        verified.status.code(),
        Some(2),
        "verify re-derives the rejection, exit 2; stderr: {:?}",
        stderr_of(&verified)
    );
    let verdict: Value = lgwks_std::json::from_str(&String::from_utf8_lossy(&verified.stdout))?;
    assert_eq!(
        verdict.get("accepted").and_then(Value::as_bool),
        Some(false),
        "verify reads the sealed report as rejected"
    );
    let pid: i32 = fs::read_to_string(&pid_file)?.trim().parse()?;
    await_group_exit(pid)?;
    assert_eq!(
        ledger_rows(&state.path().join("executor.db"))?,
        0,
        "the ledger holds no live group once the deadline reaped it"
    );
    assert!(
        !second.exists(),
        "the command after the deadline never started"
    );
    let mut err = io::stderr().lock();
    writeln!(
        err,
        "deadline 1500 ms, run ended after {} ms",
        elapsed.as_millis()
    )?;
    Ok(())
}
