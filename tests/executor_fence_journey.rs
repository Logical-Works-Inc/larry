//! Binary journeys for the executor host fence controls (larry#215, ADR 0121).

use std::ffi::OsStr;

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/larry_guard.rs"]
mod larry_guard;
#[path = "support/origin.rs"]
mod origin;
#[path = "support/sealed_report.rs"]
mod sealed_report;

use larry_cli::spawn_larry_args;
use origin::fixture;
use sealed_report::{denial_lines, report_of};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn run_with_control(control: &str) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let (state, origin) = fixture()?;
    let task = format!(
        r#"{{"repo":"{}","base":"HEAD","acceptance":[["true"]]}}"#,
        origin.path().display()
    );
    let env = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        ("LARRY_FENCE_CONTROL", OsStr::new(control)),
    ];
    Ok(spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?)
}

#[test]
fn executor_withdraws_the_current_operation_before_dispatch() -> TestResult {
    let output = run_with_control("revoke")?;
    let report = report_of(&output)?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        denial_lines(&report)?
            .iter()
            .any(|line| line.contains("revoked by name")),
        "the sealed report must name the withdrawal: {report}"
    );
    Ok(())
}

#[test]
fn executor_epoch_advance_fences_the_admitted_operation() -> TestResult {
    let output = run_with_control("advance")?;
    let report = report_of(&output)?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        denial_lines(&report)?.iter().any(|line| {
            line.contains("security epoch")
                || line.contains("SecurityEpoch")
                || line.contains("stale authority")
        }),
        "the sealed report must name the stale epoch: {report}"
    );
    Ok(())
}

#[test]
fn executor_reauthenticates_at_the_reported_epoch_before_dispatch() -> TestResult {
    let output = run_with_control("authenticate:0")?;
    let report = report_of(&output)?;
    assert_eq!(output.status.code(), Some(0));
    assert!(
        report
            .get("accepted")
            .and_then(lgwks_std::json::Value::as_bool)
            .ok_or("accepted flag missing")?,
        "re-authentication must preserve the admitted owner reach: {report}"
    );
    Ok(())
}
