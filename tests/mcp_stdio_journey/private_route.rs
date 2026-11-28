//! A stored route that points inside the host's own network (larry#627), driven through the
//! shipped `larry mcp`. Source: CWE-918 (server-side request forgery), RFC 3927 (169.254/16,
//! where cloud metadata answers) and the OWASP SSRF Prevention Cheat Sheet. Requirement
//! proved: a remote door refuses such a route typed, seals nothing and opens no connection.

use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::Path;

use larry_demo::executor::routes;

use crate::json_at::At;
use crate::mcp_support::{Session, TestResult};
use crate::mcp_task::{briefed, submit};
use crate::route_key::key_env;

/// Stores `url` as the default route of `state`'s tenant and names its key variable.
fn add_default_route(state: &Path, url: &str) -> TestResult<String> {
    let variable = key_env(state, "INWARD")?;
    let directory = state.to_string_lossy().into_owned();
    let mut out = Vec::new();
    routes::run(
        |name| (name == "LARRY_STATE_DIR").then(|| directory.clone()),
        &[
            "add",
            "inward",
            "--url",
            url,
            "--model",
            "model-a",
            "--key-env",
            &variable,
            "--residency",
            "CA",
            "--default",
        ],
        &mut out,
    )?;
    Ok(variable)
}

#[test]
fn a_route_to_the_metadata_address_is_refused() -> TestResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    for url in [
        format!("https://localhost:{port}/v1"),
        String::from("https://169.254.169.254/v1"),
        String::from("https://vault.internal/v1"),
    ] {
        let (state, origin) = crate::origin::fixture()?;
        let variable = add_default_route(state.path(), &url)?;
        let key = OsString::from("route-key");
        let env: [(&str, &OsStr); 2] = [
            ("LARRY_STATE_DIR", state.path().as_os_str()),
            (variable.as_str(), key.as_os_str()),
        ];
        let mut session = Session::start_with(&env)?;
        session.initialize()?;
        let result = submit(
            &mut session,
            &briefed(origin.path(), &["README"], &["true"]),
        )?;
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
        assert!(
            result.at("/structuredContent")?.get("report").is_none(),
            "{url}: a refusal seals nothing: {result}"
        );
        let (code, message) = super::failure_of(&result)?;
        assert!(
            code == "task_refused" && message.contains("private or local destination"),
            "{url}: {code}: {message}"
        );
    }
    assert!(
        matches!(listener.accept(), Err(ref cause) if cause.kind() == ErrorKind::WouldBlock),
        "no connection was opened to the local listener"
    );
    Ok(())
}
