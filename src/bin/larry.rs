//! Native AppCUI launcher, `larry mcp`, the MCP stdio server (larry#84), `larry acp`,
//! the Agent Client Protocol agent Harbor runs Terminal-Bench through (larry#93),
//! `larry model`, one completion through the configured gateway (larry#83),
//! `larry exec`, the task-document executor (larry#85), and `larry verify` and
//! `larry key`, the receipt check and the key it trusts (larry#86), and
//! `larry route`, the tenant's model routes in the state directory (larry#201), and
//! `larry eval`, one priced arm over the matched task set (larry#87), and
//! `larry governance` and `larry tracker`, the document gate (larry#53, ADR 0010), and
//! `larry mascot`, the pixel raccoon's SVG, preview and pet package (larry#152, ADR 0014), and
//! `larry serve`, the HTTP host on lgwks_bot's app edge (larry#88, ADR 0021).
//! The launcher paints the fixture surface and exits when the operator
//! quits; it performs no backend or workspace effect.
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use larry_demo::eval::{EvalError, MAX_SET_BYTES, PriceError, Prices, Scorecard};
use larry_demo::executor::routes::{RouteCliError, run as route_run};
use larry_demo::executor::{
    ExecError, MAX_REPORT_BYTES, Report, Settings, Task, Verdict, execute, state_public_key,
    trusted_key, verify,
};
use larry_demo::governance;
use larry_demo::model::{Completion, Message, ModelClient, ModelConfig, ModelError, Role};

/// Arguments read from the command line; `larry route add` takes the most: a
/// subcommand, an id and eight flags with their values.
const MAX_ARGUMENTS: usize = 32;

/// Exit status of a report that was produced and sealed but not accepted: a
/// failed, timed-out or denied run. 0 means accepted; 1 means refused, no report.
const REJECTED: u8 = 2;

/// Prompt bytes `larry model` reads: one past the client's 1 MiB request
/// bound, so an oversize prompt reaches the client's typed refusal.
const PROMPT_LIMIT: u64 = 1_048_577;

/// Task bytes `larry exec` reads: one past the executor's 64 KiB task bound,
/// so an oversize task reaches the parser's typed `TooLarge` refusal.
const TASK_LIMIT: u64 = 65_537;

/// Report bytes `larry verify` reads: one past the verifier's bound, so an
/// oversize report reaches its typed `TooLarge` refusal.
const REPORT_LIMIT: u64 = 268_435_457;

/// `REPORT_LIMIT` is the verifier's bound plus one, checked at compile time.
const _: () = assert!(
    REPORT_LIMIT == 268_435_457 && MAX_REPORT_BYTES == 268_435_456,
    "REPORT_LIMIT is MAX_REPORT_BYTES plus one"
);

/// Set bytes `larry eval` reads: one past the set bound, so an oversize set
/// reaches the parser's typed `TooLarge` refusal.
const SET_LIMIT: u64 = 4_194_305;

/// `SET_LIMIT` is the set bound plus one, checked at compile time.
const _: () = assert!(
    SET_LIMIT == 4_194_305 && MAX_SET_BYTES == 4_194_304,
    "SET_LIMIT is MAX_SET_BYTES plus one"
);

/// The interactive check never blocks on a clock: a retry that needs no wait
/// goes ahead, and one that would wait stops, so the operator sees the
/// gateway's refusal (with its `Retry-After`) at once.
const fn pause(wait: Duration) -> bool {
    wait.is_zero()
}

/// Why `larry model` failed; each variant keeps its cause.
#[derive(Debug)]
enum ModelCheck {
    /// Standard input could not be read as UTF-8 text.
    Prompt(io::Error),
    /// The prompt on standard input was empty.
    EmptyPrompt,
    /// A `LARRY_MODEL_*` setting is missing or unusable.
    Config(ModelError),
    /// The gateway refused or the exchange failed.
    Exchange(ModelError),
    /// The reply could not be written to standard output.
    Reply(io::Error),
}

impl fmt::Display for ModelCheck {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Prompt(ref cause) => write!(out, "model prompt unreadable: {cause}"),
            Self::EmptyPrompt => out.write_str("model: the prompt on stdin is empty"),
            Self::Config(ref cause) => write!(out, "model config: {cause}"),
            Self::Exchange(ref cause) => write!(out, "model failed: {cause}"),
            Self::Reply(ref cause) => write!(out, "model reply unwritable: {cause}"),
        }
    }
}

