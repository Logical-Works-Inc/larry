//! The shipped `larry` binary as a long-running stdio child: the one way the MCP
//! suites (`tests/mcp_support`) and the ACP journey (`tests/acp_harbor_journey.rs`,
//! larry#93) start it, with a cleared environment so no developer variable
//! (`LARRY_STATE_DIR`, `LARRY_MODEL_KEY`) can reach a test.

use std::error::Error;
use std::ffi::OsStr;
use std::io::BufReader;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Starts `larry <subcommand>` with only PATH and `env` set, stdin and stdout
/// piped and stderr as given; returns the child with both protocol pipes taken.
pub fn start(
    subcommand: &str,
    env: &[(&str, &OsStr)],
    stderr: Stdio,
) -> Result<(Child, ChildStdin, BufReader<ChildStdout>), Box<dyn Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_larry"))
        .arg(subcommand)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()?;
    let stdin = child.stdin.take().ok_or("child stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("child stdout was not piped")?;
    Ok((child, stdin, BufReader::new(stdout)))
}
