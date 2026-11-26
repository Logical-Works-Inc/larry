//! Journeys proving model egress checks happen before a request leaves Larry.

use std::ffi::OsStr;
use std::net::TcpListener;
use std::path::Path;

use super::brief_json;
use super::exec_cli::{TestResult, denial_lines, fixture, flag_of, report_of, spawn_larry_args};

#[path = "route_key.rs"]
mod route_key;

#[test]
fn exec_denies_a_brief_task_when_the_policy_lacks_network_egress() -> TestResult {
    // Source: OWASP ASVS 4.0.3 section 4.1.3. Model work is an external effect.
    let (state, origin) = fixture()?;
    let (endpoint, url) = local_endpoint()?;
    let task = note_task(origin.path());
    let env: [(&str, &OsStr); 4] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        (
            "LARRY_POLICY",
            OsStr::new("file.read,file.write,model.invoke"),
        ),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_URL", OsStr::new(url.as_str())),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    assert!(
        endpoint.accept().is_err(),
        "the gate denies before the model step"
    );
    let report = report_of(&output)?;
    assert_eq!(
        denial_lines(&report)?,
        vec![String::from("gate: policy_refuses: net.egress")]
    );
    assert!(
        !flag_of(&report, "accepted")?,
        "a gate denial is never accepted"
    );
    Ok(())
}

#[test]
fn route_residency_policy_is_stored_and_filters_model_egress() -> TestResult {
    let (state, origin) = fixture()?;
    let (endpoint, url) = local_endpoint()?;
    let key_env = route_key::key_env(state.path(), "MAIN")?;
    let add_args = [
        "route",
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
    ];
    let added = spawn_larry_args(
        &add_args,
        &[("LARRY_STATE_DIR", state.path().as_os_str())],
        &[],
    )?
    .wait_with_output()?;
    assert!(
        added.status.success(),
        "route add: {}",
        String::from_utf8_lossy(&added.stderr)
    );

    let set = spawn_larry_args(
        &["route", "residency", "CA,DE"],
        &[("LARRY_STATE_DIR", state.path().as_os_str())],
        &[],
    )?
    .wait_with_output()?;
    assert!(
        set.status.success(),
        "route residency: {}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(
        String::from_utf8_lossy(&set.stdout).contains("\"saved\":true"),
        "the stored policy reports saved: {}",
        String::from_utf8_lossy(&set.stdout)
    );

    let listed = spawn_larry_args(
        &["route", "list", "--json"],
        &[("LARRY_STATE_DIR", state.path().as_os_str())],
        &[],
    )?
    .wait_with_output()?;
    let listing = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listing.contains("\"residency\":[\"CA\",\"DE\"]"),
        "{listing}"
    );

    let task = note_task(origin.path());
    let output = spawn_larry_args(
        &["exec"],
        &[
            ("LARRY_STATE_DIR", state.path().as_os_str()),
            (
                "LARRY_POLICY",
                OsStr::new("file.read,file.write,process.spawn,model.invoke,net.egress"),
            ),
            (key_env.as_str(), OsStr::new("tenant-secret")),
        ],
        task.as_bytes(),
    )?
    .wait_with_output()?;
    assert_eq!(output.status.code(), Some(1), "typed residency refusal");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("route main"),
        "the refusal names route: {message}"
    );
    assert!(
        message.contains("residency"),
        "the refusal names residency: {message}"
    );
    assert!(
        endpoint.accept().is_err(),
        "no model request leaves the host"
    );

    let cleared = spawn_larry_args(
        &["route", "residency", "--clear"],
        &[("LARRY_STATE_DIR", state.path().as_os_str())],
        &[],
    )?
    .wait_with_output()?;
    assert!(cleared.status.success(), "route residency clear");
    Ok(())
}

/// A loopback endpoint that must receive no connection.
fn local_endpoint() -> Result<(TcpListener, String), Box<dyn std::error::Error>> {
    let endpoint = TcpListener::bind(("127.0.0.1", 0))?;
    endpoint.set_nonblocking(true)?;
    let url = format!("http://127.0.0.1:{}/v1", endpoint.local_addr()?.port());
    Ok((endpoint, url))
}

#[test]
fn exec_refuses_plaintext_non_loopback_before_any_connection() -> TestResult {
    let (state, origin) = fixture()?;
    let (endpoint, _local_url) = local_endpoint()?;
    let task = note_task(origin.path());
    let env: [(&str, &OsStr); 6] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        (
            "LARRY_POLICY",
            OsStr::new("file.read,file.write,process.spawn,model.invoke,net.egress"),
        ),
        (
            "LARRY_MODEL_URL",
            OsStr::new("http://gateway.example:8443/v1"),
        ),
        ("LARRY_MODEL", OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_RESIDENCY", OsStr::new("US")),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    let message = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "typed transport refusal: {message}"
    );
    assert!(message.contains("requires https"), "{message}");
    assert!(
        endpoint.accept().is_err(),
        "the listener saw zero connections"
    );
    Ok(())
}

/// The brief shared by both pre-send refusal journeys.
fn note_task(origin: &Path) -> String {
    brief_json(origin, "write the note", r#"["note.txt"]"#, r#"[["true"]]"#)
}
