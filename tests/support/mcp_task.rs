//! Drives `larry_submit_task` against the shipped binary with an executor
//! behind it (larry#123): the operator's environment, one connected session,
//! a briefed task, one tool call, and the offline `larry key` / `larry verify`
//! check of the sealed report. Source: MCP specification 2025-06-18
//! ("Tools: calling tools") and RFC 8032 (the receipt's ed25519).

use crate::json_at::At;
use crate::mcp_support::{Session, TestResult};
use lgwks_std::json::{self, Value, json};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::coverage::forward;

/// The variables an operator exports to put an executor behind `larry mcp`:
/// the state directory, the gateway, a key, no backoff wait, plus `extra`.
pub(crate) fn operator(
    state: &Path,
    gateway: SocketAddr,
    extra: &[(&'static str, &str)],
) -> Vec<(&'static str, OsString)> {
    let mut env = vec![
        ("LARRY_STATE_DIR", state.as_os_str().to_owned()),
        (
            "LARRY_MODEL_URL",
            OsString::from(format!("http://{gateway}/v1")),
        ),
        ("LARRY_MODEL", OsString::from("mock-model")),
        ("LARRY_MODEL_KEY", OsString::from("mcp-key")),
        ("LARRY_MODEL_BACKOFF_MS", OsString::from("0")),
    ];
    env.extend(
        extra
            .iter()
            .map(|&(name, value)| (name, OsString::from(value))),
    );
    env
}

/// Provisions the exact state ledger the remote MCP process will read: the repository root and
/// every scope needed by an isolated submitted task, network egress included
/// for the briefed tasks that reach the model (larry#203).
pub(crate) fn provision(state: &Path, repo: &Path) -> TestResult<()> {
    let state_text = state.to_string_lossy().into_owned();
    larry_demo::executor::provision_tenant(
        move |_| Some(state_text.clone()),
        "file.read,file.write,process.spawn,model.invoke,model.command,net.egress",
        &[repo],
    )?;
    Ok(())
}

/// `larry mcp` under `env`, initialized.
pub(crate) fn connected(env: &[(&'static str, OsString)]) -> TestResult<Session> {
    let borrowed: Vec<(&str, &OsStr)> = env
        .iter()
        .map(|entry| (entry.0, entry.1.as_os_str()))
        .collect();
    let mut session = Session::start_with(&borrowed)?;
    session.initialize()?;
    Ok(session)
}

/// A task with a brief: the model writes each of `files`.
pub(crate) fn briefed(repo: &Path, files: &[&str], acceptance: &[&str]) -> Value {
    json!({
        "goal": "replace the readme and add a module",
        "repo": repo.display().to_string(),
        "files": files,
        "acceptance": acceptance
    })
}

/// One `tools/call` of `larry_submit_task` and its `result`.
pub(crate) fn submit(session: &mut Session, task: &Value) -> TestResult<Value> {
    let reply = session.request(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":task}}))?;
    Ok(reply.at("/result")?.clone())
}

/// Runs `larry <subcommand>` with only PATH and `env` set and `input` on stdin.
fn run_larry(subcommand: &str, env: &[(&str, &OsStr)], input: &[u8]) -> TestResult<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg(subcommand)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    forward(&mut command);
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().ok_or("child stdin was not piped")?;
    stdin.write_all(input)?;
    drop(stdin);
    Ok(child.wait_with_output()?)
}

/// Verifies `report` offline under the key `larry key` prints for `state`;
/// returns the exit code and the verdict line (printed for accepted and rejected alike).
pub(crate) fn verify_offline(state: &Path, report: &Value) -> TestResult<(Option<i32>, Value)> {
    let key = run_larry("key", &[("LARRY_STATE_DIR", state.as_os_str())], b"")?;
    let printed = String::from_utf8(key.stdout)?;
    let trusted = OsString::from(printed.trim_end());
    let verified = run_larry(
        "verify",
        &[("LARRY_RECEIPT_KEY", trusted.as_os_str())],
        report.to_string().as_bytes(),
    )?;
    let verdict = json::from_slice::<Value>(&verified.stdout)?;
    Ok((verified.status.code(), verdict))
}
