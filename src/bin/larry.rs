//! The terminal frontend (larry#239), `larry mcp`, the MCP stdio server (larry#84), `larry acp`,
//! the Agent Client Protocol agent Harbor runs Terminal-Bench through (larry#93),
//! `larry model`, one completion through the configured gateway (larry#83),
//! `larry exec`, the task-document executor (larry#85), and `larry verify` and
//! `larry key`, the receipt check and the key it trusts (larry#86), and
//! `larry route`, the tenant's model routes in the state directory (larry#201), and
//! `larry eval`, one priced arm over the matched task set (larry#87), and
//! `larry governance` and `larry tracker`, the document gate (larry#53, ADR 0010), and
//! `larry mascot`, the pixel raccoon's SVG, preview and pet package (larry#152, ADR 0014), and
//! `larry serve`, the HTTP host on lgwks_bot's app edge (larry#88, ADR 0021), and
//! `larry help`, `--help` and `-h`, the one table every subcommand and the
//! `LARRY_*` variables it reads are printed from (larry#228, ADR 0054).
//! The frontend runs real terminal sessions and asks before every command; it
//! exits 0 when the person quits.
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use larry_demo::eval::{EvalError, MAX_SET_BYTES, PriceError, Prices, Scorecard};
use larry_demo::executor::routes::{RouteCliError, run as route_run};
use larry_demo::executor::{
    ExecError, MAX_REPORT_BYTES, Report, Settings, Task, Verdict, execute, process_env,
    state_public_key, trusted_key, verify,
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

/// One subcommand: the word the first argument must be, its one-line summary
/// and the `LARRY_*` variables it reads (larry#228). `larry help`, `--help`
/// and `-h` render this one table, so the help a first-time user reads cannot
/// drift from the arms below.
struct Subcommand {
    /// The first argument that selects the command.
    name: &'static str,
    /// The one-line summary help prints.
    summary: &'static str,
    /// The `LARRY_*` variables the command reads, in a phrase, or `none`.
    env: &'static str,
}

/// The variables a command reads that runs a task through the executor: the
/// state directory, the policy, the command timeout and the model settings.
const EXECUTOR_ENV: &str = "LARRY_STATE_DIR, LARRY_POLICY, LARRY_EXEC_TIMEOUT_MS, LARRY_MODEL_URL, LARRY_MODEL, LARRY_MODEL_KEY, LARRY_MODEL_*, LARRY_KEY_*";

/// The variables `larry model` reads: one gateway, one key and the retry knobs.
const MODEL_ENV: &str = "LARRY_MODEL_URL, LARRY_MODEL, LARRY_MODEL_KEY, LARRY_MODEL_TEMPERATURE, LARRY_MODEL_RESIDENCY, LARRY_MODEL_TIMEOUT_MS, LARRY_MODEL_ATTEMPTS, LARRY_MODEL_BACKOFF_MS, LARRY_MODEL_DEADLINE_MS";

/// The phrase a command that reads no variable of its own prints.
const NO_ENV: &str = "none";

/// Every subcommand the dispatcher handles, in help order.
const COMMANDS: [Subcommand; 13] = [
    Subcommand {
        name: "help",
        summary: "print this help and exit",
        env: NO_ENV,
    },
    Subcommand {
        name: "mcp",
        summary: "serve MCP over stdin and stdout",
        env: EXECUTOR_ENV,
    },
    Subcommand {
        name: "acp",
        summary: "serve the Agent Client Protocol on stdin and stdout",
        env: EXECUTOR_ENV,
    },
    Subcommand {
        name: "model",
        summary: "send stdin to the model gateway and print one reply",
        env: MODEL_ENV,
    },
    Subcommand {
        name: "exec",
        summary: "run a task document from stdin",
        env: EXECUTOR_ENV,
    },
    Subcommand {
        name: "verify",
        summary: "check a sealed report from stdin",
        env: "LARRY_RECEIPT_KEY, LARRY_STATE_DIR",
    },
    Subcommand {
        name: "key",
        summary: "print the receipt public key",
        env: "LARRY_STATE_DIR",
    },
    Subcommand {
        name: "route",
        summary: "add, list, remove, default or resolve the model routes",
        env: "LARRY_STATE_DIR, LARRY_KEY_*, LARRY_MODEL_URL, LARRY_MODEL, LARRY_MODEL_KEY, LARRY_MODEL_TEMPERATURE, LARRY_MODEL_RESIDENCY",
    },
    Subcommand {
        name: "eval",
        summary: "run a matched task set as one priced arm",
        env: "LARRY_STATE_DIR, LARRY_POLICY, LARRY_EXEC_TIMEOUT_MS, LARRY_MODEL_*, LARRY_KEY_*, LARRY_EVAL_PRICE_IN, LARRY_EVAL_PRICE_OUT, LARRY_EVAL_PRICE_CACHED, LARRY_EVAL_PRICE_CACHE_WRITE",
    },
    Subcommand {
        name: "governance",
        summary: "check the document structure and inventory",
        env: NO_ENV,
    },
    Subcommand {
        name: "tracker",
        summary: "write or check tools/tracker/index.html",
        env: NO_ENV,
    },
    Subcommand {
        name: "mascot",
        summary: "draw the pixel mascot's SVG, preview and pet package",
        env: NO_ENV,
    },
    Subcommand {
        name: "serve",
        summary: "serve the HTTP edge; `serve key <tenant>` and `serve revoke <tenant>` manage its keys",
        env: "LARRY_SERVE_ADDR, LARRY_SERVE_PLAINTEXT, LARRY_STATE_DIR",
    },
];

/// The help a first-time user reads: what a bare `larry` does, then every
/// subcommand with its one-line summary and the `LARRY_*` variables it reads.
fn help_text() -> String {
    let mut text = String::from(
        "larry: the always-on harness of Logic OS\n\
         \n\
         usage: larry [<subcommand> [args...]]\n\
         \n\
         With no subcommand, larry runs the terminal session and asks before every command\n\
         (env: LARRY_STATE_DIR, LARRY_POLICY, LARRY_EXEC_TIMEOUT_MS, LARRY_MODEL_*, LARRY_KEY_*).\n\
         \n\
         subcommands:\n",
    );
    let width = COMMANDS
        .iter()
        .fold(0, |widest, command| widest.max(command.name.len()));
    for command in &COMMANDS {
        let line = format!(
            "  {:<width$}  {} (env: {})\n",
            command.name,
            command.summary,
            command.env,
            width = width
        );
        text.push_str(&line);
    }
    text
}

/// `larry help`, `larry --help` and `larry -h`: print the help and exit 0.
fn help_once() -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(help_text().as_bytes())
        .and_then(|()| out.flush())
}

