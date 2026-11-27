//! The priced-route journey of `larry eval` (larry#204), split from
//! `tests/eval_journey.rs` so each file stays one responsibility. Source: the
//! tenant route table's versioned price table (ADR 0118). Requirement proved:
//! a stored priced route prices the arm with no `LARRY_EVAL_PRICE_*` in the
//! environment, and the card names the table's revision and arithmetic.

use crate::eval_arm::{FLASH, TestResult, number, operator, origins_left, report, score_arm, task};
use crate::mock_model::{Config, Handle, run};
use crate::route_key;
use crate::temp_dir::TempDir;
use lgwks_std::json::json;
use std::ffi::OsString;
use std::path::Path;

/// Runs `larry route <args>` with only PATH, the state directory and `env` set.
fn larry_route(
    state: &Path,
    env: &[(&str, OsString)],
    args: &[&str],
) -> TestResult<std::process::Output> {
    use std::process::Command;
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state)
        .envs(env.iter().map(|entry| (entry.0, entry.1.as_os_str())));
    Ok(command.output()?)
}

/// A stored priced route prices its arm without eval price settings: `route
/// add` stores the route as the default, `route price` sets thirty cents in
/// and a dollar twenty out, and the card with no `LARRY_EVAL_PRICE_*` carries
/// the table's revision and the table's arithmetic. Catches an arm the route
/// table cannot price and a card priced from a table no command sets.
#[test]
fn a_priced_route_prices_the_arm_without_eval_price_settings() -> TestResult {
    use std::sync::OnceLock;
    // Process-lifetime key variable name: the arm's environment borrows it.
    static PRICED_KEY_ENV: OnceLock<String> = OnceLock::new();
    let set = json!({"name": "priced", "tasks": [task("one", 1, &[])]}).to_string();
    let state = TempDir::new()?;
    run(Config::seeded(21), |handle: &Handle| {
        let gateway = format!("http://{}/v1", handle.addr());
        let key_env = route_key::key_env(&state, "PRICED")?;
        let added = larry_route(
            &state,
            &[],
            &[
                "add",
                "cheap",
                "--url",
                &gateway,
                "--model",
                "priced-model",
                "--key-env",
                &key_env,
                "--residency",
                "CA",
                "--default",
            ],
        )?;
        assert!(added.status.success(), "route add stores the arm's route");
        let priced = larry_route(
            &state,
            &[],
            &["price", "cheap", "--in", "0.30", "--out", "1.20"],
        )?;
        assert!(priced.status.success(), "route price sets the table");
        let key = OsString::from("priced-key");
        let base = operator(&state, handle.addr(), &FLASH, "1");
        let mut env: Vec<(&'static str, OsString)> = base
            .into_iter()
            .filter(|entry| {
                entry.0 != "LARRY_EVAL_PRICE_IN"
                    && entry.0 != "LARRY_EVAL_PRICE_OUT"
                    && entry.0 != "LARRY_MODEL_URL"
                    && entry.0 != "LARRY_MODEL"
            })
            .collect();
        let leaked: &'static str = PRICED_KEY_ENV.get_or_init(|| key_env);
        env.push((leaked, key));
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
