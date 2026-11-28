//! A `larry eval` arm that stops partway while nothing reads its standard output (larry#87,
//! larry#399, larry#612). Source: POSIX `EPIPE` on a pipe whose reader has gone. Requirement:
//! the run exits 1, standard error keeps both causes, why the arm stopped and why the card was
//! not printed, and no scored origin is left behind.

use std::io::Write as _;

use crate::eval_arm::{CLAUDE, TestResult, eval_entries, operator, spawn_arm, task};
use crate::mock_model::{Config, Handle};
use crate::temp_dir::TempDir;
use lgwks_std::json::json;

/// disconnect: the reader of standard output is gone before the arm starts on its set, and
/// the third task fails outside the arm. Catches a scorer that swallows either cause, or
/// exits 0 because the card had nowhere to go.
#[test]
fn a_stopped_arm_whose_card_cannot_be_printed_keeps_both_causes() -> TestResult {
    let broken = json!({
        "id": "broken",
        "files": {"README.md": "# broken\n", "out": "a file where a folder must be\n"},
        "goal": "write broken",
        "write": ["out/0.txt"],
        "acceptance": ["test -s out/0.txt"],
    });
    let tasks = [task("one", 1, &[]), broken, task("never", 1, &[])];
    let set = json!({"name": "unprinted", "tasks": tasks}).to_string();
    let state = TempDir::new()?;
    let output = crate::mock_model::run(Config::seeded(61), |handle: &Handle| {
        let env = operator(&state, handle.addr(), &CLAUDE, "1");
        let mut child = spawn_arm(&env, false)?;
        drop(child.stdout.take().ok_or("child stdout was not piped")?);
        let mut stdin = child.stdin.take().ok_or("child stdin was not piped")?;
        stdin.write_all(set.as_bytes())?;
        drop(stdin);
        let output = child.wait_with_output()?;
        assert_eq!(handle.served(), 1, "the one scored task's call, none after");
        Ok::<_, Box<dyn std::error::Error>>(output)
    })?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    let Some((stopped, unprinted)) = stderr.split_once("; the card was not printed: ") else {
        return Err(format!("both causes in one line: {stderr}").into());
    };
    assert!(
        stopped.contains("broken") && stopped.contains("out/0.txt"),
        "why the arm stopped: {stderr}"
    );
    assert_eq!(
        unprinted.trim_end(),
        "exec output unwritable: Broken pipe (os error 32)",
        "why the card was not printed: {stderr}"
    );
    assert_eq!(eval_entries(&state)?, 0, "no origin or lock stays");
    Ok(())
}