/// Why `larry model` failed; each variant keeps its cause.
#[derive(Debug)]
enum ModelCheck {
    /// Standard input could not be read as UTF-8 text.
    Prompt(io::Error),
    /// The prompt on standard input was empty.
    EmptyPrompt,
    /// A variable Larry reads is not UTF-8.
    Env(ExecError),
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
            Self::Env(ref cause) => write!(out, "model: {cause}"),
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
    let config = ModelConfig::from_env(process_env().map_err(ModelCheck::Env)?)
        .map_err(ModelCheck::Config)?;
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
    let settings = Settings::from_env(process_env()?)?;
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
    let trusted = trusted_key(process_env()?)?;
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
fn route_once(rest: &[OsString]) -> Result<(), Failed<RouteCliError>> {
    let words: Option<Vec<&str>> = rest.iter().map(|word| word.to_str()).collect();
    let words = words.ok_or(Failed::Run(RouteCliError::NotUtf8))?;
    let read = process_env().map_err(Failed::Env)?;
    route_run(read, &words, &mut io::stdout().lock()).map_err(Failed::Run)
}

/// A subcommand that failed reading the environment, or failed itself.
#[derive(Debug)]
enum Failed<E> {
    /// A variable Larry reads is not UTF-8.
    Env(ExecError),
    /// The subcommand's own failure.
    Run(E),
}

impl<E: fmt::Display> fmt::Display for Failed<E> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Env(ref cause) => write!(out, "{cause}"),
            Self::Run(ref cause) => write!(out, "{cause}"),
        }
    }
}

