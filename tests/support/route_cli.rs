//! Shared harness of the route-table simulations (larry#201): a state
//! directory that removes itself, and `larry route` driven through its own
//! entry point (`executor::routes::run`), the code the shipped binary calls.
//! `tests/sim_route_table.rs` and `tests/sim_route_resolve.rs` include this file
//! and use every item in it.

use std::error::Error;
use std::path::Path;
use std::thread::ScopedJoinHandle;

use larry_demo::executor::routes::{self, RouteCliError};
use lgwks_std::json::{self, Value};

#[path = "route_key.rs"]
pub(crate) mod route_key;

#[path = "temp_dir.rs"]
mod temp_dir;
pub(crate) use temp_dir::TempDir;

/// What every simulation returns.
pub(crate) type TestResult = Result<(), Box<dyn Error>>;

/// A gateway URL no simulation connects to.
pub(crate) const GATEWAY: &str = "https://gateway.test:8443/v1";

/// Route ids the generated tables draw from.
pub(crate) const POOL: [&str; 6] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

/// Runs `larry route <args>` against `state` with the extra variables `env`,
/// and returns what it printed.
pub(crate) fn route_run(
    state: &Path,
    env: &[(&str, &str)],
    args: &[&str],
) -> Result<String, RouteCliError> {
    let mut table = vec![(
        String::from("LARRY_STATE_DIR"),
        state.to_string_lossy().into_owned(),
    )];
    table.extend(
        env.iter()
            .map(|pair| (pair.0.to_owned(), pair.1.to_owned())),
    );
    let mut out = Vec::new();
    routes::run(
        move |name| {
            table
                .iter()
                .find(|entry| entry.0 == name)
                .map(|entry| entry.1.clone())
        },
        args,
        &mut out,
    )?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The flags of a valid `route add`, each with the value a case may replace. The
/// key variable is the tenant's own, so `add` fills it from the state directory.
const DEFAULTS: [(&str, &str); 6] = [
    ("--url", GATEWAY),
    ("--model", "model-1"),
    ("--residency", "CA"),
    ("--temperature", "0.5"),
    ("--request-bytes", "4096"),
    ("--describe", "a sim route"),
];

/// `route add <id>` with the default flags, each replaced by the matching
/// entry of `overrides`, and `extra` flags appended.
pub(crate) fn add(
    state: &Path,
    id: &str,
    overrides: &[(&str, &str)],
    extra: &[&str],
) -> Result<String, RouteCliError> {
    let key = match overrides.iter().find(|pair| pair.0 == "--key-env") {
        Some(pair) => pair.1.to_owned(),
        None => route_key::key_env(state, "SIM").map_err(RouteCliError::Usage)?,
    };
    let mut args = vec!["add", id];
    for (flag, value) in DEFAULTS {
        args.push(flag);
        let replaced = overrides.iter().find(|pair| pair.0 == flag);
        args.push(match replaced {
            Some(pair) => pair.1,
            None => value,
        });
    }
    args.push("--key-env");
    args.push(&key);
    args.extend_from_slice(extra);
    route_run(state, &[], &args)
}

/// Parses printed JSON.
pub(crate) fn parse(text: &str) -> Result<Value, Box<dyn Error>> {
    Ok(json::from_str::<Value>(text)?)
}

/// The ids `route list --json` prints, in order, and the default.
pub(crate) fn listed(state: &Path) -> Result<(Vec<String>, Option<String>), Box<dyn Error>> {
    let value = parse(&route_run(state, &[], &["list", "--json"])?)?;
    let routes = value
        .get("routes")
        .and_then(Value::as_array)
        .ok_or("a routes array")?;
    let mut ids = Vec::with_capacity(routes.len());
    for route in routes {
        let id = route
            .get("id")
            .and_then(Value::as_str)
            .ok_or("a route id")?;
        ids.push(String::from(id));
    }
    let default = value
        .get("default")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok((ids, default))
}

/// Joins a scoped thread, a panic being a failure of the case.
pub(crate) fn joined<Out>(handle: ScopedJoinHandle<'_, Out>) -> Result<Out, Box<dyn Error>> {
    handle
        .join()
        .map_err(|panic| format!("a thread panicked: {panic:?}").into())
}
