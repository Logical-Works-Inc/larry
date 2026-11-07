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

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/mock_model.rs"]
mod mock_model;

use lgwks_std::json::{self, Value, json};
use mock_model::{Config, Fault, Handle, hostile_message};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// What every journey returns.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The matched set the release ships.
const SHIPPED_SET: &str = include_str!("../eval/matched-v1.json");

/// The key every arm sends; the mock records it.
const KEY: &str = "eval-key";

/// A fresh directory under the system temp dir, removed when dropped.
struct Scratch {
    /// Where it lives.
    path: PathBuf,
}

impl Scratch {
    /// An empty directory no other journey in this process uses.
    fn new() -> io::Result<Scratch> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let run = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("larry-eval-{}-{run}", std::process::id()));
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => return Err(cause),
        }
        fs::create_dir_all(&path)?;
        Ok(Scratch { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort: a leftover temp dir never fails a journey.
        drop(fs::remove_dir_all(&self.path));
    }
}

/// One arm: the model it pins and its dollars per million tokens each way.
struct Arm {
    /// `LARRY_MODEL`.
    model: &'static str,
    /// `LARRY_EVAL_PRICE_IN`, as typed.
    price_in: &'static str,
    /// `LARRY_EVAL_PRICE_OUT`, as typed.
    price_out: &'static str,
    /// The same prices in micro-dollars per million tokens: the test's own reading.
    micro: (u128, u128),
}

/// Claude alone, at its list price.
const CLAUDE: Arm = Arm {
    model: "claude-alone",
    price_in: "3",
    price_out: "15",
    micro: (3_000_000, 15_000_000),
};

/// Claude plus Flash: Flash writes the files.
const FLASH: Arm = Arm {
    model: "claude-plus-flash",
    price_in: "0.30",
    price_out: "1.2",
    micro: (300_000, 1_200_000),
};

/// The operator's environment for `arm` against `gateway`, `attempts` per completion.
fn operator(
    state: &Path,
    gateway: SocketAddr,
    arm: &Arm,
    attempts: &str,
) -> Vec<(&'static str, OsString)> {
    vec![
        ("LARRY_STATE_DIR", state.as_os_str().to_owned()),
        (
            "LARRY_MODEL_URL",
            OsString::from(format!("http://{gateway}/v1")),
        ),
        ("LARRY_MODEL_KEY", OsString::from(KEY)),
        ("LARRY_MODEL_BACKOFF_MS", OsString::from("0")),
        ("LARRY_MODEL_ATTEMPTS", OsString::from(attempts)),
        ("LARRY_MODEL", OsString::from(arm.model)),
        ("LARRY_EVAL_PRICE_IN", OsString::from(arm.price_in)),
        ("LARRY_EVAL_PRICE_OUT", OsString::from(arm.price_out)),
    ]
}

/// The system tool that reports a child's peak RSS, and the flag that asks for it.
fn rss_tool() -> Option<(&'static str, &'static str)> {
    let flag = if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    };
    Path::new("/usr/bin/time")
        .is_file()
        .then_some(("/usr/bin/time", flag))
}

