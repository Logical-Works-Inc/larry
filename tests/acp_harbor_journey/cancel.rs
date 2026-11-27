//! A person's cancel through `larry acp` (larry#242, larry#458), split from
//! `tests/acp_harbor_journey.rs` so each file stays one responsibility: the
//! cancel kills the running command's process group, and no command runs after
//! it.

use crate::TestResult;
use crate::acp_agent::{kinds, prompt};
use crate::acp_session::{SESSION_POLICY, agent, scripted};
use crate::json_at::At;
use crate::mock_model;
use crate::origin::fixture;
use lgwks_std::json::json;
use std::fs;
use std::io::Write;
use std::process::Command;

#[test]
fn a_cancel_stops_the_session_before_its_next_command() -> TestResult {
    let (state, work) = fixture()?;
    let fifo = work.path().join("gate.fifo");
    assert!(
        Command::new("mkfifo").arg(&fifo).status()?.success(),
        "mkfifo"
    );
    let replies = [
        r#"{"command":"cat gate.fifo"}"#,
        r#"{"command":"touch after.txt"}"#,
    ];
    mock_model::run(scripted(17, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
        let session = acp.open(work.path())?;
        acp.send(&prompt(2, &session, "wait"))?;
        let started = acp.recv()?;
        assert_eq!(
            *started.at("/params/update/sessionUpdate")?,
            "tool_call",
            "running"
        );
        acp.send(&json!({"jsonrpc":"2.0","method":"session/cancel",
            "params":{"sessionId":session}}))?;
        acp.logged("cancel requested")?;
        // The write lets cat end. The cancel can kill cat before cat opens the fifo, and a
        // write-only open then waits for a reader for good (larry#458): opened read-write, the
        // fifo never waits, on Linux or on macOS, whichever came first.
        let mut gate = fs::OpenOptions::new().read(true).write(true).open(&fifo)?;
        gate.write_all(b"go\n")?;
        let (notes, reply) = acp.until(&json!(2))?;
        drop(gate);
        assert_eq!(*reply.at("/result/stopReason")?, "cancelled", "{reply}");
        assert_eq!(
            kinds(&notes),
            ["tool_call_update"],
            "the running command ends"
        );
        assert_eq!(mock.served(), 1, "no model call after the cancel");
        assert!(
            !work.path().join("after.txt").exists(),
            "no command after the cancel"
        );
        Ok(())
    })
}
