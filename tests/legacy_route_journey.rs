//! Binary route journey for rows written before the transport rule.

#[path = "support/route_key.rs"]
mod route_key;
#[path = "support/temp_dir.rs"]
mod temp_dir;

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/route_spawn.rs"]
mod route_spawn;
use route_spawn::{larry_route, stderr_of};

use std::error::Error;

use lgwks_deps::storefront::rusqlite::Connection;
use temp_dir::TempDir;

type TestResult = Result<(), Box<dyn Error>>;

/// A route written before the transport rule remains inspectable and removable,
/// but resolution refuses it before any key read or network attempt.
#[test]
fn a_legacy_plaintext_route_lists_and_removes_but_resolution_refuses() -> TestResult {
    let state = TempDir::new()?;
    let key_env = route_key::key_env(state.path(), "LEGACY")?;
    let added = larry_route(
        state.path(),
        &[
            "add",
            "legacy",
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
    assert!(added.status.success(), "route add: {}", stderr_of(&added));
    Connection::open(state.path().join("executor.db"))?.execute(
        "UPDATE routes SET base_url = 'http://gateway.test:8443/v1'",
        [],
    )?;
    let listed = larry_route(state.path(), &["list"])?;
    assert!(
        listed.status.success(),
        "legacy list: {}",
        stderr_of(&listed)
    );
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains("legacy"),
        "the old row remains visible"
    );
    let resolved = larry_route(state.path(), &["resolve"])?;
    assert!(!resolved.status.success(), "legacy transport is refused");
    assert!(
        stderr_of(&resolved).contains("requires https"),
        "typed transport refusal: {}",
        stderr_of(&resolved)
    );
    let removed = larry_route(state.path(), &["remove", "legacy"])?;
    assert!(
        removed.status.success(),
        "legacy remove: {}",
        stderr_of(&removed)
    );
    Ok(())
}
