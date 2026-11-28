//! What the shipped `larry acp` binary lets out of the machine (larry#589): the
//! policy refusals that stop a session before its model, and the bytes a model
//! receives from a session that runs.
//!
//! Source: OWASP ASVS 4.0.3 §4.1.3 (least privilege) and §8.3.4 (sensitive data
//! is identified and kept from places it need not reach), and CWE-200. The mock
//! gateway records each request body, so every assertion is on the bytes a
//! provider would have received.

use std::ffi::OsString;
use std::fs;

use super::acp_agent::{prompt, said};
use super::acp_session::{SESSION_POLICY, agent, operator, scripted, start};
use super::json_at::At;
use super::mock_model;
use super::origin::{TempDir, fixture};
use super::{TestResult, verify};

/// The operator's route key for the redaction journey.
const KEY: &str = "zq7-route-key-0123456789abcdef";

/// A prompt under `policy` is a refusal that names `scope`: the model is never
/// called, nothing runs and the sealed denial verifies.
fn refused(seed: u64, policy: Option<&str>, scope: &str) -> TestResult {
    let (state, work) = fixture()?;
    let replies = [r#"{"command":"touch ran.txt"}"#];
    mock_model::run(scripted(seed, &replies), |mock| -> TestResult {
        let mut acp = agent(state.path(), mock, policy)?;
        let session = acp.open(work.path())?;
        let (notes, reply) = acp.call(&prompt(2, &session, "touch ran.txt"))?;
        assert_eq!(*reply.at("/result/stopReason")?, "refusal", "{reply}");
        let line = format!("gate: policy_refuses: {scope}");
        assert_eq!(
            said(&notes),
            Some(line.as_str()),
            "the refusal names the scope"
        );
        assert_eq!(mock.served(), 0, "no model call");
        assert!(!work.path().join("ran.txt").exists(), "nothing ran");
        let report = reply.at("/result/_meta/larry/report")?;
        assert_eq!(
            *report.at("/denied/0")?,
            line.as_str(),
            "the denial is sealed"
        );
        let sealed = verify(state.path(), report)?.0;
        assert_eq!(sealed, Some(2), "its seal verifies");
        Ok(())
    })
}

/// The default policy holds no scope on a remote door, so the first one a session
/// needs is named.
#[test]
fn the_default_policy_refuses_a_session_and_runs_nothing() -> TestResult {
    refused(7, None, "file.read")
}

/// A session sends what it reads to a model, so a policy without `net.egress`
/// refuses it before the first call.
#[test]
fn a_session_without_net_egress_is_refused_before_the_model() -> TestResult {
    let policy = "file.read,file.write,process.spawn,model.invoke,model.command";
    refused(67, Some(policy), "net.egress")
}

/// The route key, a token and the host's paths reach a session by its task, its
/// command output and its file reads, one key cut by the output bound among them:
/// no request holds any of them, and the other words still arrive.
#[test]
fn a_session_sends_no_route_key_and_no_host_path() -> TestResult {
    let (state, work) = fixture()?;
    let home = TempDir::new()?;
    // A token the folder holds that Larry was never told about.
    let token = format!("ghp_{}", "7".repeat(36));
    let held = format!(
        "key {KEY}\ntoken {token}\nhome {}/notes\nkept words\n",
        home.path().display()
    );
    fs::write(work.path().join("held.txt"), held)?;
    // The key starts six bytes before the 4,096-byte bound on observed text.
    let edge = format!("{}{KEY}", "e".repeat(4090));
    fs::write(work.path().join("edge.txt"), edge)?;
    let replies = [
        r#"{"command":"cat held.txt; pwd; pwd -P >&2"}"#,
        r#"{"command":"cat edge.txt"}"#,
        r#"{"read":"held.txt"}"#,
        r#"{"read":"edge.txt"}"#,
        r#"{"done":"printed"}"#,
    ];
    mock_model::run(scripted(61, &replies), |mock| -> TestResult {
        let mut env = operator(state.path(), mock, Some(SESSION_POLICY));
        env.retain(|pair| pair.0 != "LARRY_MODEL_KEY");
        env.push(("LARRY_MODEL_KEY", OsString::from(KEY)));
        env.push(("HOME", home.path().as_os_str().to_owned()));
        let mut acp = start(&env)?;
        let session = acp.open(work.path())?;
        let task = format!("show the files; the key is {KEY}");
        let (_, reply) = acp.call(&prompt(2, &session, &task))?;
        assert_eq!(*reply.at("/result/stopReason")?, "end_turn", "{reply}");
        assert_eq!(mock.served(), 5, "every step reached the model");
        let canonical = fs::canonicalize(work.path())?;
        let private = [
            // The head of the key: what a clip would leave of it.
            KEY.chars().take(6).collect(),
            token.clone(),
            work.path().display().to_string(),
            canonical.display().to_string(),
            state.path().display().to_string(),
            home.path().display().to_string(),
        ];
        let requests = mock.requests();
        for (turn, body) in requests.iter().enumerate() {
            let sent = String::from_utf8_lossy(body);
            for value in &private {
                assert!(!sent.contains(value.as_str()), "turn {turn} sent {value}");
            }
        }
        let last = String::from_utf8_lossy(requests.last().ok_or("a request")?).into_owned();
        assert!(last.contains("kept words"), "the output arrives: {last}");
        assert!(
            last.contains("~/notes"),
            "the home folder is named ~: {last}"
        );
        assert_eq!(acp.finish()?, Some(0), "end of input is a clean exit");
        Ok(())
    })
}
