//! The shipped `larry` binary as a one-shot child process: started with a cleared
//! environment, fed on stdin, and its stderr read back. Shared by
//! `tests/support/exec_cli.rs` (re-exported to the executor and receipt
//! journeys), `tests/sim_exec_gate.rs` (larry#164) and `tests/cli_help_journey.rs`,
//! so the child is started one way. Its sealed report is read by
//! `tests/support/sealed_report.rs`.

use std::error::Error;
use std::ffi::OsStr;
use std::process::{Child, Command, Output, Stdio};

#[path = "stdin_feed.rs"]
mod stdin_feed;
use stdin_feed::feed_stdin;

use crate::coverage::forward;

/// Starts `larry` with `args`, only PATH and `env` set, writes `input` to its
/// stdin and closes it. The one-shot children of the executor and CLI journeys
/// are built here; `larry_child` builds the long-running protocol children
/// (larry#228).
pub(crate) fn spawn_larry_args(
    args: &[&str],
    env: &[(&str, &OsStr)],
    input: &[u8],
) -> Result<Child, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .envs(env.iter().copied());
    forward(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(stdin) = child.stdin.take() {
        // A refused input may exit before it reads stdin; the broken pipe is the refusal itself.
        feed_stdin(stdin, input)?;
    }
    Ok(child)
}

/// The child's stderr as text.
#[must_use]
pub(crate) fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
