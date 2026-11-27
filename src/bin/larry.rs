//! The terminal frontend (larry#239), `larry mcp`, the MCP stdio server (larry#84), `larry acp`,
//! the Agent Client Protocol agent Harbor runs Terminal-Bench through (larry#93),
//! `larry model`, one completion through the configured gateway (larry#83),
//! `larry exec`, the task-document executor (larry#85), and `larry verify` and
//! `larry key`, the receipt check and the key it trusts (larry#86), and
//! `larry route`, the tenant's model routes in the state directory (larry#201), and
//! `larry eval`, one priced arm over the matched task set (larry#87), and
//! `larry governance` and `larry tracker`, the document gate (larry#53, ADR 0010), and
//! `larry mascot`, the pixel raccoon's SVG, preview and pet package (larry#152, ADR 0014), and
//! `larry mutants verdict`, the mutation lane's verdict from main's build (larry#387, ADR 0098), and
//! `larry serve`, the HTTP host on lgwks_bot's app edge (larry#88, ADR 0021), and
//! `larry help`, `--help` and `-h`, the one table every subcommand and the
//! `LARRY_*` variables it reads are printed from (larry#228, ADR 0054).
//! The frontend runs real terminal sessions and asks before every command; it
//! exits 0 when the person quits.
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Duration;

use larry_demo::executor::routes::{RouteCliError, run as route_run};
use larry_demo::executor::{
    ExecError, MAX_REPORT_BYTES, Report, Settings, Task, Verdict, execute, process_env,
    state_public_key, trusted_key, verify,
};
use larry_demo::model::{Completion, Message, ModelClient, ModelConfig, ModelError, Role};
use lgwks_macros::Error;

/// `larry governance` and `larry tracker`, split out by responsibility (ADR 0065).
#[path = "larry/governance_cli.rs"]
mod governance_cli;
use governance_cli::{governance_once, tracker_once};

/// `larry eval`, split out by responsibility when a stopped arm's card took the path (larry#399).
#[path = "larry/eval_cli.rs"]
mod eval_cli;
use eval_cli::eval_once;

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

/// The interactive check never blocks on a clock: a retry that needs no wait
/// goes ahead, and one that would wait stops, so the operator sees the
/// gateway's refusal (with its `Retry-After`) at once.
const fn pause(wait: Duration) -> bool {
    wait.is_zero()
}

/// What a subcommand does with the arguments after its word: its exit status, or
/// the refusal `main` prints as one line on standard error before exiting 1.
type Handler = fn(&[OsString]) -> Result<ExitCode, Refusal>;

/// One subcommand: the word the first argument must be, its one-line summary,
/// the `LARRY_*` variables it reads (larry#228) and the handler that runs it
/// (larry#295). `larry help`, `--help` and `-h` render this one table and `main`
/// dispatches by looking the word up in it, so the help a first-time user reads
/// cannot drift from what runs.
struct Subcommand {
    /// The first argument that selects the command.
    name: &'static str,
    /// The one-line summary help prints.
    summary: &'static str,
    /// The `LARRY_*` variables the command reads, in a phrase, or `none`.
    env: &'static str,
    /// Runs the command on the arguments after its word; it refuses any it does not take.
    run: Handler,
}

/// The variables a command reads that runs a task through the executor: the
/// state directory, the policy, the command timeout and the model settings.
const EXECUTOR_ENV: &str = "LARRY_STATE_DIR, LARRY_POLICY, LARRY_EXEC_TIMEOUT_MS, LARRY_MODEL_URL, LARRY_MODEL, LARRY_MODEL_KEY, LARRY_MODEL_*, LARRY_KEY_*";

/// The variables `larry model` reads: one gateway, one key, the retry knobs, the
/// tenant budgets and the price overrides a route without a table is priced from.
const MODEL_ENV: &str = "LARRY_MODEL_URL, LARRY_MODEL, LARRY_MODEL_KEY, LARRY_MODEL_TEMPERATURE, LARRY_MODEL_RESIDENCY, LARRY_MODEL_TIMEOUT_MS, LARRY_MODEL_ATTEMPTS, LARRY_MODEL_BACKOFF_MS, LARRY_MODEL_DEADLINE_MS, LARRY_MODEL_TOKEN_BUDGET, LARRY_MODEL_MICRO_DOLLAR_BUDGET, LARRY_MODEL_PRICE_IN_MICROS, LARRY_MODEL_PRICE_OUT_MICROS";

/// The phrase a command that reads no variable of its own prints.
const NO_ENV: &str = "none";

