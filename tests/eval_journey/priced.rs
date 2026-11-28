//! The priced-route journey of `larry eval` (larry#204), split from
//! `tests/eval_journey.rs` so each file stays one responsibility. Source: the
//! tenant route table's versioned price table (ADR 0127). Requirement proved:
//! a stored priced route prices the arm with no `LARRY_EVAL_PRICE_*` in the
//! environment, even priced at zero and zero, and the card names the table's
//! revision and arithmetic; an unpriced route with no environment prices is
//! refused before the first model call.

use crate::eval_arm::{FLASH, TestResult, number, operator, origins_left, report, score_arm, task};
use crate::mock_model::{Config, Handle, run};
use crate::route_key;
use crate::temp_dir::TempDir;
use lgwks_std::json::json;
use std::ffi::OsString;
use std::path::Path;
use std::sync::OnceLock;

/// Process-lifetime key variable names: each arm's environment borrows its own.
static PRICED_KEY_ENV: OnceLock<String> = OnceLock::new();
static ZERO_PRICED_KEY_ENV: OnceLock<String> = OnceLock::new();
static UNPRICED_KEY_ENV: OnceLock<String> = OnceLock::new();

/// Runs `larry route <args>` with only PATH, the state directory and `env` set.
fn larry_route(
    state: &Path,
    env: &[(&str, OsString)],
    args: &[&str],
) -> TestResult<std::process::Output> {
    use crate::coverage::forward;
    use std::process::Command;
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state)
        .envs(env.iter().map(|entry| (entry.0, entry.1.as_os_str())));
    forward(&mut command);
    Ok(command.output()?)
}

