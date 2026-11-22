//! The confinement journey of a task submitted over MCP (larry#324, ADR 0088), split from
//! `tests/mcp_stdio_journey.rs` so each file stays one responsibility. Source: OWASP ASVS
//! 4.0.3 §4.2.1 (enforcement at the action's boundary), CWE-59, forge#158 and larry#243.
//! Requirement proved: `larry_submit_task` has no person watching a command, so its acceptance
//! commands run behind the same kernel confinement a terminal session uses. A write outside the
//! workspace is refused, the origin stays byte-identical and the tool result is a
//! `task_rejected`, never a success.

use crate::json_at::At;
use crate::mcp_support::TestResult;
use crate::mcp_task::{briefed, connected, operator, submit};
use crate::mock_model::{Config, Handle};
use crate::tree_digest::tree_digest;
use lgwks_std::json::Value;

/// larry#324, forge#158: a submitted task's acceptance command runs behind the kernel
/// confinement a terminal session uses (larry#243), because a brief let the model write the
/// tree and no person watches the command. A write outside the tree is refused, the origin
/// stays byte-identical and the run is a `task_rejected`, never a success.
#[test]
fn a_submitted_briefs_acceptance_cannot_write_outside_its_tree() -> TestResult<()> {
    let (state, origin) = crate::origin::fixture()?;
    let outside = crate::origin::TempDir::new()?;
    let target = outside.path().join("escaped");
    let before = tree_digest(origin.path())?;
    let command = format!("printf pwned > {}", target.display());
    let task = briefed(origin.path(), &["README"], &[command.as_str()]);
    let result =
        crate::mock_model::run(Config::seeded(23), |handle: &Handle| -> TestResult<Value> {
            let mut session = connected(&operator(state.path(), handle.addr(), &[]))?;
            let result = submit(&mut session, &task)?;
            assert_eq!(session.finish()?.code(), Some(0), "clean exit");
            Ok(result)
        })?;
    let (code, _) = super::failure_of(&result)?;
    assert_eq!(
        code, "task_rejected",
        "a refused acceptance is a rejected run"
    );
    let report = result.at("/structuredContent/report")?;
    assert_eq!(*report.at("/accepted")?, false, "sealed as rejected");
    assert_ne!(
        *report.at("/results/0/exit_code")?,
        0,
        "the kernel refused the write, so the command did not exit 0"
    );
    assert!(
        !target.exists(),
        "nothing may write outside the tree, whatever the model chose"
    );
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "the origin stays byte-identical"
    );
    assert_eq!(
        super::workspaces(state.path())?,
        0,
        "no workspace is left behind"
    );
    Ok(())
}
