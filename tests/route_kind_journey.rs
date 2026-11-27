//! Refusals of route kinds the wire does not serve yet (larry#539): until the
//! per-kind adapters of PR #534 land, the wire speaks OpenAI chat only, so
//! `route add` of any other kind is refused typed with nothing stored, and
//! `route resolve` of a stored row of such a kind is refused the same way.
//!
//! Source: the Director's SD-1 half-built rule; a user's first call must never
//! go out in the wrong shape. Requirement proved: each unserved kind's add
//! exits 1 with the exact no-adapter line and leaves an empty list, and a
//! stored row rewritten to an unserved kind resolves to the same line.
//! Every run drives the shipped binary with a cleared environment.

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
use lgwks_deps::storefront::rusqlite::Connection;
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

/// Until the per-kind wire adapters land, the wire speaks OpenAI chat only:
/// adding any other kind exits 1 with the exact no-adapter line and stores
/// nothing, and resolving a stored row of such a kind exits 1 the same way.
#[test]
fn an_unserved_kind_is_refused_before_anything_is_stored() -> TestResult {
    for kind in ["anthropic-messages", "openai-responses"] {
        let state = TempDir::new()?;
        let key_env = route_key::key_env(state.path(), "KIND")?;
        let refused = larry_route(
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
            ],
        )?;
        assert_eq!(
            refused.status.code(),
            Some(1),
            "{kind} is refused: {}",
            stderr_of(&refused)
        );
        let line = format!("route kind {kind} has no adapter in this build");
        assert!(
            stderr_of(&refused).lines().any(|text| text.trim() == line),
            "{kind}: the refusal is the exact line: {}",
            stderr_of(&refused)
        );
        let stored = larry_route(state.path(), &["list"])?;
        assert!(
            stored.stdout.is_empty(),
            "{kind}: no refused route was stored"
        );
    }
    let state = TempDir::new()?;
    let key_env = route_key::key_env(state.path(), "KIND")?;
    let added = larry_route(
        state.path(),
        &[
            "add",
            "main",
            "--url",
            "https://gateway.test:8443/v1",
            "--model",
            "model-a",
            "--key-env",
            &key_env,
            "--residency",
            "CA",
            "--default",
        ],
    )?;
    assert!(
        added.status.success(),
        "openai-chat stores: {}",
        stderr_of(&added)
    );
    Connection::open(state.path().join("executor.db"))?
        .execute("UPDATE routes SET kind = 'anthropic-messages'", [])?;
    let resolved = larry_route(state.path(), &["resolve"])?;
    assert_eq!(
        resolved.status.code(),
        Some(1),
        "a stored unserved kind is refused: {}",
        stderr_of(&resolved)
    );
    assert!(
        stderr_of(&resolved)
            .lines()
            .any(|text| text.trim() == "route kind anthropic-messages has no adapter in this build"),
        "the stored refusal is the exact line: {}",
        stderr_of(&resolved)
    );
    Ok(())
}