/// Stores `id` as the tenant default over `gateway`: the arm resolves it, and
/// the key variable `key_env` names is read only when the arm reaches it.
fn add_default(state: &Path, id: &str, model: &str, key_env: &str, gateway: &str) -> TestResult {
    let added = larry_route(
        state,
        &[],
        &[
            "add",
            id,
            "--url",
            gateway,
            "--model",
            model,
            "--key-env",
            key_env,
            "--residency",
            "CA",
            "--default",
        ],
    )?;
    assert!(
        added.status.success(),
        "route add stores the arm's route: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    Ok(())
}

/// The arm's environment: the operator's, without eval or model settings (the
/// route names the model, the table or the refusal names the prices), plus the
/// route's key variable `leaked` holding `key`.
fn arm_env(
    state: &Path,
    gateway: std::net::SocketAddr,
    leaked: &'static str,
    key: &str,
) -> Vec<(&'static str, OsString)> {
    let base = operator(state, gateway, &FLASH, "1");
    let mut env: Vec<(&'static str, OsString)> = base
        .into_iter()
        .filter(|entry| {
            entry.0 != "LARRY_EVAL_PRICE_IN"
                && entry.0 != "LARRY_EVAL_PRICE_OUT"
                && entry.0 != "LARRY_MODEL_URL"
                && entry.0 != "LARRY_MODEL"
        })
        .collect();
    env.push((leaked, OsString::from(key)));
    env
}

/// A stored priced route prices its arm without eval price settings: `route
/// add` stores the route as the default, `route price` sets thirty cents in
/// and a dollar twenty out, and the card with no `LARRY_EVAL_PRICE_*` carries
/// the table's revision and the table's arithmetic. Catches an arm the route
/// table cannot price and a card priced from a table no command sets.
#[test]
fn a_priced_route_prices_the_arm_without_eval_price_settings() -> TestResult {
    let set = json!({"name": "priced", "tasks": [task("one", 1, &[])]}).to_string();
    let state = TempDir::new()?;
    run(Config::seeded(21), |handle: &Handle| {
        let gateway = format!("http://{}/v1", handle.addr());
        let key_env = route_key::key_env(&state, "PRICED")?;
        add_default(&state, "cheap", "priced-model", &key_env, &gateway)?;
        let priced = larry_route(
            &state,
            &[],
            &["price", "cheap", "--in", "0.30", "--out", "1.20"],
        )?;
        assert!(priced.status.success(), "route price sets the table");
        let env = arm_env(
            &state,
            handle.addr(),
            PRICED_KEY_ENV.get_or_init(|| key_env),
            "priced-key",
        );
        let ran = score_arm(&env, set.as_bytes())?;
        assert_eq!(
            ran.code,
            Some(0),
            "the table prices the arm: {}",
            ran.stderr
        );
        let card = ran.card()?;
        assert_eq!(card["set"], "priced", "the set is named");
        assert_eq!(card["model"], "priced-model", "the route names the model");
        assert_eq!(
            card["price_version"], 2,
            "the card names the table revision"
        );
        assert_eq!(
            card["cost_complete"], true,
            "every completion reported usage"
        );
        let prompt = number(card, "prompt_tokens")?;
        let completion = number(card, "completion_tokens")?;
        let expected = prompt
            .saturating_mul(300_000)
            .saturating_add(completion.saturating_mul(1_200_000));
        assert_eq!(
            card["cost_picousd"].as_str(),
            Some(expected.to_string()).as_deref(),
            "the cost is the table's arithmetic on the charged tokens"
        );
        assert_eq!(origins_left(&state)?, 0, "every origin removed");
        report("priced (mock)", &ran)
    })?;
    Ok(())
}

/// A route priced at zero and zero is still a priced route: the revision is
/// past 1, so the arm scores at zero cost with no `LARRY_EVAL_PRICE_*` in its
/// environment. Catches a priced table read as unpriced because both rates are
/// zero (a local model that costs nothing), which would demand environment
/// prices for an arm that needs none.
#[test]
fn a_zero_priced_route_scores_at_zero_cost_with_no_eval_price_settings() -> TestResult {
    let set = json!({"name": "zero", "tasks": [task("one", 1, &[])]}).to_string();
    let state = TempDir::new()?;
    run(Config::seeded(22), |handle: &Handle| {
        let gateway = format!("http://{}/v1", handle.addr());
        let key_env = route_key::key_env(&state, "ZERO")?;
        add_default(&state, "cheap", "zero-model", &key_env, &gateway)?;
        let priced = larry_route(&state, &[], &["price", "cheap", "--in", "0", "--out", "0"])?;
        assert!(priced.status.success(), "route price sets zero rates");
        let env = arm_env(
            &state,
            handle.addr(),
            ZERO_PRICED_KEY_ENV.get_or_init(|| key_env),
            "zero-key",
        );
        let ran = score_arm(&env, set.as_bytes())?;
        assert_eq!(
            ran.code,
            Some(0),
            "a zero-priced table prices the arm: {}",
            ran.stderr
        );
        let card = ran.card()?;
        assert_eq!(
            card["price_version"], 2,
            "the card names the table revision"
        );
        assert!(
            number(card, "prompt_tokens")? > 0,
            "the mock reported real usage"
        );
        assert_eq!(
            card["cost_picousd"].as_str(),
            Some("0"),
            "zero rates price real tokens at zero"
        );
        assert_eq!(origins_left(&state)?, 0, "every origin removed");
        report("zero-priced (mock)", &ran)
    })?;
    Ok(())
}

/// An unpriced route with no environment prices refuses the arm before the
/// first model call: the revision is still 1, so no table can price it and no
/// cost is guessed. Catches a silent zero-cost card for an arm nobody priced.
#[test]
fn an_unpriced_route_with_no_eval_price_settings_is_refused_naming_the_variable() -> TestResult {
    let set = json!({"name": "unpriced", "tasks": [task("one", 1, &[])]}).to_string();
    let state = TempDir::new()?;
    run(Config::seeded(23), |handle: &Handle| -> TestResult {
        let gateway = format!("http://{}/v1", handle.addr());
        let key_env = route_key::key_env(&state, "UNPRICED")?;
        add_default(&state, "plain", "plain-model", &key_env, &gateway)?;
        let env = arm_env(
            &state,
            handle.addr(),
            UNPRICED_KEY_ENV.get_or_init(|| key_env),
            "unpriced-key",
        );
        let ran = score_arm(&env, set.as_bytes())?;
        assert_eq!(
            ran.code,
            Some(1),
            "an unpriced arm with no environment prices stops: {}",
            ran.stderr
        );
        assert!(
            ran.card.is_none(),
            "no scorecard is printed for a refused arm"
        );
        assert!(
            ran.stderr.contains("LARRY_EVAL_PRICE_IN"),
            "the refusal names the missing variable: {}",
            ran.stderr
        );
        assert!(
            handle.requests().is_empty(),
            "no model call is made before the refusal"
        );
        assert_eq!(origins_left(&state)?, 0, "every origin removed");
        Ok(())
    })?;
    Ok(())
}