/// `larry key`: print the state directory's receipt public key as 64 hex characters,
/// the value an offline verifier sets as `LARRY_RECEIPT_KEY`.
fn key_once() -> Result<(), ExecError> {
    let public = state_public_key(process_env()?)?;
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
    let settings =
        Settings::from_env(process_env().map_err(EvalRun::Exec)?).map_err(EvalRun::Exec)?;
    let prices = Prices::from_env(process_env().map_err(EvalRun::Exec)?).map_err(EvalRun::Price)?;
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
fn serve_once() -> Result<(), Failed<larry_demo::serve::ServeError>> {
    let read = process_env().map_err(Failed::Env)?;
    let settings = larry_demo::serve::Settings::from_env(read).map_err(Failed::Run)?;
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
    .map_err(Failed::Run)
}

/// `larry serve key <tenant>`: issue (or rotate) the tenant's bearer key and print it once,
/// as JSON; only its digest is kept in `LARRY_STATE_DIR`.
fn serve_key_once(tenant: &str) -> Result<(), Failed<larry_demo::serve::ServeError>> {
    let read = process_env().map_err(Failed::Env)?;
    let key = larry_demo::serve::issue_key(read, tenant).map_err(Failed::Run)?;
    let line = lgwks_std::json::json!({
        "tenant": tenant,
        "key": key,
        "shown": "once: only its digest is stored; issuing again rotates it",
    });
    let mut out = io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(|cause| Failed::Run(larry_demo::serve::ServeError::Announce(cause)))
}

/// `larry serve revoke <tenant>`: delete the tenant's key digest from `LARRY_STATE_DIR`, so a
/// running serve refuses it from its next SIGHUP and a stopped one at its next start.
fn serve_revoke_once(tenant: &str) -> Result<(), Failed<larry_demo::serve::ServeError>> {
    let read = process_env().map_err(Failed::Env)?;
    larry_demo::serve::revoke_key(read, tenant).map_err(Failed::Run)?;
    let line = lgwks_std::json::json!({
        "tenant": tenant,
        "revoked": true,
        "takes_effect": "a running serve on SIGHUP; a stopped one at its next start",
    });
    let mut out = io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(|cause| Failed::Run(larry_demo::serve::ServeError::Announce(cause)))
}

/// The exit status of a sealed verdict.
fn verdict_status(accepted: bool) -> ExitCode {
    if accepted {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(REJECTED)
    }
}

/// The refusal for arguments `larry help` does not describe.
const UNKNOWN: &str =
    "larry: unknown arguments; run `larry help` for every subcommand and its LARRY_* variables";

/// Whether `word` is a subcommand `COMMANDS` names, or one of the three help
/// spellings. The first arm of `main`'s match refuses every word this refuses,
/// so an arm for a word no help row names is unreachable at run time and the
/// table is the list (larry#228).
fn listed(word: &str) -> bool {
    matches!(word, "help" | "--help" | "-h") || COMMANDS.iter().any(|command| command.name == word)
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).take(MAX_ARGUMENTS).collect();
    let outcome = match *args.as_slice() {
        // A bare `larry` is the terminal session; help's first paragraph names it.
        [] => larry_demo::tui::run()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| format!("larry: {error}")),
        // Every word that reaches an arm below is one help names (larry#228).
        [ref command, ..] if !listed(command.to_string_lossy().as_ref()) => {
            Err(String::from(UNKNOWN))
        }
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
        [ref command, ref sub, ref tenant] if command == "serve" && sub == "revoke" => {
            serve_revoke_once(&tenant.to_string_lossy())
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error| error.to_string())
        }
        [ref command, ..] if command == "help" || command == "--help" || command == "-h" => {
            help_once()
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error| format!("larry help: {error}"))
        }
        _ => Err(String::from(UNKNOWN)),
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

#[cfg(test)]
mod tests {
    use super::{COMMANDS, help_text, listed};

    /// Every word an arm of `main`'s match handles, beside the arms (larry#228).
    /// `listed` is what stops a word no help row names before any arm runs, so
    /// this list and `COMMANDS` must hold the same names.
    const DISPATCHED: [&str; 13] = [
        "help",
        "mcp",
        "acp",
        "model",
        "exec",
        "verify",
        "key",
        "route",
        "eval",
        "governance",
        "tracker",
        "mascot",
        "serve",
    ];

    /// Every dispatched word appears in help once with its variables, and every
    /// help row is a dispatched word: the table and the arms cannot drift.
    #[test]
    fn every_dispatch_arm_is_in_help() {
        let help = help_text();
        let rows: Vec<&str> = COMMANDS.iter().map(|command| command.name).collect();
        for name in DISPATCHED {
            assert!(
                rows.contains(&name),
                "{name} dispatches but has no help row"
            );
            let found: Vec<&str> = help
                .lines()
                .filter(|line| line.trim_start().starts_with(&format!("{name} ")))
                .collect();
            assert_eq!(found.len(), 1, "{name} appears once in help: {found:?}");
            for line in &found {
                assert!(
                    line.contains("(env: "),
                    "{name} names the LARRY_* variables it reads: {line}"
                );
            }
        }
        assert_eq!(
            rows.len(),
            DISPATCHED.len(),
            "every help row is a dispatch arm"
        );
        assert!(help.contains("serve"), "help names the last subcommand");
        assert_eq!(
            help.lines().filter(|line| line.starts_with("  ")).count(),
            COMMANDS.len(),
            "help prints one line per subcommand"
        );
    }

    /// The gate admits every table row and the help spellings, and refuses
    /// every other word, so an arm no row names never runs (larry#228).
    #[test]
    fn only_listed_words_dispatch() {
        for command in &COMMANDS {
            assert!(listed(command.name), "{} is listed", command.name);
        }
        for spelling in ["help", "--help", "-h"] {
            assert!(listed(spelling), "{spelling} is listed");
        }
        for word in ["", "doctor", "--verbose", "HELP", "help me"] {
            assert!(!listed(word), "{word:?} is not listed");
        }
    }
}
