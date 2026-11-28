//! What a terminal session may send to a model, through the shipped `larry acp`
//! binary (larry#203). Source: OWASP ASVS 4.0.3 §4.1.3 (deny by default) and
//! §8.3.4 (sensitive data is identified before it leaves). Requirement proved:
//! a session reaches no model under a policy without `net.egress`, and neither
//! the route key nor the absolute working directory is in any request it does
//! send. Included by `tests/acp_harbor_journey.rs`.

use std::ffi::{OsStr, OsString};

use super::acp_agent::{self, prompt, said};
use super::acp_session::{SESSION_POLICY, agent, operator, scripted};
use super::json_at::At;
use super::mock_model;
use super::origin::fixture;
use super::{TestResult, verify};

/// Every scope a host session needs but `net.egress`: the session is refused
/// naming it, nothing reaches the model and nothing runs.
#[test]
fn a_session_without_net_egress_is_refused_before_the_model() -> TestResult {
    let (state, work) = fixture()?;
    let policy = "file.read,file.write,process.spawn,model.invoke,model.command";
    let replies = [r#"{"command":"touch ran.txt"}"#];
    mock_model::run(scripted(67, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, Some(policy))?;
        let session = acp.open(work.path())?;
        let (notes, reply) = acp.call(&prompt(2, &session, "touch ran.txt"))?;
        assert_eq!(*reply.at("/result/stopReason")?, "refusal", "{reply}");
        let line = "gate: policy_refuses: net.egress";
        assert_eq!(said(&notes), Some(line), "the refusal names the scope");
        assert_eq!(mock.served(), 0, "no model call");
        assert!(!work.path().join("ran.txt").exists(), "nothing ran");
        let report = reply.at("/result/_meta/larry/report")?;
        assert_eq!(*report.at("/denied/0")?, line, "the denial is sealed");
        assert_eq!(
            verify(state.path(), report)?.0,
            Some(2),
            "its seal verifies"
        );
        Ok(())
    })
}

/// A command prints the route key and the working directory: neither is in any
/// request the model receives.
#[test]
fn a_terminal_command_redacts_route_key_and_workspace_path_in_model_requests() -> TestResult {
    let (state, work) = fixture()?;
    let secret = "acp-route-secret";
    let command = format!(r#"{{"command":"printf '{secret}'; pwd"}}"#);
    let replies = [command.as_str(), r#"{"done":"printed"}"#];
    mock_model::run(scripted(61, &replies), |mock| -> TestResult {
        let mut env = operator(state.path(), mock, Some(SESSION_POLICY));
        for entry in env.iter_mut() {
            if entry.0 == "LARRY_MODEL_KEY" {
                entry.1 = OsString::from(secret);
            }
        }
        let pairs: Vec<(&str, &OsStr)> = env
            .iter()
            .map(|pair| (pair.0, pair.1.as_os_str()))
            .collect();
        let mut acp = acp_agent::Agent::start(&pairs)?;
        let session = acp.open(work.path())?;
        let (_, reply) = acp.call(&prompt(2, &session, "print the route key"))?;
        assert_eq!(*reply.at("/result/stopReason")?, "end_turn", "{reply}");
        let workspace = work.path().to_string_lossy();
        for body in mock.requests() {
            let shown = String::from_utf8_lossy(&body);
            assert!(
                !shown.contains(secret),
                "the route key reached the model: {shown}"
            );
            assert!(
                !shown.contains(workspace.as_ref()),
                "the absolute workspace reached the model: {shown}"
            );
        }
        assert_eq!(mock.served(), 2, "the command and done replies both ran");
        assert_eq!(acp.finish()?, Some(0), "clean ACP exit");
        Ok(())
    })
}
