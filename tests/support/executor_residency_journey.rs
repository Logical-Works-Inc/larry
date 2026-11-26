//! Journeys proving model egress checks happen before a request leaves Larry.

use std::ffi::OsStr;
use std::net::TcpListener;
use std::path::Path;

use super::brief_json;
use super::exec_cli::{TestResult, denial_lines, fixture, flag_of, report_of, spawn_larry_args};

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
fn exec_refuses_a_route_outside_the_tenant_residency_before_model_send() -> TestResult {
    let (state, origin) = fixture()?;
    let (endpoint, url) = local_endpoint()?;
    let task = note_task(origin.path());
    let env: [(&str, &OsStr); 7] = [
        ("LARRY_STATE_DIR", state.path().as_os_str()),
        (
            "LARRY_POLICY",
            OsStr::new("file.read,file.write,process.spawn,model.invoke,net.egress"),
        ),
        ("LARRY_RESIDENCY_ALLOWLIST", OsStr::new("CA")),
        ("LARRY_MODEL_URL", OsStr::new(url.as_str())),
        ("LARRY_MODEL", OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_RESIDENCY", OsStr::new("US")),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(1),
        "the residency refusal is typed"
    );
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        message.contains("outside the tenant allowlist"),
        "the refusal names the residency boundary: {message}"
    );
    assert!(
        endpoint.accept().is_err(),
        "the route was refused before send"
    );
    Ok(())
}

/// A loopback endpoint that must receive no connection.
fn local_endpoint() -> Result<(TcpListener, String), Box<dyn std::error::Error>> {
    let endpoint = TcpListener::bind(("127.0.0.1", 0))?;
    endpoint.set_nonblocking(true)?;
    let url = format!("http://127.0.0.1:{}/v1", endpoint.local_addr()?.port());
    Ok((endpoint, url))
}

/// The brief shared by both pre-send refusal journeys.
fn note_task(origin: &Path) -> String {
    brief_json(origin, "write the note", r#"["note.txt"]"#, r#"[["true"]]"#)
}
