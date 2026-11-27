//! Every named route kind is served by its wire adapter (larry#205,
//! larry#546): `route add` of any named kind stores it, and `route resolve`
//! of a stored row of such a kind resolves it. An unknown kind is still
//! refused before anything is stored.
//!
//! Source: ADR 0126 (the no-adapter refusal PR #534's remainder deletes).
//! Requirement proved: each named kind's add succeeds and resolves to a seal
//! naming the kind. Every run drives the shipped binary with a cleared
//! environment.

#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/route_key.rs"]
mod route_key;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::error::Error;
use std::path::Path;
use std::process::Output;

use larry_cli::{spawn_larry_args, stderr_of};
use temp_dir::TempDir;

/// What every journey returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Runs `larry route <args>` with only PATH and the state directory set.
fn larry_route(state: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let mut full = vec!["route"];
    full.extend_from_slice(args);
    let env = [("LARRY_STATE_DIR", state.as_os_str())];
    Ok(spawn_larry_args(full.as_slice(), &env, &[])?.wait_with_output()?)
}

/// Every named kind is served: adding it stores it, and resolving it seals
/// its name. An unknown kind is refused before anything is stored.
#[test]
fn every_named_kind_is_served_and_seals_its_name() -> TestResult {
    for kind in ["openai-chat", "anthropic-messages", "openai-responses"] {
        let state = TempDir::new()?;
        let key_env = route_key::key_env(state.path(), "KIND")?;
        let added = larry_route(
            state.path(),
            &[
                "add",
                "other",
                "--url",
                "https://gateway.test:8443/v1",
                "--model",
                "model-a",
                "--key-env",
                &key_env,
                "--residency",
                "CA",
                "--kind",
                kind,
                "--default",
            ],
        )?;
        assert!(
            added.status.success(),
            "{kind} is served: {}",
            stderr_of(&added)
        );
        let resolved = larry_route(state.path(), &["resolve"])?;
        assert!(
            resolved.status.success(),
            "{kind} resolves: {}",
            stderr_of(&resolved)
        );
        let seal = String::from_utf8_lossy(&resolved.stdout);
        assert!(
            seal.contains(&format!("\"kind\":\"{kind}\"")),
            "{kind} seals with its name: {seal}"
        );
    }
    let state = TempDir::new()?;
    let key_env = route_key::key_env(state.path(), "KIND")?;
    let refused = larry_route(
        state.path(),
        &[
            "add",
            "bogus",
            "--url",
            "https://gateway.test:8443/v1",
            "--model",
            "model-a",
            "--key-env",
            &key_env,
            "--residency",
            "CA",
            "--kind",
            "bogus-kind",
        ],
    )?;
    assert_eq!(
        refused.status.code(),
        Some(1),
        "an unknown kind is refused: {}",
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("route kind must be one of"),
        "the refusal names the kind: {}",
        stderr_of(&refused)
    );
    let stored = larry_route(state.path(), &["list"])?;
    assert!(stored.stdout.is_empty(), "no refused route was stored");
    Ok(())
}
