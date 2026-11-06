//! Native AppCUI launcher, `rocco mcp`, the MCP stdio server (larry#84),
//! `rocco model`, one completion through the configured gateway (larry#83),
//! and `rocco exec`, the task-document executor (larry#85).
//! The launcher paints the fixture surface and exits when the operator
//! quits; it performs no backend or workspace effect.
#[path = "../mcp/mod.rs"]
mod mcp;

use std::fmt;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Duration;

use rocco_demo::executor::{ExecError, Settings, Task, execute, refuse_model_command};
use rocco_demo::model::{Completion, Message, ModelClient, ModelConfig, ModelError, Role};

/// Prompt bytes `rocco model` reads: one past the client's 1 MiB request
/// bound, so an oversize prompt reaches the client's typed refusal.
const PROMPT_LIMIT: u64 = 1_048_577;

/// Task bytes `rocco exec` reads: one past the executor's 64 KiB task bound,
/// so an oversize task reaches the parser's typed `TooLarge` refusal.
const TASK_LIMIT: u64 = 65_537;

/// The interactive check never blocks on a clock: a retry that needs no wait
/// goes ahead, and one that would wait stops, so the operator sees the
/// gateway's refusal (with its `Retry-After`) at once.
const fn pause(wait: Duration) -> bool {
    wait.is_zero()
}

/// Why `rocco model` failed; each variant keeps its cause.
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

/// `rocco model`: send standard input as one user turn to the gateway that
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

/// `rocco exec`: parse the task document on standard input, run it in a fresh clone,
/// and print the JSON report. Model commands are refused until Nova isolation lands.
fn exec_once() -> Result<(), ExecError> {
    let settings = Settings::from_env(|name| std::env::var(name).ok())?;
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(TASK_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(ExecError::Input)?;
    let task = Task::parse(&bytes).map_err(ExecError::Task)?;
    if task.has_model_commands() {
        return Err(refuse_model_command());
    }
    let report = execute(&task, &settings)?;
    let mut out = io::stdout().lock();
    writeln!(out, "{report}")
        .and_then(|()| out.flush())
        .map_err(ExecError::Output)
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let outcome = match (args.next(), args.next()) {
        (None, _) => {
            rocco_demo::native::run().map_err(|error| format!("DEMO terminal failed: {error}"))
        }
        (Some(command), None) if command == "mcp" => {
            mcp::serve(io::stdin().lock(), io::stdout().lock())
                .map_err(|error| format!("MCP server failed: {error}"))
        }
        (Some(command), None) if command == "model" => {
            model_once().map_err(|error| error.to_string())
        }
        (Some(command), None) if command == "exec" => {
            exec_once().map_err(|error| error.to_string())
        }
        _ => Err(String::from(
            "DEMO: no arguments supported except `mcp`, `model` and `exec`; F1 opens native help",
        )),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            if diagnose(&message).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::FAILURE
        }
    }
}