/// Starts `larry eval` under `env` with only PATH besides, under `rss_tool` when `timed`
/// and present; untimed, the child is `larry` itself, so a kill reaches it.
fn spawn_arm(env: &[(&'static str, OsString)], timed: bool) -> TestResult<Child> {
    let binary = env!("CARGO_BIN_EXE_larry");
    let mut command = match rss_tool().filter(|_| timed) {
        Some((tool, flag)) => {
            let mut timed = Command::new(tool);
            timed.arg(flag).arg(binary);
            timed
        }
        None => Command::new(binary),
    };
    let child = command
        .arg("eval")
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .envs(env.iter().map(|entry| (entry.0, entry.1.as_os_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    Ok(child)
}

/// What one `larry eval` printed.
struct Ran {
    /// Its exit code.
    code: Option<i32>,
    /// The scorecard, when one was printed.
    card: Option<Value>,
    /// Its standard error, the RSS tool's report included.
    stderr: String,
}

impl Ran {
    /// The peak RSS in KiB the tool reported: bytes on macOS, KiB on Linux.
    fn peak_rss_kib(&self) -> Option<u64> {
        let line = self.stderr.lines().find(|line| {
            line.to_ascii_lowercase()
                .contains("maximum resident set size")
        })?;
        let digits: String = line.chars().filter(char::is_ascii_digit).collect();
        let value: u64 = digits.parse().ok()?;
        if cfg!(target_os = "macos") {
            Some(value.div_ceil(1024))
        } else {
            Some(value)
        }
    }

    /// The scorecard, or the failure that printed none.
    fn card(&self) -> TestResult<&Value> {
        self.card
            .as_ref()
            .ok_or_else(|| format!("no scorecard (exit {:?}): {}", self.code, self.stderr).into())
    }
}

/// Runs `larry eval` with `set` on stdin to its end.
fn score_arm(env: &[(&'static str, OsString)], set: &[u8]) -> TestResult<Ran> {
    let mut child = spawn_arm(env, true)?;
    let mut stdin = child.stdin.take().ok_or("child stdin was not piped")?;
    stdin.write_all(set)?;
    drop(stdin);
    let output: Output = child.wait_with_output()?;
    let card = if output.stdout.is_empty() {
        None
    } else {
        Some(json::from_slice::<Value>(&output.stdout)?)
    };
    Ok(Ran {
        code: output.status.code(),
        card,
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// A task whose `count` files the model writes, accepted when each is non-empty.
fn task(id: &str, count: usize, extra: &[&str]) -> Value {
    let write: Vec<String> = (0..count).map(|slot| format!("out/{slot}.txt")).collect();
    let mut acceptance: Vec<String> = write.iter().map(|path| format!("test -s {path}")).collect();
    acceptance.extend(extra.iter().map(|&command| command.to_owned()));
    json!({
        "id": id,
        "files": { "README.md": format!("# {id}\n") },
        "goal": format!("write {id}"),
        "write": write,
        "acceptance": acceptance,
    })
}

/// The scorecard field `key` as a whole number.
fn number(card: &Value, key: &str) -> TestResult<u64> {
    card[key]
        .as_u64()
        .ok_or_else(|| format!("card {key} is not a number: {card}").into())
}

/// The prompt tokens the mock charged for every request it recorded: ceil(body / 4).
fn charged_prompt_tokens(handle: &Handle) -> TestResult<u64> {
    let mut total: u64 = 0;
    for body in handle.requests() {
        let tokens = u64::try_from(body.len().div_ceil(4))?;
        total = total.checked_add(tokens).ok_or("token overflow")?;
    }
    Ok(total)
}

/// The card with every wall-time field cleared: what a replay must match.
fn timeless(card: &Value) -> TestResult<Value> {
    let mut stripped = card.clone();
    let top = stripped.as_object_mut().ok_or("card is an object")?;
    for key in ["p50_micros", "p95_micros", "p99_micros"] {
        top.insert(key, Value::Null);
    }
    let results = top
        .get_mut("results")
        .and_then(Value::as_array_mut)
        .ok_or("card results")?;
    for result in results {
        result
            .as_object_mut()
            .ok_or("result object")?
            .insert("micros", Value::Null);
    }
    Ok(stripped)
}

/// How many entries `<state>/eval` holds: every run removes its own origins.
fn origins_left(state: &Path) -> TestResult<usize> {
    match fs::read_dir(state.join("eval")) {
        Ok(entries) => Ok(entries.count()),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(cause) => Err(cause.into()),
    }
}

/// Writes one card's line to the run log: acceptance, cost, latency and RSS.
fn report(label: &str, ran: &Ran) -> TestResult {
    let card = ran.card()?;
    let rss = match ran.peak_rss_kib() {
        Some(kib) => format!("{kib} KiB"),
        None => String::from("NOT RUN (no /usr/bin/time)"),
    };
    let line = format!(
        "{label}: model {} accepted {}/{} cost {} USD, {} USD per accepted task, \
         task p50 {} us p95 {} us p99 {} us, peak RSS {rss}\n",
        card["model"],
        card["accepted"],
        card["tasks"],
        card["cost_usd"],
        card["cost_per_accepted_task_usd"],
        card["p50_micros"],
        card["p95_micros"],
        card["p99_micros"],
    );
    io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

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
            assert_eq!(request["model"], arm.model, "seed {seed}: the arm's model");
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
        for arm in [&CLAUDE, &FLASH, &CLAUDE] {
            let state = Scratch::new()?;
            let scored = arm_on_seed(seed, arm, &set, &state.path)?;
            check_card(seed, arm, &scored)?;
            assert_eq!(
                origins_left(&state.path)?,
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
    let state = Scratch::new()?;
    mock_model::run(Config::seeded(11), |handle: &Handle| {
        let base = operator(&state.path, handle.addr(), &CLAUDE, "1");
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
    assert_eq!(origins_left(&state.path)?, 0, "no origin was made");
    Ok(())
}

/// The gateway's lie for each task in order, and the ending it must score.
const LIES: [(Option<Fault>, &str); 8] = [
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
    assert_eq!(counts, (3, 5), "every lie counted");
    assert_eq!(
        card["cost_complete"], false,
        "a refusal leaves its cost unknown, and says so"
    );
    Ok(())
}

/// liar-upstream: a slow reply, a 429, a non-JSON 200, a truncated body, a 503, a
/// padded `[DONE]` stream and a hostile 401 are each scored as that task's ending,
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
    for _ in 0..2 {
        let state = Scratch::new()?;
        let config = Config {
            script: LIES.iter().map(|lie| lie.0).collect(),
            ..Config::seeded(23)
        };
        let (ran, trace) = mock_model::run(config, |handle: &Handle| {
            handle.release_delayed();
            let ran = score_arm(
                &operator(&state.path, handle.addr(), &FLASH, "1"),
                set.as_bytes(),
            )?;
            assert_eq!(handle.served(), 8, "one request per task at one attempt");
            Ok::<_, Box<dyn std::error::Error>>((ran, handle.trace_hash()))
        })?;
        check_lies(&ran)?;
        assert_eq!(origins_left(&state.path)?, 0, "every origin removed");
        replays.push((trace, timeless(ran.card()?)?));
    }
    assert_eq!(
        replays.first(),
        replays.get(1),
        "the same lies replay to the same trace and card"
    );
    Ok(())
}

/// pulled-plug: `larry eval` killed with SIGKILL while its first completion is held
/// leaves its origin behind (no destructor runs); the next run on the same state
/// directory collects it, scores its own set and leaves nothing. Catches origins
/// that pile up under the state directory after every crash.
#[test]
fn a_killed_eval_leaves_nothing_the_next_run_does_not_collect() -> TestResult {
    let set = json!({"name": "plug", "tasks": [task("held", 1, &[])]}).to_string();
    let state = Scratch::new()?;
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(31)
    };
    mock_model::run(config, |handle: &Handle| {
        let env = operator(&state.path, handle.addr(), &CLAUDE, "1");
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
        assert_eq!(
            origins_left(&state.path)?,
            1,
            "the held task's origin exists"
        );
        child.kill()?;
        child.wait()?;
        handle.release_delayed();
        assert_eq!(origins_left(&state.path)?, 1, "SIGKILL ran no destructor");
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
        origins_left(&state.path)?,
        0,
        "the dead run's origin was collected"
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
    let listed = shipped["tasks"].as_array().ok_or("tasks")?;
    let ids: Vec<&Value> = listed.iter().map(|one| &one["id"]).collect();
    let files: usize = listed
        .iter()
        .filter_map(|one| one["write"].as_array().map(Vec::len))
        .sum();
    assert_eq!(ids.len(), 12, "the shipped set holds twelve tasks");
    let state = Scratch::new()?;
    let ran = mock_model::run(Config::seeded(7), |handle: &Handle| {
        let ran = score_arm(
            &operator(&state.path, handle.addr(), &FLASH, "3"),
            SHIPPED_SET.as_bytes(),
        )?;
        assert_eq!(
            handle.served(),
            u64::try_from(files)?,
            "one completion per written file"
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
    assert_eq!(
        number(card, "refused")?,
        0,
        "a clean gateway refuses nothing"
    );
    let sealed = number(card, "accepted")?.checked_add(number(card, "rejected")?);
    assert_eq!(sealed, Some(12), "every task sealed");
    assert_eq!(
        card["cost_complete"], true,
        "every completion reported usage"
    );
    assert_eq!(origins_left(&state.path)?, 0, "every origin removed");
    if rss_tool().is_some() {
        assert!(
            ran.peak_rss_kib().is_some(),
            "the RSS tool reported a peak: {}",
            ran.stderr
        );
    }
    report("matched-v1 (mock)", &ran)
}
