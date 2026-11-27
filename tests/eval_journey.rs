//! Journeys of `larry eval` (larry#87) as an operator scoring one arm lives them:
//! a matched set on stdin, the arm's model and prices in the environment, one
//! scorecard line on stdout. Every run drives the shipped binary against the
//! seeded mock model endpoint, so no test makes a paid model call.
//!
//! Source: the SWE-bench and Artificial Analysis method (one task set, every arm
//! scored on it, cost per solved task = total spend over solved tasks), the
//! OpenAI chat completions `usage` object, NIST/SEMATECH 7.2.6.2 (nearest-rank
//! percentiles) and the fault classes of forge#158 and larry#128. Requirement:
//! two arms over one set print comparable cards whose tokens and cost equal what
//! the gateway reported; a fat-fingered set or price spends nothing; a lying
//! gateway is scored as refusals while the arm keeps going; a kill -9 leaves
//! nothing the next run does not collect; the shipped set runs end to end.

#[path = "support/json_at.rs"]
mod json_at;
use json_at::At;

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/mock_model.rs"]
mod mock_model;

#[path = "support/eval_arm.rs"]
mod eval_arm;
use eval_arm::{Arm, CLAUDE, FLASH, Ran, TestResult};
use eval_arm::{charged_prompt_tokens, eval_entries, number, operator, origins_left, report};
use eval_arm::{rss_tool, score_arm, spawn_arm, task, timeless};

use lgwks_std::json::{self, Value, json};
use mock_model::{Config, Fault, Handle, hostile_message};
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

#[path = "support/temp_dir.rs"]
mod temp_dir;
use temp_dir::TempDir;

/// The matched set the release ships.
const SHIPPED_SET: &str = include_str!("../eval/matched-v1.json");

/// What one arm's run against one seeded mock produced.
struct Scored {
    /// The run.
    ran: Ran,
    /// Prompt tokens the gateway charged.
    charged: u64,
    /// The gateway's trace hash.
    trace: String,
}

