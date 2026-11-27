//! Seeded simulations of versioned route prices (larry#204), through `larry
//! route`'s own entry point, the code the shipped binary calls. Source: the
//! tenant route table, SQLite upsert atomicity for the 256-route cap, the eval
//! price grammar (dollars per million tokens, at most ten thousand) and ISO
//! 3166-1 alpha-2. Requirement proved: `price` sets micro-dollar rates and
//! bumps the revision on every set, re-adding a route keeps its table, every
//! malformed rate and unknown id is its own refusal that stores nothing, eight
//! concurrent pricings bump once each, and every tabled route prices alone.

#[path = "support/json_at.rs"]
mod json_at;
use json_at::At;

#[path = "support/route_cli.rs"]
mod route_cli;

use route_cli::{POOL, TempDir, TestResult, add, joined, listed, parse, route_run};
use std::sync::Barrier;

/// One route's price revision in a `list --json` table.
fn version(table: &lgwks_std::json::Value) -> Result<u64, Box<dyn std::error::Error>> {
    table
        .at("/routes/0/price_version")
        .map_err(|cause| format!("a price revision: {cause}"))?
        .as_u64()
        .ok_or_else(|| "a price revision is not a number".into())
}

/// A route's price table is versioned and typed through the entry point: an
/// unpriced row carries revision 1 with no rates, `price` sets dollars-per-
/// million-tokens rates and bumps the revision on every set, re-adding the
/// route keeps the table, and a malformed, too-precise or too-large rate, an
/// unknown id, a missing rate or a bad flag is its own refusal that stores
/// nothing. Catches a price the entry point cannot set and a table `add`
/// wipes.
#[test]
fn sim_route_price_is_versioned_and_typed() -> TestResult {
    let state = TempDir::new()?;
    add(state.path(), "flash", &[], &[])?;
    let table = || parse(&route_run(state.path(), &[], &["list", "--json"])?);
    assert_eq!(version(&table()?)?, 1, "an unpriced row carries revision 1");
    for (input, output) in [("0.30", "1.20"), ("3", "15")] {
        let saved = parse(&route_run(
            state.path(),
            &[],
            &["price", "flash", "--in", input, "--out", output],
        )?)?;
        assert_eq!(saved.at("/priced")?.as_bool(), Some(true), "priced");
    }
    let priced = table()?;
    assert_eq!(version(&priced)?, 3, "two sets bump twice");
    assert_eq!(
        priced.at("/routes/0/price_input_micros")?.as_u64(),
        Some(3_000_000),
        "three dollars a million tokens in"
    );
    assert_eq!(
        priced.at("/routes/0/price_output_micros")?.as_u64(),
        Some(15_000_000),
        "fifteen dollars out"
    );
    // Re-adding the route replaces the fields `add` names and keeps the table.
    add(state.path(), "flash", &[("--model", "model-2")], &[])?;
    let kept = table()?;
    assert_eq!(version(&kept)?, 3, "a re-add keeps the priced table");
    assert_eq!(
        kept.at("/routes/0/model")?.as_str(),
        Some("model-2"),
        "a re-add still replaces what it names"
    );
    for args in [
        vec!["price", "nope", "--in", "0.30", "--out", "1.20"],
        vec!["price", "flash", "--in", "abc", "--out", "1.20"],
        vec!["price", "flash", "--in", "1e3", "--out", "1.20"],
        vec!["price", "flash", "--in", "0.0000001", "--out", "1.20"],
        vec!["price", "flash", "--in", "0.30", "--out", "10000.01"],
        vec!["price", "flash", "--in", "-1", "--out", "1.20"],
        vec!["price", "flash", "--in", "0.30"],
        vec![
            "price", "flash", "--in", "0.30", "--out", "1.20", "--in", "3",
        ],
        vec![
            "price", "flash", "--in", "0.30", "--out", "1.20", "--bogus", "1",
        ],
    ] {
        assert!(
            route_run(state.path(), &[], &args).is_err(),
            "a bad price is refused: {args:?}"
        );
    }
    // The boundary holds: ten thousand dollars prices, one micro-dollar more refuses.
    route_run(
        state.path(),
        &[],
        &["price", "flash", "--in", "10000", "--out", "0"],
    )?;
    assert!(
        route_run(
            state.path(),
            &[],
            &["price", "flash", "--in", "10000.000001", "--out", "0"],
        )
        .is_err(),
        "one micro-dollar past ten thousand refuses"
    );
    assert_eq!(
        version(&table()?)?,
        4,
        "refused sets keep the table they found"
    );
    Ok(())
}

/// Eight pricings at once bump once each: the revision bump is one statement,
/// so two writers racing on one row cannot write one revision. The final table
/// carries revision 9 whatever the interleaving.
#[test]
fn sim_concurrent_prices_bump_once_each() -> TestResult {
    let state = TempDir::new()?;
    add(state.path(), "hot", &[], &[])?;
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| -> TestResult {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let (barrier, path) = (&barrier, state.path());
            handles.push(scope.spawn(move || {
                barrier.wait();
                route_run(path, &[], &["price", "hot", "--in", "1", "--out", "2"])
                    .map(drop)
                    .map_err(|error| error.to_string())
            }));
        }
        for handle in handles {
            joined(handle)??;
        }
        Ok(())
    })?;
    let value = parse(&route_run(state.path(), &[], &["list", "--json"])?)?;
    assert_eq!(
        version(&value)?,
        9,
        "eight concurrent sets bump once each from revision 1"
    );
    Ok(())
}

/// Every tabled route prices alone: pricing one id leaves every other row's
/// table and revision untouched. Catches a price written to the wrong row.
#[test]
fn sim_every_tabled_route_prices_alone() -> TestResult {
    let state = TempDir::new()?;
    for id in POOL {
        add(state.path(), id, &[], &[])?;
    }
    assert_eq!(
        listed(state.path())?.0.len(),
        POOL.len(),
        "the pool is tabled"
    );
    for (position, id) in POOL.iter().enumerate() {
        let dollars = position.saturating_add(1).to_string();
        route_run(
            state.path(),
            &[],
            &["price", id, "--in", &dollars, "--out", &dollars],
        )?;
    }
    let value = parse(&route_run(state.path(), &[], &["list", "--json"])?)?;
    let rows = value
        .get("routes")
        .and_then(lgwks_std::json::Value::as_array)
        .ok_or("a routes array")?;
    for (position, id) in POOL.iter().enumerate() {
        let row = rows
            .iter()
            .find(|row| row.get("id").and_then(lgwks_std::json::Value::as_str) == Some(*id))
            .ok_or("a priced row")?;
        let rate = u64::try_from(position.saturating_add(1))?.saturating_mul(1_000_000);
        assert_eq!(
            row.get("price_input_micros")
                .and_then(lgwks_std::json::Value::as_u64),
            Some(rate),
            "{id} prices its own input"
        );
        assert_eq!(
            row.get("price_output_micros")
                .and_then(lgwks_std::json::Value::as_u64),
            Some(rate),
            "{id} prices its own output"
        );
        assert_eq!(
            row.get("price_version")
                .and_then(lgwks_std::json::Value::as_u64),
            Some(2),
            "{id} carries its own revision"
        );
    }
    Ok(())
}