/// `larry model`: send standard input as one user turn to the gateway that
/// `LARRY_MODEL_*` names and print the reply. The operator's check that a key
/// and a gateway work before the executor spends on them.
fn model_once() -> Result<(), ModelCheck> {
    let mut prompt = String::new();
    io::stdin()
        .lock()
        .take(PROMPT_LIMIT)
        .read_to_string(&mut prompt)
        .map_err(ModelCheck::Prompt)?;
    let prompt = prompt.trim_end();
    if prompt.is_empty() {
        return Err(ModelCheck::EmptyPrompt);
    }
    let config =
        ModelConfig::from_env(|name| std::env::var(name).ok()).map_err(ModelCheck::Config)?;
    let client = ModelClient::new(config, pause);
    let completion: Completion = client
        .complete(&[Message::new(Role::User, prompt)])
        .map_err(|error: ModelError| ModelCheck::Exchange(error))?;
    let mut out = io::stdout().lock();
    writeln!(out, "{}", completion.content())
        .and_then(|()| out.flush())
        .map_err(ModelCheck::Reply)
}

/// Writes one diagnostic line to standard error, never to the protocol stream.
fn diagnose(message: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(message.as_bytes())?;
    stderr.write_all(b"\n")
}

/// Reads standard input up to `limit` bytes.
fn read_stdin(limit: u64) -> Result<Vec<u8>, ExecError> {
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(ExecError::Input)?;
    Ok(bytes)
}

/// Writes one line to standard output.
fn print_line(line: &dyn fmt::Display) -> Result<(), ExecError> {
    let mut out = io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(ExecError::Output)
}

/// `larry exec`: parse the task document on standard input, run it in a fresh clone,
/// and print the sealed JSON report. Returns whether the run was accepted; a task asking
/// for model commands is denied until Nova isolation lands, and never accepted.
fn exec_once() -> Result<bool, ExecError> {
    let settings = Settings::from_env(|name| std::env::var(name).ok())?;
    let bytes = read_stdin(TASK_LIMIT)?;
    let task = Task::parse(&bytes).map_err(ExecError::Task)?;
    let report: Report = execute(&task, &settings)?;
    print_line(report.json())?;
    Ok(report.accepted())
}

/// `larry verify`: check the report on standard input against the trusted key
/// (`LARRY_RECEIPT_KEY`, else the state directory's) and print its verdict.
/// Returns whether the verified run was accepted.
fn verify_once() -> Result<bool, ExecError> {
    let trusted = trusted_key(|name| std::env::var(name).ok())?;
    let bytes = read_stdin(REPORT_LIMIT)?;
    let verdict: Verdict = verify(&bytes, &trusted).map_err(ExecError::Receipt)?;
    print_line(&lgwks_std::json::json!({
        "verified": true,
        "accepted": verdict.accepted(),
        "task_id": verdict.task_id(),
    }))?;
    Ok(verdict.accepted())
}

/// `larry route add|list|remove|default|resolve`: the tenant's model routes in the
/// state directory (larry#201). Every argument must be UTF-8 text.
fn route_once(rest: &[OsString]) -> Result<(), RouteCliError> {
    let words: Option<Vec<&str>> = rest.iter().map(|word| word.to_str()).collect();
    let words = words.ok_or(RouteCliError::NotUtf8)?;
    route_run(
        |name| std::env::var(name).ok(),
        &words,
        &mut io::stdout().lock(),
    )
}

/// `larry key`: print the state directory's receipt public key as 64 hex characters,
/// the value an offline verifier sets as `LARRY_RECEIPT_KEY`.
fn key_once() -> Result<(), ExecError> {
    let public = state_public_key(|name| std::env::var(name).ok())?;
    print_line(&lgwks_std::hex::encode(public.to_bytes()))
}

/// Why `larry eval` printed no scorecard; each variant keeps its cause.
#[derive(Debug)]
enum EvalRun {
    /// The executor settings or standard input failed.
    Exec(ExecError),
    /// A price setting is missing or refused.
    Price(PriceError),
    /// The set was refused or a task failed outside the arm's doing.
    Eval(EvalError),
}

impl fmt::Display for EvalRun {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Exec(ref cause) => write!(out, "eval: {cause}"),
            Self::Price(ref cause) => write!(out, "eval price: {cause}"),
            Self::Eval(ref cause) => write!(out, "{cause}"),
        }
    }
}