/// Runs `arm` over `set` against a fresh mock on `seed`, checking what reached it.
fn arm_on_seed(seed: u64, arm: &Arm, set: &str, state: &Path) -> TestResult<Scored> {
    mock_model::run(Config::seeded(seed), |handle: &Handle| {
        let ran = score_arm(&operator(state, handle.addr(), arm, "1"), set.as_bytes())?;
        assert_eq!(handle.served(), 5, "seed {seed}: one completion per file");
        for authorization in handle.authorizations() {
            assert_eq!(
                authorization.as_deref(),
                Some("Bearer eval-key"),
                "the arm's key"
            );
        }
        for body in handle.requests() {
            let request = json::from_slice::<Value>(&body)?;
            assert_eq!(
                *request.at("/model")?,
                arm.model,
                "seed {seed}: the arm's model"
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(Scored {
            ran,
            charged: charged_prompt_tokens(handle)?,
            trace: handle.trace_hash(),
        })
    })
}

/// Checks one arm's card against what the gateway charged and the arm's prices.
fn check_card(seed: u64, arm: &Arm, scored: &Scored) -> TestResult {
    let card = scored.ran.card()?;
    assert_eq!(
        scored.ran.code,
        Some(0),
        "seed {seed}: a printed card exits 0"
    );
    assert_eq!(card["set"], "journey", "the set is named");
    assert_eq!(card["model"], arm.model, "the arm is named");
    let outcomes: Vec<&Value> = card["results"]
        .as_array()
        .ok_or("results")?
        .iter()
        .map(|done| &done["outcome"])
        .collect();
    let expected = [
        json!("accepted"),
        json!("accepted"),
        json!("rejected"),
        json!("accepted"),
    ];
    assert_eq!(outcomes, expected.iter().collect::<Vec<_>>(), "seed {seed}");
    let counts = (
        number(card, "accepted")?,
        number(card, "rejected")?,
        number(card, "refused")?,
    );
    assert_eq!(counts, (3, 1, 0), "seed {seed}: each task ends once");
    assert_eq!(
        card["cost_complete"], true,
        "every completion reported usage"
    );
    assert!(
        card.get("peak_rss_kib").is_some(),
        "the scorecard names peak RSS"
    );
    if cfg!(target_os = "linux") {
        assert!(
            card["peak_rss_kib"].as_u64().is_some(),
            "Linux exposes peak RSS in KiB"
        );
    }
    let cache = [&card["cache_complete"], &card["cached_tokens"]];
    assert_eq!(cache, [&json!(false), &json!(0)], "seed {seed}: cache keys");
    let prompt = number(card, "prompt_tokens")?;
    let completion = number(card, "completion_tokens")?;
    assert_eq!(
        prompt, scored.charged,
        "seed {seed}: prompt tokens are what the gateway charged"
    );
    assert!(
        (15..=60).contains(&completion),
        "seed {seed}: 3 to 12 completion tokens per file, got {completion}"
    );
    let cost = u128::from(prompt)
        .checked_mul(arm.micro.0)
        .and_then(|input| input.checked_add(u128::from(completion).checked_mul(arm.micro.1)?))
        .ok_or("cost overflow")?;
    assert_eq!(
        card["cost_picousd"],
        json!(cost.to_string()),
        "seed {seed}: cost is tokens times price"
    );
    let share = cost.checked_div(3).ok_or("no accepted task")?;
    assert_eq!(
        card["cost_per_accepted_task_picousd"],
        json!(share.to_string()),
        "seed {seed}: a rejected task's spend counts against the accepted ones"
    );
    Ok(())
}

/// Replies for the shipped set, with the session task's command and summary in order.
fn matched_session_config(files: usize, session_request: usize) -> TestResult<Config> {
    let mut contents = vec![String::from("generated\n"); files.saturating_add(1)];
    *contents
        .get_mut(session_request)
        .ok_or("session command reply")? =
        String::from(r#"{"command":"printf 'session\n' > session.txt"}"#);
    *contents
        .get_mut(session_request.saturating_add(1))
        .ok_or("session done reply")? = String::from(r#"{"done":"wrote session.txt"}"#);
    Ok(Config {
        contents,
        ..Config::seeded(7)
    })
}

/// Counts model requests that carry the session's command reply as an assistant message.
fn session_command_replies(handle: &Handle) -> usize {
    handle
        .requests()
        .into_iter()
        .filter_map(|body| json::from_slice::<Value>(&body).ok())
        .filter(|request| {
            request["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["role"] == "assistant"
                        && message["content"].as_str().is_some_and(|content| {
                            content.contains(r#"{"command":"printf 'session"#)
                        })
                })
            })
        })
        .count()
}

/// The shipped card's session result carries the real path and a successful outcome.
fn assert_session_result(card: &Value) -> TestResult {
    let result = card["results"]
        .as_array()
        .ok_or("results")?
        .iter()
        .find(|result| result["id"] == "session-path")
        .ok_or("session result")?;
    assert_eq!(result["path"], "session", "the result names its path");
    assert_eq!(
        result["outcome"], "accepted",
        "the session command completed successfully"
    );
    Ok(())
}

/// Two arms over one set, four seeds: each card's tokens equal what the gateway
/// charged, its cost equals tokens times the arm's price by the test's own
/// arithmetic, rejected runs are counted and paid for, every request carried the
/// arm's model and key, no origin is left, and a rerun of the same arm on the same
/// seed prints the same card and drives the same gateway trace. Catches a card
/// that prices an arm with the other's rates, drops a rejected task's spend,
/// counts a task twice, or leaks origins.
#[test]
fn two_arms_score_one_set_and_a_rerun_replays_the_same_card() -> TestResult {
    let set = json!({
        "name": "journey",
        "tasks": [
            task("one", 1, &[]),
            task("two-files", 2, &[]),
            task("fails", 1, &["false"]),
            task("three", 1, &[]),
        ],
    })
    .to_string();
    for seed in 0..4_u64 {
        let mut traces = Vec::new();
        let mut cards = Vec::new();
        // One state directory is one tenant: the prompt names its cache partition, so a
        // rerun that must replay the same gateway trace runs in the same directory.
        let state = TempDir::new()?;
        for arm in [&CLAUDE, &FLASH, &CLAUDE] {
            let scored = arm_on_seed(seed, arm, &set, &state)?;
            check_card(seed, arm, &scored)?;
            assert_eq!(
                origins_left(&state)?,
                0,
                "seed {seed}: every origin removed"
            );
            report(&format!("seed {seed}"), &scored.ran)?;
            cards.push(timeless(scored.ran.card()?)?);
            traces.push(scored.trace);
        }
        assert_eq!(
            traces.first(),
            traces.get(2),
            "seed {seed}: the rerun drives the same gateway trace"
        );
        assert_eq!(
            cards.first(),
            cards.get(2),
            "seed {seed}: the rerun prints the same card"
        );
        assert_ne!(
            cards.first(),
            cards.get(1),
            "seed {seed}: two arms print two cards"
        );
    }
    Ok(())
}

/// One fat-fingered run: its environment, its set and the words stderr must carry.
type Mistake = (Vec<(&'static str, OsString)>, String, &'static str);

/// `base` with `name` replaced by `value`, or removed when `value` is `None`.
fn with(
    base: &[(&'static str, OsString)],
    name: &'static str,
    value: Option<&str>,
) -> Vec<(&'static str, OsString)> {
    let mut env: Vec<(&'static str, OsString)> = base
        .iter()
        .filter(|entry| entry.0 != name)
        .cloned()
        .collect();
    if let Some(text) = value {
        env.push((name, OsString::from(text)));
    }
    env
}

/// fat-finger: a price written as `1e3`, `1,5` or with seven decimals, a missing
/// price, a set task with no acceptance command, a set that is not JSON and an unset
/// key each exit 1 with a message naming the mistake, and the gateway serves nothing.
/// Catches an eval that guesses a price, accepts a task no command checks, or spends
/// on half a set before refusing the rest.
#[test]
fn a_fat_fingered_price_or_set_spends_nothing() -> TestResult {
    let good = json!({"name": "s", "tasks": [task("one", 1, &[])]}).to_string();
    let half_bad = json!({"name": "s", "tasks": [
        task("one", 1, &[]),
        {"id": "two", "files": {"a": ""}, "goal": "g", "write": ["b"], "acceptance": []},
    ]})
    .to_string();
    let state = TempDir::new()?;
    mock_model::run(Config::seeded(11), |handle: &Handle| {
        let base = operator(&state, handle.addr(), &CLAUDE, "1");
        let cases: [Mistake; 7] = [
            (
                with(&base, "LARRY_EVAL_PRICE_IN", Some("1e3")),
                good.clone(),
                "LARRY_EVAL_PRICE_IN is not a plain decimal",
            ),
            (
                with(&base, "LARRY_EVAL_PRICE_OUT", Some("1,5")),
                good.clone(),
                "LARRY_EVAL_PRICE_OUT is not a plain decimal",
            ),
            (
                with(&base, "LARRY_EVAL_PRICE_IN", Some("0.0000001")),
                good.clone(),
                "more than 6 fraction digits",
            ),
            (
                with(&base, "LARRY_EVAL_PRICE_OUT", None),
                good.clone(),
                "LARRY_EVAL_PRICE_OUT is unset",
            ),
            (
                base.clone(),
                half_bad.clone(),
                "task 1 carries 0 acceptance commands",
            ),
            (
                base.clone(),
                String::from("{\"name\":"),
                "the task set is not JSON",
            ),
            (
                with(&base, "LARRY_MODEL_KEY", None),
                good.clone(),
                "LARRY_MODEL_KEY",
            ),
        ];
        for (env, set, named) in cases {
            let ran = score_arm(&env, set.as_bytes())?;
            assert_eq!(ran.code, Some(1), "{named}: refused with exit 1");
            assert!(ran.card.is_none(), "{named}: no scorecard");
            assert!(
                ran.stderr.contains(named),
                "{named}: stderr names it: {}",
                ran.stderr
            );
        }
        assert_eq!(handle.served(), 0, "nothing reached the gateway");
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    assert_eq!(origins_left(&state)?, 0, "no origin was made");
    Ok(())
}

/// Bytes of padding after a valid set: past every pipe buffer (64 KiB on Linux), so the write
/// is still going when a refusing child exits without having read any of it.
const PIPE_OVERFLOW: usize = 262_144;

/// larry#444: `larry eval` refuses an unset price before it reads stdin, so a set larger than
/// the pipe can never be written whole. The broken pipe is the child having stopped reading:
/// the harness takes its status and stderr and the run is the refusal, never an I/O error.
#[test]
fn a_set_larger_than_the_pipe_to_a_refusing_eval_is_a_refusal() -> TestResult {
    let mut set = json!({"name": "s", "tasks": [task("one", 1, &[])]}).to_string();
    set.push_str(&" ".repeat(PIPE_OVERFLOW));
    let state = TempDir::new()?;
    mock_model::run(Config::seeded(12), |handle: &Handle| {
        let base = operator(&state, handle.addr(), &CLAUDE, "1");
        let env = with(&base, "LARRY_EVAL_PRICE_OUT", None);
        let ran = score_arm(&env, set.as_bytes())?;
        assert_eq!(ran.code, Some(1), "refused with exit 1");
        assert!(ran.card.is_none(), "no scorecard");
        assert!(
            ran.stderr.contains("LARRY_EVAL_PRICE_OUT is unset"),
            "stderr names it: {}",
            ran.stderr
        );
        assert_eq!(handle.served(), 0, "nothing reached the gateway");
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    assert_eq!(origins_left(&state)?, 0, "no origin was made");
    Ok(())
}

/// The gateway's lie for each task in order, and the ending it must score.
const LIES: [(Option<Fault>, &str); 11] = [
    (Some(Fault::Delay), "accepted"),
    (
        Some(Fault::RateLimit {
            retry_after_secs: 1,
        }),
        "refused",
    ),
    (Some(Fault::MalformedJson), "refused"),
    (Some(Fault::TruncatedBody), "refused"),
    (Some(Fault::ServerError { status: 503 }), "refused"),
    (Some(Fault::PaddedDone), "accepted"),
    (Some(Fault::HostileRefusal { status: 401 }), "refused"),
    // larry#396: a reply cut at the token limit, stopped by a filter or empty.
    (Some(Fault::Finish { reason: "length" }), "refused"),
    (
        Some(Fault::Finish {
            reason: "content_filter",
        }),
        "refused",
    ),
    (Some(Fault::EmptyContent), "refused"),
    (None, "accepted"),
];

/// Checks the liars' card: each lie scored as its task's ending, bounded clean reasons.
fn check_lies(ran: &Ran) -> TestResult {
    let card = ran.card()?;
    assert_eq!(
        ran.code,
        Some(0),
        "lies are scored, not fatal: {}",
        ran.stderr
    );
    let results = card["results"].as_array().ok_or("results")?;
    assert_eq!(results.len(), LIES.len(), "the arm ran to the last task");
    for (index, (result, lie)) in results.iter().zip(LIES.iter()).enumerate() {
        assert_eq!(result["outcome"], lie.1, "task {index} under {:?}", lie.0);
        let reason = result["reason"].as_str();
        assert_eq!(
            reason.is_some(),
            lie.1 == "refused",
            "task {index}: a refusal says why"
        );
        if let Some(text) = reason {
            assert!(
                !text.contains('\u{1b}') && !text.contains('\u{7}'),
                "task {index}: no escape bytes: {text:?}"
            );
            assert!(
                text.len() < hostile_message().len(),
                "task {index}: the reason is bounded"
            );
        }
    }
    let counts = (number(card, "accepted")?, number(card, "refused")?);
    assert_eq!(counts, (3, 8), "every lie counted");
    assert_eq!(
        card["cost_complete"], false,
        "a refusal leaves its cost unknown, and says so"
    );
    Ok(())
}

/// liar-upstream: a slow reply, a 429, a non-JSON 200, a truncated body, a 503, a
/// padded `[DONE]` stream, a hostile 401, and a reply cut by `length`, stopped by
/// `content_filter` or empty (larry#396) are each scored as that task's ending,
/// the arm runs on to the last task, the card says its cost is incomplete, no
/// refusal reason carries the gateway's escape bytes, and the same lies replay to
/// the same trace and card. Catches an eval that aborts the arm on one bad reply,
/// scores a refusal as a rejection the arm paid for, or prints a terminal attack.
#[test]
fn a_lying_gateway_is_scored_as_refusals_and_the_arm_keeps_going() -> TestResult {
    let tasks: Vec<Value> = (0..LIES.len())
        .map(|index| task(&format!("t{index}"), 1, &[]))
        .collect();
    let set = json!({"name": "liars", "tasks": tasks}).to_string();
    let mut replays = Vec::new();
    // The same tenant both times: the prompt names its cache partition (larry#230).
    let state = TempDir::new()?;
    for _ in 0..2 {
        let config = Config {
            script: LIES.iter().map(|lie| lie.0).collect(),
            ..Config::seeded(23)
        };
        let (ran, trace) = mock_model::run(config, |handle: &Handle| {
            handle.release_delayed();
            let ran = score_arm(
                &operator(&state, handle.addr(), &FLASH, "1"),
                set.as_bytes(),
            )?;
            assert_eq!(
                handle.served(),
                u64::try_from(LIES.len())?,
                "one request per task at one attempt"
            );
            Ok::<_, Box<dyn std::error::Error>>((ran, handle.trace_hash()))
        })?;
        check_lies(&ran)?;
        assert_eq!(
            eval_entries(&state)?,
            0,
            "every origin and its lock removed"
        );
        replays.push((trace, timeless(ran.card()?)?));
    }
    assert_eq!(
        replays.first(),
        replays.get(1),
        "the same lies replay to the same trace and card"
    );
    Ok(())
}

/// broken-machine (larry#399): the third task names a file whose parent folder is a file, so
/// the machine cannot write it and the arm stops there before its model call. The card still
/// prints the two scored outcomes with the tokens the gateway charged, says `complete: false`,
/// names the task and its typed cause, and the run exits 1. Catches an eval that drops every
/// paid outcome on one infrastructure fault, prints a partial card as complete, or spends past
/// a fault the machine owns.
#[test]
fn an_infrastructure_fault_stops_the_arm_and_prints_the_outcomes_before_it() -> TestResult {
    let broken = json!({
        "id": "broken",
        "files": {"README.md": "# broken\n", "out": "a file where a folder must be\n"},
        "goal": "write broken",
        "write": ["out/0.txt"],
        "acceptance": ["test -s out/0.txt"],
    });
    let tasks = [
        task("one", 1, &[]),
        task("two-files", 2, &[]),
        broken,
        task("never", 1, &[]),
    ];
    let set = json!({"name": "stopped", "tasks": tasks});
    let state = TempDir::new()?;
    let (ran, charged) = mock_model::run(Config::seeded(41), |handle: &Handle| {
        let env = operator(&state, handle.addr(), &CLAUDE, "1");
        let ran = score_arm(&env, set.to_string().as_bytes())?;
        assert_eq!(
            handle.served(),
            3,
            "the two scored tasks' calls, none after"
        );
        Ok::<_, Box<dyn std::error::Error>>((ran, charged_prompt_tokens(handle)?))
    })?;
    assert_eq!(ran.code, Some(1), "a stopped arm exits 1: {}", ran.stderr);
    let card = ran.card()?;
    assert_eq!(card["complete"], false, "the card says it is partial");
    let ids: Vec<&Value> = card["results"]
        .as_array()
        .ok_or("results")?
        .iter()
        .map(|done| &done["id"])
        .collect();
    assert_eq!(
        ids,
        [&json!("one"), &json!("two-files")],
        "the scored outcomes"
    );
    assert_eq!(
        number(card, "accepted")?,
        2,
        "both scored tasks were accepted"
    );
    assert_eq!(
        number(card, "prompt_tokens")?,
        charged,
        "the paid tokens stay on the card"
    );
    assert_eq!(
        *card.at("/stopped/task")?,
        "broken",
        "the stopping task is named"
    );
    let cause = card.at("/stopped/cause")?.as_str().ok_or("stopped cause")?;
    assert!(cause.contains("out/0.txt"), "the typed cause: {cause}");
    assert!(
        ran.stderr.contains("broken"),
        "stderr names it: {}",
        ran.stderr
    );
    assert_eq!(
        eval_entries(&state)?,
        0,
        "the stopped origin and its lock removed"
    );
    report("stopped arm", &ran)
}

/// pulled-plug: `larry eval` killed with SIGKILL while its first completion is held
/// leaves its origin behind (no destructor runs); the next run on the same state
/// directory collects it, scores its own set and leaves nothing. Catches origins
/// that pile up under the state directory after every crash.
#[test]
fn a_killed_eval_leaves_nothing_the_next_run_does_not_collect() -> TestResult {
    let set = json!({"name": "plug", "tasks": [task("held", 1, &[])]}).to_string();
    let state = TempDir::new()?;
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(31)
    };
    mock_model::run(config, |handle: &Handle| {
        let env = operator(&state, handle.addr(), &CLAUDE, "1");
        let mut child = spawn_arm(&env, false)?;
        let mut stdin = child.stdin.take().ok_or("child stdin was not piped")?;
        stdin.write_all(set.as_bytes())?;
        drop(stdin);
        while handle.served() == 0 {
            if child.try_wait()?.is_some() {
                return Err("eval exited before its first completion".into());
            }
            std::thread::yield_now();
        }
        assert_eq!(origins_left(&state)?, 1, "the held task's origin exists");
        child.kill()?;
        child.wait()?;
        handle.release_delayed();
        assert_eq!(origins_left(&state)?, 1, "SIGKILL ran no destructor");
        let ran = score_arm(&env, set.as_bytes())?;
        let card = ran.card()?;
        assert_eq!(
            card["accepted"], 1,
            "the next run scores its own set: {}",
            ran.stderr
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    assert_eq!(
        eval_entries(&state)?,
        0,
        "the dead run's origin and its lock file were collected"
    );
    Ok(())
}

/// The shipped matched set, against the mock: every task is sealed (the mock's
/// replies are not solutions, so most are rejected), ids come back in the set's
/// order, every completion reported usage, and the card, latency and peak RSS are
/// printed. Catches a shipped set the parser refuses, a task whose origin cannot
/// be committed, and an acceptance command the executor cannot start.
#[test]
fn the_shipped_matched_set_runs_end_to_end_against_the_mock() -> TestResult {
    let shipped = json::from_slice::<Value>(SHIPPED_SET.as_bytes())?;
    let listed = shipped.at("/tasks")?.as_array().ok_or("tasks")?;
    let ids: Vec<&Value> = listed.iter().map(|one| &one["id"]).collect();
    let files: usize = listed
        .iter()
        .filter_map(|one| one["write"].as_array().map(Vec::len))
        .sum();
    assert_eq!(ids.len(), 14, "the shipped set holds fourteen tasks");
    assert!(
        ids.iter().any(|id| *id == "session-path"),
        "the set covers ACP"
    );
    assert!(
        listed.iter().any(|task| {
            task["id"] == "session-path"
                && task["path"] == "session"
                && task["goal"]
                    .as_str()
                    .is_some_and(|goal| goal.contains("larry acp"))
        }),
        "the ACP task names the shipped session path"
    );
    assert!(
        ids.iter().any(|id| *id == "multi-file"),
        "the set covers multiple files"
    );
    let session_index = listed
        .iter()
        .position(|task| task["id"] == "session-path")
        .ok_or("session task")?;
    let session_request = listed
        .iter()
        .take(session_index)
        .filter_map(|task| task["write"].as_array().map(Vec::len))
        .sum::<usize>();
    let config = matched_session_config(files, session_request)?;
    let state = TempDir::new()?;
    let ran = mock_model::run(config, |handle: &Handle| {
        // The session task runs its model-chosen commands on the host, exactly as
        // `larry acp` runs them, so this arm grants `model.command` the way a
        // Harbor task container's operator does; brief tasks need only four scopes.
        // The string repeats `support/acp_session.rs`'s SESSION_POLICY, which stays
        // the canonical home; a drift fails the served-count assertion below.
        let mut env = operator(&state, handle.addr(), &FLASH, "3");
        env.push((
            "LARRY_POLICY",
            OsString::from("file.read,file.write,process.spawn,model.invoke,model.command"),
        ));
        let ran = score_arm(&env, SHIPPED_SET.as_bytes())?;
        assert_eq!(
            handle.served(),
            u64::try_from(files.saturating_add(1))?,
            "one completion per written file plus the session done reply"
        );
        assert_eq!(
            session_command_replies(handle),
            1,
            "the mock saw the session command reply"
        );
        Ok::<_, Box<dyn std::error::Error>>(ran)
    })?;
    let card = ran.card()?;
    assert_eq!(card["set"], "matched-v1", "the shipped set names itself");
    let returned: Vec<&Value> = card["results"]
        .as_array()
        .ok_or("results")?
        .iter()
        .map(|done| &done["id"])
        .collect();
    assert_eq!(returned, ids, "every task, in the set's order");
    assert_session_result(card)?;
    assert_eq!(
        number(card, "refused")?,
        0,
        "a clean gateway refuses nothing"
    );
    let sealed = number(card, "accepted")?.checked_add(number(card, "rejected")?);
    assert_eq!(sealed, Some(14), "every task sealed");
    assert_eq!(
        card["cost_complete"], true,
        "every completion reported usage"
    );
    assert_eq!(origins_left(&state)?, 0, "every origin removed");
    if rss_tool().is_some() {
        assert!(
            ran.peak_rss_kib().is_some(),
            "the RSS tool reported a peak: {}",
            ran.stderr
        );
    }
    report("matched-v1 (mock)", &ran)
}
