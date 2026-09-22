//! Native AppCUI launcher. Paints the fixture surface and exits when the
//! operator quits; it performs no backend or workspace effect.
use std::io::Write;

fn main() -> std::process::ExitCode {
    if std::env::args_os().len() != 1 {
        if writeln!(
            std::io::stderr().lock(),
            "DEMO: no arguments supported; F1 opens native help"
        )
        .is_err()
        {
            return std::process::ExitCode::FAILURE;
        }
        return std::process::ExitCode::FAILURE;
    }
    match rocco_demo::native::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if writeln!(std::io::stderr().lock(), "DEMO terminal failed: {error}").is_err() {
                return std::process::ExitCode::FAILURE;
            }
            std::process::ExitCode::FAILURE
        }
    }
}
