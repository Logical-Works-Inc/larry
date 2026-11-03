//! Native AppCUI launcher, and `rocco mcp`, the MCP stdio server (larry#84).
//! The launcher paints the fixture surface and exits when the operator
//! quits; it performs no backend or workspace effect.
#[path = "../mcp/mod.rs"]
mod mcp;

use std::io::{self, Write};
use std::process::ExitCode;

/// Writes one diagnostic line to standard error, never to the protocol stream.
fn diagnose(message: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(message.as_bytes())?;
    stderr.write_all(b"\n")
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
        _ => Err(String::from(
            "DEMO: no arguments supported except `mcp`; F1 opens native help",
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
