//! The admission gate of `larry exec` (larry#164, larry#589), split from
//! `tests/executor_journey.rs` so each file stays one responsibility.
//!
//! Source: OWASP ASVS 4.0.3 §4.1.1 (the privilege check happens where the action
//! is served), §4.1.3 (deny by default) and §8.3.4 (sensitive data is identified
//! and kept from places it need not reach), and CWE-200. Requirement proved: a
//! task its policy does not cover is sealed as rejected with nothing run or sent,
//! an unknown policy name is refused typed, and a brief holding the model key is
//! never sent.

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::Output;

use lgwks_std::json::{self, Value};

use crate::exec_cli::{
    TestResult, assert_refused, denial_lines, fixture, flag_of, report_of, result_at,
    spawn_larry_args, task_json, text_of,
};
use crate::tree_digest::tree_digest;

/// The model key of the brief journeys.
const BRIEF_KEY: &str = "zq7-brief-key-0123456789abcdef";

/// Every scope a brief needs.
const BRIEF_POLICY: &str = "file.read,file.write,process.spawn,model.invoke,net.egress";

/// A task document with a brief: `goal` and `files` the model writes before
/// the acceptance commands run.
pub(crate) fn brief_json(repo: &Path, goal: &str, files: &str, acceptance: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"HEAD","goal":"{goal}","files":{files},"acceptance":{acceptance}}}"#,
        repo.display()
    )
}

/// Whether any entry under `root`, at any depth, is named `name`.
fn has_entry_named(root: &Path, name: &str) -> Result<bool, Box<dyn Error>> {
    let mut found = false;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            found = found || path.ends_with(name);
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(found)
}

/// The verdict `larry verify` printed for `report` bytes, trusting `state`'s key.
fn verified_verdict(state: &Path, report: &[u8]) -> Result<Value, Box<dyn Error>> {
    let output = spawn_larry_args(
        &["verify"],
        &[("LARRY_STATE_DIR", state.as_os_str())],
        report,
    )?
    .wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "a genuine denied receipt verifies as rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(json::from_str::<Value>(&String::from_utf8_lossy(
        &output.stdout,
    ))?)
}

/// Runs a brief with `goal` under `policy` against a model endpoint that is bound
/// but never answered, and returns the output once no connection reached it: a
/// connection attempt completes the TCP handshake into the listener's backlog, so
/// one accepted connection is proof a request was attempted.
fn unsent_brief(
    state: &Path,
    repo: &Path,
    goal: &str,
    policy: &str,
) -> Result<Output, Box<dyn Error>> {
    let endpoint = TcpListener::bind(("127.0.0.1", 0))?;
    endpoint.set_nonblocking(true)?;
    let url = format!("http://127.0.0.1:{}/v1", endpoint.local_addr()?.port());
    let task = brief_json(repo, goal, r#"["note.txt"]"#, r#"[["true"]]"#);
    let env: [(&str, &OsStr); 5] = [
        ("LARRY_STATE_DIR", state.as_os_str()),
        ("LARRY_POLICY", OsStr::new(policy)),
        ("LARRY_MODEL", OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", OsStr::new(BRIEF_KEY)),
        ("LARRY_MODEL_URL", OsStr::new(url.as_str())),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    assert!(
        endpoint.accept().is_err(),
        "no connection may reach the model endpoint"
    );
    Ok(output)
}

/// A brief under `policy`, which lacks `scope`, is denied: nothing runs, nothing
/// is sent, and the sealed report is genuine but rejected.
fn a_brief_is_denied(policy: &str, scope: &str) -> TestResult {
    let (state, origin) = fixture()?;
    let output = unsent_brief(state.path(), origin.path(), "write the note", policy)?;
    assert_eq!(output.status.code(), Some(2), "a gate denial exits 2");
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "a gate denial is never accepted"
    );
    assert_eq!(
        denial_lines(&report)?,
        vec![format!("gate: policy_refuses: {scope}")],
        "the denial names the first scope the policy refuses"
    );
    assert_eq!(
        report.get("diff").map(Value::is_null),
        Some(true),
        "a denied run seals no diff"
    );
    assert!(
        result_at(&report, 0).is_err(),
        "no command result exists after the denial"
    );
    let verdict = verified_verdict(state.path(), &output.stdout)?;
    assert!(flag_of(&verdict, "verified")?, "the receipt is genuine");
    assert!(
        !flag_of(&verdict, "accepted")?,
        "the verdict says not accepted"
    );
    assert_eq!(
        text_of(&verdict, "task_id")?,
        text_of(&report, "task_id")?,
        "the verdict names the denied task"
    );
    Ok(())
}

#[test]
fn exec_denies_a_brief_task_when_the_policy_lacks_file_write() -> TestResult {
    a_brief_is_denied(
        "file.read,process.spawn,model.invoke,net.egress",
        "file.write",
    )
}

/// A brief sends the repository's files to a model (larry#589).
#[test]
fn exec_denies_a_brief_task_when_the_policy_lacks_net_egress() -> TestResult {
    a_brief_is_denied(
        "file.read,file.write,process.spawn,model.invoke",
        "net.egress",
    )
}

/// A brief is sent as it stands and its files are rewritten from the reply, so a
/// key in it cannot be masked: the brief is refused and nothing is sent.
#[test]
fn exec_refuses_a_brief_that_holds_the_route_key() -> TestResult {
    let (state, origin) = fixture()?;
    let goal = format!("write the note with {BRIEF_KEY}");
    let output = unsent_brief(state.path(), origin.path(), &goal, BRIEF_POLICY)?;
    assert_eq!(output.status.code(), Some(1), "a refused brief exits 1");
    let printed = String::from_utf8_lossy(&output.stderr);
    assert!(
        printed.contains("the goal holds a key or token"),
        "the refusal names the part: {printed}"
    );
    assert!(!printed.contains(BRIEF_KEY), "the key is never echoed");
    assert!(output.stdout.is_empty(), "nothing is sealed");
    Ok(())
}

/// An empty policy admits nothing, so the first required scope in Scope order
/// (file.read) is named, the acceptance command that would create the marker never
/// runs, and neither the state directory nor the origin changes (larry#164).
#[test]
fn exec_denies_every_command_when_the_policy_is_empty() -> TestResult {
    let (state, origin) = fixture()?;
    let before = tree_digest(origin.path())?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","echo pwned > marker"]]"#,
    );
    let env: [(&str, &OsStr); 2] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        ("LARRY_POLICY", OsStr::new("")),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "an empty policy denies with a sealed report"
    );
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "an empty policy accepts nothing"
    );
    assert_eq!(
        denial_lines(&report)?,
        vec![String::from("gate: policy_refuses: file.read")],
        "the first required scope in Scope order is named"
    );
    assert!(
        !has_entry_named(state.path(), "marker")?,
        "no marker file may exist anywhere under the state directory"
    );
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "an empty policy must leave the origin byte-identical"
    );
    Ok(())
}

/// A policy name the executor does not know is refused before anything is read or
/// run: one typed line naming it, exit 1, nothing sealed (larry#164).
#[test]
fn exec_refuses_an_unknown_policy_name() -> TestResult {
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", "[]");
    let env: [(&str, &OsStr); 2] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        ("LARRY_POLICY", OsStr::new("file.read,root")),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    assert_refused(&output, "root");
    Ok(())
}
