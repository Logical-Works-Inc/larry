//! Native AppCUI launcher, `larry mcp`, the MCP stdio server (larry#84),
//! `larry model`, one completion through the configured gateway (larry#83),
//! `larry exec`, the task-document executor (larry#85), and `larry verify` and
//! `larry key`, the receipt check and the key it trusts (larry#86).
//! The launcher paints the fixture surface and exits when the operator
//! quits; it performs no backend or workspace effect.
#[path = "../mcp/mod.rs"]
mod mcp;

use std::fmt;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Duration;

use larry_demo::executor::{
    ExecError, MAX_REPORT_BYTES, Report, Settings, Task, Verdict, execute, state_public_key,
    trusted_key, verify,
};
use larry_demo::model::{Completion, Message, ModelClient, ModelConfig, ModelError, Role};

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

/// `larry key`: print the state directory's receipt public key as 64 hex characters,
/// the value an offline verifier sets as `LARRY_RECEIPT_KEY`.
fn key_once() -> Result<(), ExecError> {
    let public = state_public_key(|name| std::env::var(name).ok())?;
    print_line(&lgwks_std::hex::encode(public.to_bytes()))
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
    let mut args = std::env::args_os().skip(1);
    let outcome = match (args.next(), args.next()) {
        (None, _) => larry_demo::native::run()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| format!("DEMO terminal failed: {error}")),
        (Some(command), None) if command == "mcp" => {
            mcp::serve(io::stdin().lock(), io::stdout().lock())
                .map(|()| ExitCode::SUCCESS)
                .map_err(|error| format!("MCP server failed: {error}"))
        }
        (Some(command), None) if command == "model" => model_once()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        (Some(command), None) if command == "exec" => exec_once()
            .map(verdict_status)
            .map_err(|error| error.to_string()),
        (Some(command), None) if command == "verify" => verify_once()
            .map(verdict_status)
            .map_err(|error| error.to_string()),
        (Some(command), None) if command == "key" => key_once()
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| error.to_string()),
        _ => Err(String::from(
            "DEMO: no arguments supported except `mcp`, `model`, `exec`, `verify` and `key`; F1 opens native help",
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
