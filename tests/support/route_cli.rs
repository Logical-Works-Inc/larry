//! Shared harness of the route-table simulations (larry#201): a state
//! directory that removes itself, and `larry route` driven through its own
//! entry point (`executor::routes::run`), the code the shipped binary calls.
//! `tests/sim_route_table.rs` and `tests/sim_route_resolve.rs` include this file
//! and use every item in it.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::ScopedJoinHandle;

use larry_demo::executor::routes::{self, RouteCliError};
use lgwks_std::json::{self, Value};

/// What every simulation returns.
pub type TestResult = Result<(), Box<dyn Error>>;

/// A gateway URL no simulation connects to.
pub const GATEWAY: &str = "http://gateway.test:8443/v1";

/// Route ids the generated tables draw from.
pub const POOL: [&str; 6] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

/// A tenant's state directory, removed when it goes out of scope.
pub struct Scratch {
    /// The directory every route row of this tenant lives under.
    root: PathBuf,
}

impl Scratch {
    /// A new, empty state directory that no other test in this process holds.
    pub fn new() -> io::Result<Scratch> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("larry-route-sim-{}-{serial}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;
        Ok(Scratch { root })
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A leftover temp directory must never fail a simulation.
        drop(fs::remove_dir_all(&self.root));
    }
}

/// Runs `larry route <args>` against `state` with the extra variables `env`,
/// and returns what it printed.
pub fn route_run(
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

/// The flags of a valid `route add`, each with the value a case may replace.
const DEFAULTS: [(&str, &str); 7] = [
    ("--url", GATEWAY),
    ("--model", "model-1"),
    ("--key-env", "LARRY_KEY_SIM"),
    ("--residency", "CA"),
    ("--temperature", "0.5"),
    ("--request-bytes", "4096"),
    ("--describe", "a sim route"),
];

/// `route add <id>` with the default flags, each replaced by the matching
/// entry of `overrides`, and `extra` flags appended.
pub fn add(
    state: &Path,
    id: &str,
    overrides: &[(&str, &str)],
    extra: &[&str],
) -> Result<String, RouteCliError> {
    let mut args = vec!["add", id];
    for (flag, value) in DEFAULTS {
        args.push(flag);
        let replaced = overrides.iter().find(|pair| pair.0 == flag);
        args.push(match replaced {
            Some(pair) => pair.1,
            None => value,
        });
    }
    args.extend_from_slice(extra);
    route_run(state, &[], &args)
}

/// Parses printed JSON.
pub fn parse(text: &str) -> Result<Value, Box<dyn Error>> {
    Ok(json::from_str::<Value>(text)?)
}

/// The ids `route list --json` prints, in order, and the default.
pub fn listed(state: &Path) -> Result<(Vec<String>, Option<String>), Box<dyn Error>> {
    let value = parse(&route_run(state, &[], &["list", "--json"])?)?;
    let routes = value["routes"].as_array().ok_or("a routes array")?;
    let mut ids = Vec::with_capacity(routes.len());
    for route in routes {
        ids.push(String::from(route["id"].as_str().ok_or("a route id")?));
    }
    Ok((ids, value["default"].as_str().map(str::to_owned)))
}

/// Joins a scoped thread, a panic being a failure of the case.
pub fn joined<Out>(handle: ScopedJoinHandle<'_, Out>) -> Result<Out, Box<dyn Error>> {
    handle
        .join()
        .map_err(|_panicked| "a thread panicked".into())
}
