//! The session-acceptance journey of `larry eval` (larry#217), split from
//! `tests/eval_journey.rs` so each file stays one responsibility. Source: the matched
//! task set's `session-path` task, whose verifier owns the reward (INVARIANTS.md).
//! Requirement proved: a session that ends its turn is accepted only when every
//! acceptance command passes in the origin it worked in.

use crate::eval_arm::{FLASH, Ran, TestResult, operator, origins_left, score_arm};
use crate::mock_model::{Config, Handle};
use crate::temp_dir::TempDir;
use lgwks_std::json::json;
use std::ffi::OsString;

/// A one-task session set named `name` whose acceptance is `acceptance`.
fn session_set(name: &str, acceptance: &str) -> String {
    json!({
        "name": name,
        "tasks": [{
            "id": "session-path",
            "files": {"README.md": format!("# {name}\n")},
            "goal": "write session.txt",
            "path": "session",
            "write": ["session.txt"],
            "acceptance": [acceptance],
        }],
    })
    .to_string()
}

/// Runs `set` against the mock, whose session runs its edit, then a read-only
/// check, then the typed finish pair, and returns the run. The session works on
/// the host exactly as `larry acp` runs it, so the arm grants `model.command`
/// the way a Harbor task container's operator does.
fn run_session_task(state: &TempDir, set: &str, replies: &[&str]) -> TestResult<Ran> {
    let config = Config {
        contents: replies.iter().map(|reply| (*reply).to_owned()).collect(),
        ..Config::seeded(7)
    };
    crate::mock_model::run(config, |handle: &Handle| {
        let mut env = operator(state, handle.addr(), &FLASH, "3");
        env.push((
            "LARRY_POLICY",
            OsString::from("file.read,file.write,process.spawn,model.invoke,model.command"),
        ));
        let ran = score_arm(&env, set.as_bytes())?;
        assert_eq!(
            handle.served(),
            4,
            "the edit, the read-only check and the finish pair"
        );
        Ok::<_, Box<dyn std::error::Error>>(ran)
    })
}

/// The card's one result, proved to be the session path with `outcome`.
fn check_session_result(ran: &Ran, outcome: &str) -> TestResult {
    let card = ran.card()?;
    let result = card["results"]
        .as_array()
        .ok_or("results")?
        .first()
        .ok_or("one result")?;
    assert_eq!(result["id"], "session-path", "the one task");
    assert_eq!(result["path"], "session", "the result names its path");
    assert_eq!(result["outcome"], outcome, "the session scores {outcome}");
    Ok(())
}

/// Runs one session-task set through the mock and checks its card: the result
/// names the session path with `outcome`, every origin is removed, and the run
/// prints its card.
fn check_session_case(name: &str, acceptance: &str, replies: &[&str], outcome: &str) -> TestResult {
    let state = TempDir::new()?;
    let set = session_set(name, acceptance);
    let ran = run_session_task(&state, &set, replies)?;
    assert_eq!(ran.code, Some(0), "a run prints its card: {}", ran.stderr);
    check_session_result(&ran, outcome)?;
    assert_eq!(origins_left(&state)?, 0, "every origin removed");
    Ok(())
}

/// scored-by-acceptance (larry#217): a session task whose commands all exit 0 but
/// whose acceptance fails is rejected, and its card result names its path. Catches
/// a scorer that accepts a session for its commands alone without running the set's
/// verifier.
#[test]
fn a_session_task_whose_commands_pass_but_miss_the_acceptance_is_rejected() -> TestResult {
    let finish = r#"{"finish":{"summary":"ready for review","check":"true","artifacts":["README.md"],"state":["ready"],"servers":["none"]}}"#;
    check_session_case(
        "missed",
        "test -s session.txt",
        &[
            r#"{"command":"true"}"#,
            r#"{"command":"true"}"#,
            finish,
            finish,
        ],
        "rejected",
    )
}

/// scored-by-acceptance (larry#217): a session task whose commands pass and whose
/// acceptance passes is accepted, and its card result names its path. The acceptance
/// sleeps past the runner's cancel poll, so a runner that reports cancelled is
/// observed as rejected instead.
#[test]
fn a_session_task_whose_commands_and_acceptance_pass_is_accepted() -> TestResult {
    let finish = r#"{"finish":{"summary":"wrote session.txt","check":"test -s session.txt","artifacts":["session.txt"],"state":["ready"],"servers":["none"]}}"#;
    check_session_case(
        "kept",
        "sleep 2 && test -s session.txt",
        &[
            r#"{"command":"printf 'session\n' > session.txt"}"#,
            r#"{"command":"test -s session.txt"}"#,
            finish,
            finish,
        ],
        "accepted",
    )
}