/// Every subcommand the dispatcher handles, in help order. A `static`, so a
/// lookup hands out a `'static` row.
static COMMANDS: [Subcommand; 14] = [
    Subcommand {
        name: "help",
        summary: "print this help and exit",
        env: NO_ENV,
        // Words after it still get the help, so `larry help me` helps.
        run: |_rest| ran("larry help", help_once()),
    },
    Subcommand {
        name: "mcp",
        summary: "serve MCP over stdin and stdout",
        env: EXECUTOR_ENV,
        run: |rest| {
            bare(rest)?;
            let served = larry_demo::mcp::serve(io::stdin().lock(), io::stdout().lock());
            ran("MCP server failed", served)
        },
    },
    Subcommand {
        name: "acp",
        summary: "serve the Agent Client Protocol on stdin and stdout",
        env: EXECUTOR_ENV,
        run: |rest| {
            bare(rest)?;
            let stdin = io::BufReader::new(io::stdin());
            ran(
                "ACP agent failed",
                larry_demo::acp::serve(stdin, io::stdout().lock()),
            )
        },
    },
    Subcommand {
        name: "model",
        summary: "send stdin to the model gateway and print one reply",
        env: MODEL_ENV,
        run: |rest| bare(rest).and_then(|()| ran("", model_once())),
    },
    Subcommand {
        name: "exec",
        summary: "run a task document from stdin",
        env: EXECUTOR_ENV,
        run: |rest| bare(rest).and_then(|()| judged(exec_once())),
    },
    Subcommand {
        name: "verify",
        summary: "check a sealed report from stdin",
        env: "LARRY_RECEIPT_KEY, LARRY_STATE_DIR",
        run: |rest| bare(rest).and_then(|()| judged(verify_once())),
    },
    Subcommand {
        name: "key",
        summary: "print the receipt public key",
        env: "LARRY_STATE_DIR",
        run: |rest| bare(rest).and_then(|()| ran("", key_once())),
    },
    Subcommand {
        name: "route",
        summary: "add, price, set residency, list, remove, default or resolve model routes",
        env: "LARRY_STATE_DIR, LARRY_KEY_*, LARRY_MODEL_URL, LARRY_MODEL, LARRY_MODEL_KEY, LARRY_MODEL_TEMPERATURE, LARRY_MODEL_RESIDENCY",
        run: |rest| ran("", route_once(rest)),
    },
    Subcommand {
        name: "eval",
        summary: "run a matched task set as one priced arm",
        env: "LARRY_STATE_DIR, LARRY_POLICY, LARRY_EXEC_TIMEOUT_MS, LARRY_MODEL_*, LARRY_KEY_*, LARRY_EVAL_PRICE_IN, LARRY_EVAL_PRICE_OUT, LARRY_EVAL_PRICE_CACHED, LARRY_EVAL_PRICE_CACHE_WRITE",
        run: |rest| bare(rest).and_then(|()| ran("", eval_once())),
    },
    Subcommand {
        name: "governance",
        summary: "check the document structure and inventory",
        env: NO_ENV,
        run: |rest| governance_once(rest).map_err(failed("")),
    },
    Subcommand {
        name: "tracker",
        summary: "write or check tools/tracker/index.html",
        env: NO_ENV,
        run: |rest| tracker_once(rest).map_err(failed("")),
    },
    Subcommand {
        name: "mascot",
        summary: "draw the pixel mascot's SVG, preview and pet package",
        env: NO_ENV,
        run: |rest| ran("", larry_demo::mascot::run(rest, &mut io::stdout().lock())),
    },
    Subcommand {
        name: "mutants",
        summary: "`mutants verdict <settings> <artifacts> <jobs.json>` judges one mutation run from its data",
        env: NO_ENV,
        run: |rest| {
            larry_demo::mutants::run(rest, &mut io::stdout().lock())
                .map(verdict_status)
                .map_err(failed(""))
        },
    },
    Subcommand {
        name: "serve",
        summary: "serve the HTTP edge; `serve key <tenant>` and `serve revoke <tenant>` manage its keys",
        env: "LARRY_SERVE_ADDR, LARRY_SERVE_PLAINTEXT, LARRY_STATE_DIR",
        run: run_serve,
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
#[derive(Debug, Error)]
enum ModelCheck {
    /// Standard input could not be read as UTF-8 text.
    #[error("model prompt unreadable: {0}")]
    Prompt(#[source] io::Error),
    /// The prompt on standard input was empty.
    #[error("model: the prompt on stdin is empty")]
    EmptyPrompt,
    /// A variable Larry reads is not UTF-8.
    #[error("model: {0}")]
    Env(#[source] ExecError),
    /// A `LARRY_MODEL_*` setting is missing or unusable.
    #[error("model config: {0}")]
    Config(#[source] ModelError),
    /// The gateway refused or the exchange failed.
    #[error("model failed: {0}")]
    Exchange(#[source] ModelError),
    /// The host's clock reads before the Unix epoch, so no retry budget can be timed (larry#436).
    #[error("model clock: {0}")]
    Clock(#[source] larry_runtime::error::RuntimeError),
    /// The reply could not be written to standard output.
    #[error("model reply unwritable: {0}")]
    Reply(#[source] io::Error),
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
    let clock = larry_runtime::live_clock().map_err(ModelCheck::Clock)?;
    let client = ModelClient::new(config, clock, pause);
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
#[derive(Debug, Error)]
enum Failed<E: std::error::Error + 'static> {
    /// A variable Larry reads is not UTF-8.
    #[error("{0}")]
    Env(#[source] ExecError),
    /// The subcommand's own failure.
    #[error("{0}")]
    Run(#[source] E),
}

/// `larry key`: print the state directory's receipt public key as 64 hex characters,
/// the value an offline verifier sets as `LARRY_RECEIPT_KEY`.
fn key_once() -> Result<(), ExecError> {
    let public = state_public_key(process_env()?)?;
    print_line(&lgwks_std::hex::encode(public.to_bytes()))
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

/// Why a subcommand exited 1 without its result.
#[derive(Debug)]
enum Refusal {
    /// Arguments `larry help` does not describe: an unknown word, or a listed
    /// word with arguments it does not take.
    Unknown,
    /// More arguments than the command line takes, named by the bound.
    TooMany {
        /// The most arguments a command line may carry.
        limit: usize,
    },
    /// The subcommand ran and failed; the cause keeps its type and source.
    Failed {
        /// The words that name the command, or empty when the cause names it.
        context: &'static str,
        /// What failed.
        cause: Box<dyn std::error::Error>,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unknown => out.write_str(UNKNOWN),
            Self::TooMany { limit } => write!(
                out,
                "larry: more than {limit} arguments; run `larry help` for every subcommand and its LARRY_* variables"
            ),
            Self::Failed {
                context: "",
                ref cause,
            } => write!(out, "{cause}"),
            Self::Failed { context, ref cause } => write!(out, "{context}: {cause}"),
        }
    }
}

/// The failure of a command that ran, under the words that name it.
fn failed<E: std::error::Error + 'static>(context: &'static str) -> impl FnOnce(E) -> Refusal {
    move |cause| Refusal::Failed {
        context,
        cause: Box::new(cause),
    }
}

/// Exit 0 once a command ran to its end, else its failure.
fn ran<E: std::error::Error + 'static>(
    context: &'static str,
    outcome: Result<(), E>,
) -> Result<ExitCode, Refusal> {
    outcome.map(|()| ExitCode::SUCCESS).map_err(failed(context))
}

/// A sealed verdict's exit status, else the failure that stopped the run.
fn judged(outcome: Result<bool, ExecError>) -> Result<ExitCode, Refusal> {
    outcome.map(verdict_status).map_err(failed(""))
}

/// Refuses any argument after a word that takes none.
fn bare(rest: &[OsString]) -> Result<(), Refusal> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(Refusal::Unknown)
    }
}

/// `larry serve`, `larry serve key <tenant>` and `larry serve revoke <tenant>`; any other
/// shape is refused before anything binds or writes.
fn run_serve(rest: &[OsString]) -> Result<ExitCode, Refusal> {
    let outcome = match *rest {
        [] => serve_once(),
        [ref sub, ref tenant] if sub == "key" => serve_key_once(&tenant.to_string_lossy()),
        [ref sub, ref tenant] if sub == "revoke" => serve_revoke_once(&tenant.to_string_lossy()),
        _ => return Err(Refusal::Unknown),
    };
    ran("", outcome)
}

/// The row `word` selects: a table name, or `--help` and `-h` for help's row. A word
/// no row names finds nothing, so it runs nothing (larry#228, larry#295).
fn find(word: &str) -> Option<&'static Subcommand> {
    let name = match word {
        "--help" | "-h" => "help",
        other => other,
    };
    COMMANDS.iter().find(|command| command.name == name)
}

/// The command line's arguments, or a typed refusal past `MAX_ARGUMENTS`: a bound
/// that silently drops arguments is no bound (larry#338).
fn bounded(args: impl IntoIterator<Item = OsString>) -> Result<Vec<OsString>, Refusal> {
    let args: Vec<OsString> = args
        .into_iter()
        .take(MAX_ARGUMENTS.saturating_add(1))
        .collect();
    if args.len() > MAX_ARGUMENTS {
        Err(Refusal::TooMany {
            limit: MAX_ARGUMENTS,
        })
    } else {
        Ok(args)
    }
}

fn main() -> ExitCode {
    let outcome = bounded(std::env::args_os().skip(1)).and_then(|args| match args.split_first() {
        // A bare `larry` is the terminal session; help's first paragraph names it.
        None => ran("larry", larry_demo::tui::run()),
        // The table is the dispatcher: a word runs only through the row help prints.
        Some((word, rest)) => match word.to_str().and_then(find) {
            Some(row) => (row.run)(rest),
            None => Err(Refusal::Unknown),
        },
    });
    match outcome {
        Ok(status) => status,
        Err(refusal) => {
            if diagnose(&refusal.to_string()).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::eval_cli::EvalRun;
    use super::{COMMANDS, Failed, MAX_ARGUMENTS, ModelCheck, Refusal, bounded, find, help_text};

    /// Every subcommand refusal keeps its cause as a source, never an empty `Error` impl
    /// (larry#343).
    #[test]
    fn every_command_refusal_keeps_its_source() {
        use std::error::Error as _;

        use larry_demo::eval::{EvalError, PriceError};
        use larry_demo::executor::ExecError;
        use larry_demo::model::ModelError;

        let model = ModelCheck::Prompt(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        assert!(model.source().is_some(), "model prompt");
        let model = ModelCheck::Config(ModelError::MissingKey);
        assert!(model.source().is_some(), "model config");
        let model = ModelCheck::Env(ExecError::StateDirUnset);
        assert!(model.source().is_some(), "model env");
        assert!(
            ModelCheck::EmptyPrompt.source().is_none(),
            "a refusal with no cause has no source"
        );
        let failed: Failed<std::io::Error> = Failed::Env(ExecError::StateDirUnset);
        assert!(failed.source().is_some(), "a failed environment read");
        let failed: Failed<std::io::Error> =
            Failed::Run(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        assert!(failed.source().is_some(), "a failed subcommand");
        let eval = EvalRun::Exec(ExecError::StateDirUnset);
        assert!(eval.source().is_some(), "eval exec");
        let eval = EvalRun::Price(PriceError::Missing { variable: "P" });
        assert!(eval.source().is_some(), "eval price");
        let eval = EvalRun::Eval(EvalError::ModelUnconfigured);
        assert!(eval.source().is_some(), "eval task");
        let eval = EvalRun::Unprinted {
            stopped: Box::new(EvalError::ModelUnconfigured),
            output: Box::new(ExecError::StateDirUnset),
        };
        assert!(eval.source().is_some(), "eval stopped and unprinted");
    }

    /// Every row is found by its own name and by no other, and help prints it
    /// once with its variables: the table is the dispatcher, so there is no
    /// second list of words to drift from it (larry#228, larry#295).
    #[test]
    fn every_listed_word_finds_its_own_row() {
        let help = help_text();
        for command in &COMMANDS {
            let found = find(command.name).map(|row| row.summary);
            assert_eq!(
                found,
                Some(command.summary),
                "{} finds its row",
                command.name
            );
            let lines: Vec<&str> = help
                .lines()
                .filter(|line| line.trim_start().starts_with(&format!("{} ", command.name)))
                .collect();
            assert_eq!(lines.len(), 1, "{} appears once: {lines:?}", command.name);
            assert!(
                lines.iter().all(|line| line.contains("(env: ")),
                "{lines:?}"
            );
        }
        let last = COMMANDS.last().map(|row| row.summary);
        assert_eq!(find("serve").map(|row| row.summary), last, "serve is last");
        assert_eq!(
            help.lines().filter(|line| line.starts_with("  ")).count(),
            COMMANDS.len(),
            "help prints one line per subcommand"
        );
    }

    /// `--help` and `-h` find help's row and every other word finds none, so a
    /// word no row names runs nothing (larry#228, larry#295).
    #[test]
    fn only_listed_words_dispatch() {
        for spelling in ["help", "--help", "-h"] {
            assert_eq!(
                find(spelling).map(|row| row.name),
                Some("help"),
                "{spelling}"
            );
        }
        for word in ["", "doctor", "--verbose", "HELP", "help me"] {
            assert!(find(word).is_none(), "{word:?} is not listed");
        }
    }

    /// The command line takes exactly the bound and refuses one past it, naming
    /// the bound (larry#338).
    #[test]
    fn larry_takes_arguments_up_to_the_bound_and_refuses_the_next() {
        let kept: Vec<OsString> = (0..MAX_ARGUMENTS).map(|_| OsString::from("x")).collect();
        let kept = bounded(kept)
            .map(|args| args.len())
            .map_err(|refusal| refusal.to_string());
        assert_eq!(kept, Ok(MAX_ARGUMENTS), "the bound itself is admitted");
        let past: Vec<OsString> = (0..=MAX_ARGUMENTS).map(|_| OsString::from("x")).collect();
        assert!(
            matches!(bounded(past), Err(Refusal::TooMany { limit }) if limit == MAX_ARGUMENTS),
            "one past the bound names it"
        );
    }
}
