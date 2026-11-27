//! One cleared `larry route` spawn for the journeys that take no extra
//! environment (legacy rows, price tables): a single builder, so the cleared
//! environment and the profile forward never drift between copies.

use std::error::Error;
use std::path::Path;
use std::process::{Command, Output};

use crate::coverage::forward;

/// Runs `larry route <args>` with only PATH and the state directory set.
pub(crate) fn larry_route(state: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state);
    forward(&mut command);
    Ok(command.output()?)
}

/// The child's stderr as text.
pub(crate) fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
