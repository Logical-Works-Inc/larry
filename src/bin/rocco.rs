//! Native AppCUI launcher. Paints the fixture surface and exits when the
//! operator quits; it performs no backend or workspace effect.
fn main() -> std::process::ExitCode {
    if std::env::args_os().len() != 1 {
        eprintln!("DEMO: no arguments supported; F1 opens native help");
        return std::process::ExitCode::FAILURE;
    }
    match rocco_demo::native::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("DEMO terminal failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
