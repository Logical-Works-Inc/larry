//! The shipped `larry` binary as a child process: started with a cleared
//! environment, fed on stdin, and its sealed JSON report read back. Shared by
//! `tests/support/exec_cli.rs` (re-exported to the executor and receipt
//! journeys) and `tests/sim_exec_gate.rs` (larry#164), so the child is started
//! and read one way.

use std::error::Error;
use std::ffi::OsStr;
use std::io::Write;
use std::process::{Child, Command, Output, Stdio};

use lgwks_std::json::{self, Value};

/// Starts `larry <subcommand>` with only PATH and `env` set, writes `input` to its
/// stdin and closes it.
pub fn spawn_larry(
    subcommand: &str,
    env: &[(&str, &OsStr)],
    input: &[u8],
) -> Result<Child, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg(subcommand)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A refused input may exit before it reads stdin; the broken pipe is the refusal itself.
        stdin.write_all(input).ok();
    }
    Ok(child)
}

/// The child's stderr as text.
pub fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Parses a sealed run's stdout as the JSON report: exit 0 for an accepted run,
/// 2 for a rejected one, and the exit status always agrees with the sealed verdict.
pub fn report_of(output: &Output) -> Result<Value, Box<dyn Error>> {
    let code = output.status.code();
    assert!(
        matches!(code, Some(0 | 2)),
        "larry exec must print a sealed report (exit 0 or 2), got {code:?}; stderr: {}",
        stderr_of(output)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let report = json::from_str::<Value>(&text)?;
    assert_eq!(
        flag_of(&report, "accepted")?,
        code == Some(0),
        "the exit status agrees with the sealed verdict"
    );
    Ok(report)
}

/// The member `key` of `value` as a boolean.
pub fn flag_of(value: &Value, key: &str) -> Result<bool, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("report member {key} is not a boolean").into())
}

/// The report's exact `denied` lines, as text (larry#86, larry#164).
pub fn denial_lines(report: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    let lines = report
        .get("denied")
        .and_then(Value::as_array)
        .ok_or("the report lists its denials")?;
    let mut denials = Vec::with_capacity(lines.len());
    for line in lines {
        denials.push(String::from(line.as_str().ok_or("a denial line is text")?));
    }
    Ok(denials)
}
