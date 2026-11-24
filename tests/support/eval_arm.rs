//! The harness of the `larry eval` journeys (larry#87): one arm's environment, the shipped
//! binary run with a set on stdin, the card and peak RSS it printed, and what it left under
//! the state directory. The journeys in `tests/eval_journey.rs` hold the requirements; this
//! file only drives and reads the binary (larry#399 split it out of the journey file).

use super::mock_model::Handle;
use lgwks_std::json::{self, Value, json};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

/// What every journey returns.
pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The key every arm sends; the mock records it.
const KEY: &str = "eval-key";

/// One arm: the model it pins and its dollars per million tokens each way.
pub(super) struct Arm {
    /// `LARRY_MODEL`.
    pub(super) model: &'static str,
    /// `LARRY_EVAL_PRICE_IN`, as typed.
    price_in: &'static str,
    /// `LARRY_EVAL_PRICE_OUT`, as typed.
    price_out: &'static str,
    /// The same prices in micro-dollars per million tokens: the test's own reading.
    pub(super) micro: (u128, u128),
}

/// Claude alone, at its list price.
pub(super) const CLAUDE: Arm = Arm {
    model: "claude-alone",
    price_in: "3",
    price_out: "15",
    micro: (3_000_000, 15_000_000),
};

/// Claude plus Flash: Flash writes the files.
pub(super) const FLASH: Arm = Arm {
    model: "claude-plus-flash",
    price_in: "0.30",
    price_out: "1.2",
    micro: (300_000, 1_200_000),
};

/// The operator's environment for `arm` against `gateway`, `attempts` per completion.
pub(super) fn operator(
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
pub(super) fn rss_tool() -> Option<(&'static str, &'static str)> {
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
pub(super) fn spawn_arm(env: &[(&'static str, OsString)], timed: bool) -> TestResult<Child> {
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
pub(super) struct Ran {
    /// Its exit code.
    pub(super) code: Option<i32>,
    /// The scorecard, when one was printed.
    pub(super) card: Option<Value>,
    /// Its standard error, the RSS tool's report included.
    pub(super) stderr: String,
}

impl Ran {
    /// The peak RSS in KiB the tool reported: bytes on macOS, KiB on Linux.
    pub(super) fn peak_rss_kib(&self) -> Option<u64> {
        let line = self.stderr.lines().find(|line| {
            line.to_ascii_lowercase()
                .contains("maximum resident set size")
        })?;
        let digits: String = line.chars().filter(char::is_ascii_digit).collect();
        // A report line with no number is no reading, the same as no line at all.
        let value: u64 = match digits.parse() {
            Ok(value) => value,
            Err(_no_number) => return None,
        };
        if cfg!(target_os = "macos") {
            Some(value.div_ceil(1024))
        } else {
            Some(value)
        }
    }

    /// The scorecard, or the failure that printed none.
    pub(super) fn card(&self) -> TestResult<&Value> {
        self.card
            .as_ref()
            .ok_or_else(|| format!("no scorecard (exit {:?}): {}", self.code, self.stderr).into())
    }
}

/// Runs `larry eval` with `set` on stdin to its end.
pub(super) fn score_arm(env: &[(&'static str, OsString)], set: &[u8]) -> TestResult<Ran> {
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
pub(super) fn task(id: &str, count: usize, extra: &[&str]) -> Value {
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
pub(super) fn number(card: &Value, key: &str) -> TestResult<u64> {
    card[key]
        .as_u64()
        .ok_or_else(|| format!("card {key} is not a number: {card}").into())
}

/// The prompt tokens the mock charged for every request it recorded: ceil(body / 4).
pub(super) fn charged_prompt_tokens(handle: &Handle) -> TestResult<u64> {
    let mut total: u64 = 0;
    for body in handle.requests() {
        let tokens = u64::try_from(body.len().div_ceil(4))?;
        total = total.checked_add(tokens).ok_or("token overflow")?;
    }
    Ok(total)
}

/// The card with every wall-time field cleared: what a replay must match.
pub(super) fn timeless(card: &Value) -> TestResult<Value> {
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

/// The entries of `<state>/eval`, origins and their sibling lock files alike, and
/// whether each is a directory: a finished run leaves none (larry#87, larry#214).
fn eval_listing(state: &Path) -> TestResult<Vec<bool>> {
    match fs::read_dir(state.join("eval")) {
        Ok(entries) => entries
            .map(|entry| Ok(entry?.file_type()?.is_dir()))
            .collect(),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(cause) => Err(cause.into()),
    }
}

/// How many entries `<state>/eval` holds, lock files included.
pub(super) fn eval_entries(state: &Path) -> TestResult<usize> {
    Ok(eval_listing(state)?.len())
}

/// How many origin directories `<state>/eval` holds; a lock file is not an origin.
pub(super) fn origins_left(state: &Path) -> TestResult<usize> {
    Ok(eval_listing(state)?.into_iter().filter(|dir| *dir).count())
}

/// Writes one card's line to the run log: acceptance, cost, latency and RSS.
pub(super) fn report(label: &str, ran: &Ran) -> TestResult {
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
