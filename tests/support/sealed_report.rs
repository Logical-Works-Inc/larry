//! Readers of the sealed JSON report a finished `larry exec` prints (larry#86, larry#164),
//! split from `larry_cli.rs` by responsibility (larry#341): that file starts the child, this
//! one reads what it sealed. Included beside `larry_cli` by `tests/support/exec_cli.rs` and
//! `tests/sim_exec_gate.rs`.

use std::error::Error;
use std::process::Output;

use lgwks_std::json::{self, Value};

use super::larry_cli::stderr_of;

/// Parses a sealed run's stdout as the JSON report: exit 0 for an accepted run,
/// 2 for a rejected one, and the exit status always agrees with the sealed verdict.
pub(crate) fn report_of(output: &Output) -> Result<Value, Box<dyn Error>> {
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
pub(crate) fn flag_of(value: &Value, key: &str) -> Result<bool, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("report member {key} is not a boolean").into())
}

/// The report's exact `denied` lines, as text (larry#86, larry#164).
pub(crate) fn denial_lines(report: &Value) -> Result<Vec<String>, Box<dyn Error>> {
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
