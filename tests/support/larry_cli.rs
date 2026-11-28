//! The shipped `larry` binary as a one-shot child process: started with a cleared
//! environment, fed on stdin, and its stderr read back. Shared by
//! `tests/support/exec_cli.rs` (re-exported to the executor and receipt
//! journeys), `tests/sim_exec_gate.rs` (larry#164) and `tests/cli_help_journey.rs`,
//! so the child is started one way. Its sealed report is read by
//! `tests/support/sealed_report.rs`.

use std::error::Error;
use std::ffi::OsStr;
use std::process::{Command, Output, Stdio};

#[path = "stdin_feed.rs"]
mod stdin_feed;
use stdin_feed::feed_stdin;

use crate::coverage::forward;
use crate::larry_guard::Larry;

/// Starts `larry` with `args`, only PATH and `env` set, writes `input` to its
/// stdin and closes it. The one-shot children of the executor and CLI journeys
/// are built here; `larry_child` builds the long-running protocol children
/// (larry#228). The guard kills and reaps the child when it drops (larry#573).
pub(crate) fn spawn_larry_args(
    args: &[&str],
    env: &[(&str, &OsStr)],
    input: &[u8],
) -> Result<Larry, Box<dyn Error>> {
    let mut child = spawn_larry_on(args, env, Stdio::piped())?;
    if let Some(stdin) = child.stdin.take() {
        // A refused input may exit before it reads stdin; the broken pipe is the refusal itself.
        feed_stdin(stdin, input)?;
    }
    Ok(child)
}

/// Starts `larry` with `args` on the `stdin` given, only PATH and `env` set and
/// both output streams piped.
pub(crate) fn spawn_larry_on(
    args: &[&str],
    env: &[(&str, &OsStr)],
    stdin: Stdio,
) -> Result<Larry, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .envs(env.iter().copied());
    forward(&mut command);
    command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(Larry::spawn(&mut command)?)
}

/// The child's stderr as text.
#[must_use]
pub(crate) fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
