//! The tenant residency policy through the shipped binary (larry#203, larry#587).
//! Source: OWASP ASVS 4.0.3 section 8.3 (customer data is classified and kept
//! where policy says). Requirement proved: a route outside the stored country
//! allowlist is refused before any connection, for a briefed task and for
//! `route resolve`, and the same route resolves once the policy admits it.

use std::ffi::OsStr;
use std::net::TcpListener;
use std::path::Path;
use std::process::Output;

use super::brief_json;
use super::exec_cli::{TestResult, fixture, spawn_larry_args};

#[path = "route_key.rs"]
mod route_key;

/// Runs `larry route <args>` on the state directory.
fn route(state: &Path, args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    let mut argv = vec!["route"];
    argv.extend_from_slice(args);
    let env = [("LARRY_STATE_DIR", state.as_os_str())];
    Ok(spawn_larry_args(&argv, &env, &[])?.wait_with_output()?)
}

/// The command's stdout, after it exited zero.
fn succeeded(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The command exited 1 with a refusal naming the route and its residency.
fn refused_for_residency(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "typed residency refusal");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("route main"), "names the route: {message}");
    assert!(
        message.contains("outside the tenant residency allowlist"),
        "names the rule: {message}"
    );
}

#[test]
fn route_residency_policy_is_stored_and_filters_model_egress() -> TestResult {
    let (state, origin) = fixture()?;
    let state = state.path();
    let endpoint = TcpListener::bind(("127.0.0.1", 0))?;
    endpoint.set_nonblocking(true)?;
    let url = format!("http://127.0.0.1:{}/v1", endpoint.local_addr()?.port());
    let key_env = route_key::key_env(state, "MAIN")?;
    succeeded(&route(
        state,
        &[
            "add",
            "main",
            "--url",
            url.as_str(),
            "--model",
            "mock-model",
            "--key-env",
            key_env.as_str(),
            "--residency",
            "US",
            "--default",
        ],
    )?);

    let saved = succeeded(&route(state, &["residency", "CA,DE"])?);
    assert!(saved.contains("\"saved\":true"), "{saved}");
    let listing = succeeded(&route(state, &["list", "--json"])?);
    assert!(
        listing.contains("\"residency\":[\"CA\",\"DE\"]"),
        "{listing}"
    );

    // A briefed task: refused before the key is sent anywhere.
    let task = brief_json(
        origin.path(),
        "write the note",
        r#"["note.txt"]"#,
        r#"[["true"]]"#,
    );
    let env: [(&str, &OsStr); 3] = [
        ("LARRY_STATE_DIR", state.as_os_str()),
        (
            "LARRY_POLICY",
            OsStr::new("file.read,file.write,process.spawn,model.invoke,net.egress"),
        ),
        (key_env.as_str(), OsStr::new("tenant-secret")),
    ];
    refused_for_residency(&spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?);
    assert!(
        endpoint.accept().is_err(),
        "no model request leaves the host"
    );

    // The resolver a terminal session uses refuses the same route.
    refused_for_residency(&route(state, &["resolve"])?);

    // A policy that admits the country, or no policy, resolves the route again.
    succeeded(&route(state, &["residency", "US"])?);
    succeeded(&route(state, &["resolve"])?);
    succeeded(&route(state, &["residency", "CA"])?);
    succeeded(&route(state, &["residency", "--clear"])?);
    succeeded(&route(state, &["resolve"])?);
    Ok(())
}
