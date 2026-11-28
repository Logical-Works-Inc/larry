//! Journeys of `larry route price` (larry#204) as an operator lives it: `route
//! add` stores the route, `route price` sets its versioned price table, and
//! `list` and `resolve` print it. Every run drives the shipped binary with a
//! cleared environment, so no test touches the network: pricing never sends a
//! model call.
//!
//! Source: the tenant route table (larry#201), the eval price grammar
//! (dollars per million tokens, at most ten thousand) and POSIX exit status.
//! Requirement proved: a priced row round trips with its revision, every set
//! bumps, and a bad price or an unknown id is a typed refusal that changes
//! nothing.

#[path = "support/json_at.rs"]
mod json_at;
use json_at::At;

#[path = "support/route_key.rs"]
mod route_key;

#[path = "support/temp_dir.rs"]
mod temp_dir;
use temp_dir::TempDir;

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/route_spawn.rs"]
mod route_spawn;
use route_spawn::{larry_route, stderr_of};

use lgwks_std::json::{self, Value};
use std::error::Error;

/// What every journey returns.
type TestResult = Result<(), Box<dyn Error>>;

/// A route's price table is versioned through the binary: `route price` sets
/// dollars-per-million-tokens rates, the revision bumps on every set, `list`
/// and `resolve` print the table, and a bad price or an unknown id is a typed
/// refusal that changes nothing. Catches a price the store cannot round trip
/// and a scorecard priced from a table no operator can set.
#[test]
fn a_priced_route_is_versioned_and_listed() -> TestResult {
    let state = TempDir::new()?;
    let key_env = route_key::key_env(state.path(), "PRICE")?;
    let added = larry_route(
        state.path(),
        &[
            "add",
            "flash",
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
    let listed = larry_route(state.path(), &["list", "--json"])?;
    let value: Value = json::from_slice(&listed.stdout)?;
    assert_eq!(
        value.at("/routes/0/price_version")?.as_u64(),
        Some(1),
        "a new route carries the first table revision"
    );
    let priced = larry_route(
        state.path(),
        &["price", "flash", "--in", "0.30", "--out", "1.20"],
    )?;
    assert!(
        priced.status.success(),
        "route price: {}",
        stderr_of(&priced)
    );
    let saved: Value = json::from_slice(&priced.stdout)?;
    assert_eq!(
        saved.at("/price_version")?.as_u64(),
        Some(2),
        "the set bumps"
    );
    assert_eq!(
        saved.at("/price_input_micros")?.as_u64(),
        Some(300_000),
        "thirty cents a million tokens in"
    );
    assert_eq!(
        saved.at("/price_output_micros")?.as_u64(),
        Some(1_200_000),
        "a dollar twenty out"
    );
    let again = larry_route(
        state.path(),
        &["price", "flash", "--in", "0.30", "--out", "1.20"],
    )?;
    let repeated: Value = json::from_slice(&again.stdout)?;
    assert_eq!(
        repeated.at("/price_version")?.as_u64(),
        Some(3),
        "every set bumps the revision"
    );
    let resolved = larry_route(state.path(), &["resolve", "flash"])?;
    let seal: Value = json::from_slice(&resolved.stdout)?;
    assert_eq!(
        seal.at("/price_version")?.as_u64(),
        Some(3),
        "the sealed view carries the table"
    );
    for args in [
        vec!["price", "nope", "--in", "0.30", "--out", "1.20"],
        vec!["price", "flash", "--in", "abc", "--out", "1.20"],
        vec!["price", "flash", "--in", "0.30", "--out", "10000.01"],
        vec!["price", "flash", "--in", "0.30"],
    ] {
        let refused = larry_route(state.path(), &args)?;
        assert!(
            !refused.status.success(),
            "a bad price is refused: {args:?}"
        );
    }
    let kept = larry_route(state.path(), &["resolve", "flash"])?;
    let seal: Value = json::from_slice(&kept.stdout)?;
    assert_eq!(
        seal.at("/price_version")?.as_u64(),
        Some(3),
        "refused sets keep the table they found"
    );
    Ok(())
}