/// `larry eval`: run every task of the matched set on standard input as one arm (the
/// model `LARRY_MODEL_*` pins, priced by `LARRY_EVAL_PRICE_IN`/`_OUT`) and print its
/// scorecard. Prices and the set are checked before the first model call (larry#87).
fn eval_once() -> Result<(), EvalRun> {
    let settings = Settings::from_env(|name| std::env::var(name).ok()).map_err(EvalRun::Exec)?;
    let prices = Prices::from_env(|name| std::env::var(name).ok()).map_err(EvalRun::Price)?;
    let bytes = read_stdin(SET_LIMIT).map_err(EvalRun::Exec)?;
    let card: Scorecard =
        larry_demo::eval::run(&bytes, &settings, prices).map_err(EvalRun::Eval)?;
    print_line(card.json()).map_err(EvalRun::Exec)
}

/// Why `larry governance` or `larry tracker` stopped without its verdict; each keeps its cause.
#[derive(Debug)]
enum GovernanceRun {
    /// The arguments were not `[FLAG] [--root DIR]`.
    Usage(Vec<OsString>),
    /// No root was given and the working directory is unreadable.
    NoDirectory(io::Error),
    /// Standard output could not be written.
    Stdout(io::Error),
    /// Standard error could not be written.
    Stderr(io::Error),
    /// `--check` found the committed page differs from the render.
    Stale,
    /// The tracker could not read, render or write.
    Tracker(governance::TrackerError),
}

impl fmt::Display for GovernanceRun {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usage(ref rest) => write!(
                out,
                "usage: larry governance [--report] [--root DIR] | larry tracker [--check] [--root DIR] (got {rest:?})"
            ),
            Self::NoDirectory(ref cause) => {
                write!(out, "governance: no working directory: {cause}")
            }
            Self::Stdout(ref cause) => write!(out, "governance: stdout unwritable: {cause}"),
            Self::Stderr(ref cause) => write!(out, "governance: stderr unwritable: {cause}"),
            Self::Stale => {
                out.write_str("FAIL: tools/tracker/index.html is stale — run `larry tracker`")
            }
            Self::Tracker(ref cause) => write!(out, "FAIL: {cause}"),
        }
    }
}

/// The root and flag `larry governance` and `larry tracker` take: `[FLAG] [--root DIR]`, in any
/// order, the root defaulting to the working directory.
fn governance_args(rest: &[OsString], flag: &str) -> Result<(bool, PathBuf), GovernanceRun> {
    let usage = || GovernanceRun::Usage(rest.to_vec());
    let mut set = false;
    let mut root: Option<PathBuf> = None;
    let mut items = rest.iter();
    while let Some(item) = items.next() {
        if item == flag && !set {
            set = true;
        } else if item == "--root" && root.is_none() {
            let Some(directory) = items.next() else {
                return Err(usage());
            };
            root = Some(PathBuf::from(directory));
        } else {
            return Err(usage());
        }
    }
    let root = match root {
        Some(root) => root,
        None => std::env::current_dir().map_err(GovernanceRun::NoDirectory)?,
    };
    Ok((set, root))
}

/// Writes each line to standard output and flushes.
fn print_lines(lines: &[&str]) -> Result<(), GovernanceRun> {
    let mut out = io::stdout().lock();
    for line in lines {
        out.write_all(line.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .map_err(GovernanceRun::Stdout)?;
    }
    out.flush().map_err(GovernanceRun::Stdout)
}

/// `larry governance [--report] [--root DIR]`: the structural checker (ADR 0010). Prints the
/// PASS block or the JSON report, or each error as a `FAIL:` line on standard error; exits 1
/// on any error.
fn governance_once(rest: &[OsString]) -> Result<ExitCode, GovernanceRun> {
    let (report_wanted, root) = governance_args(rest, "--report")?;
    let report: governance::Report = governance::check(&root);
    if report_wanted {
        print_lines(&[&report.pretty()])?;
    } else if report.ok() {
        print_lines(&[
            "PASS: document structure and governance inventory",
            &report.counts_line(),
            "NOT CHECKED BY THIS GATE: Mermaid rendering",
            "NOT RUN: runtime, model evaluations, hardware and human trials",
        ])?;
    } else {
        for error in report.errors() {
            diagnose(&format!("FAIL: {error}")).map_err(GovernanceRun::Stderr)?;
        }
    }
    Ok(if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `larry tracker [--check] [--root DIR]`: writes `tools/tracker/index.html` atomically, or with
/// `--check` exits 1 unless the committed page equals the render.
fn tracker_once(rest: &[OsString]) -> Result<ExitCode, GovernanceRun> {
    let (check_only, root) = governance_args(rest, "--check")?;
    if check_only {
        let current: Result<bool, governance::TrackerError> =
            governance::tracker::is_current(&root);
        return match current {
            Ok(true) => print_lines(&["PASS: tools/tracker/index.html is current"])
                .map(|()| ExitCode::SUCCESS),
            Ok(false) => Err(GovernanceRun::Stale),
            Err(error) => Err(GovernanceRun::Tracker(error)),
        };
    }
    let written: Result<usize, governance::TrackerError> = governance::tracker::write(&root);
    match written {
        Ok(bytes) => print_lines(&[&format!("wrote tools/tracker/index.html ({bytes} bytes)")])
            .map(|()| ExitCode::SUCCESS),
        Err(error) => Err(GovernanceRun::Tracker(error)),
    }
}

/// `larry serve`: bind `LARRY_SERVE_ADDR` (loopback by default), print one JSON line naming
/// the bound address, the edge's liveness path, the tenant probe and how many tenants hold a
/// key in `LARRY_STATE_DIR`, then serve until SIGTERM or SIGINT and exit 0.
fn serve_once() -> Result<(), larry_demo::serve::ServeError> {
    let settings = larry_demo::serve::Settings::from_env(|name| std::env::var(name).ok())?;
    larry_demo::serve::run(settings, |addr, keyed| {
        let path = larry_demo::serve::HEALTH_PATH;
        let line = lgwks_std::json::json!({
            "serving": format!("http://{addr}"),
            "health": format!("http://{addr}{path}"),
            "probe": format!("http://{addr}/t/{{tenant}}{path}"),
            "keyed_tenants": keyed,
        });
        let mut out = io::stdout().lock();
        writeln!(out, "{line}").and_then(|()| out.flush())
    })
}

/// `larry serve key <tenant>`: issue (or rotate) the tenant's bearer key and print it once,
/// as JSON; only its digest is kept in `LARRY_STATE_DIR`.
fn serve_key_once(tenant: &str) -> Result<(), larry_demo::serve::ServeError> {
    let key = larry_demo::serve::issue_key(|name| std::env::var(name).ok(), tenant)?;
    let line = lgwks_std::json::json!({
        "tenant": tenant,
        "key": key,
        "shown": "once: only its digest is stored; issuing again rotates it",
    });
    let mut out = io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(larry_demo::serve::ServeError::Announce)
}

/// The exit status of a sealed verdict.
fn verdict_status(accepted: bool) -> ExitCode {
    if accepted {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(REJECTED)
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).take(MAX_ARGUMENTS).collect();
    let outcome = match *args.as_slice() {
        [] => larry_demo::native::run()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| format!("DEMO terminal failed: {error}")),
        [ref command] if command == "mcp" => {
            larry_demo::mcp::serve(io::stdin().lock(), io::stdout().lock())
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error| format!("MCP server failed: {error}"))
        }
        [ref command] if command == "acp" => {
            larry_demo::acp::serve(io::BufReader::new(io::stdin()), io::stdout().lock())
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error| format!("ACP agent failed: {error}"))
        }
        [ref command] if command == "model" => model_once()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        [ref command] if command == "exec" => exec_once()
            .map(verdict_status)
            .map_err(|error| error.to_string()),
        [ref command] if command == "verify" => verify_once()
            .map(verdict_status)
            .map_err(|error| error.to_string()),
        [ref command] if command == "key" => key_once()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        [ref command, ref rest @ ..] if command == "route" => route_once(rest)
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        [ref command] if command == "eval" => eval_once()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        [ref command, ref rest @ ..] if command == "governance" => {
            governance_once(rest).map_err(|error: GovernanceRun| error.to_string())
        }
        [ref command, ref rest @ ..] if command == "tracker" => {
            tracker_once(rest).map_err(|error: GovernanceRun| error.to_string())
        }
        [ref command, ref rest @ ..] if command == "mascot" => {
            larry_demo::mascot::run(rest, &mut io::stdout().lock())
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error: larry_demo::mascot::MascotError| error.to_string())
        }
        [ref command] if command == "serve" => serve_once()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        [ref command, ref sub, ref tenant] if command == "serve" && sub == "key" => {
            serve_key_once(&tenant.to_string_lossy())
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error| error.to_string())
        }
        _ => Err(String::from(
            "DEMO: no arguments supported except `mcp`, `acp`, `model`, `exec`, `verify`, `key`, `route`, `eval`, `governance`, `tracker`, `mascot` and `serve`; F1 opens native help",
        )),
    };
    match outcome {
        Ok(status) => status,
        Err(message) => {
            if diagnose(&message).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::FAILURE
        }
    }
}
