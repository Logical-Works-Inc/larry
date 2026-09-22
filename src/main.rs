//! Line-oriented fixture driver, not the requested native AppCUI view.
use rocco_demo::{Demo, MAX_INPUT};
use std::io::{self, BufRead, Read, Write};

/// Runs the line-oriented fixture protocol over standard input and output.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err("no arguments supported; enter help on stdin".into());
    }
    let mut demo = Demo::default();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut line = Vec::with_capacity(MAX_INPUT + 2);
    writeln!(output, "{}", demo.snapshot()?)?;
    output.flush()?;
    loop {
        line.clear();
        let count = input
            .by_ref()
            .take(u64::try_from(MAX_INPUT.saturating_add(3))?)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.len() > MAX_INPUT {
            return Err("DEMO command exceeds 4096 bytes".into());
        }
        let command = std::str::from_utf8(&line)?;
        match demo.command(command) {
            Ok(false) => break,
            Ok(true) => {}
            Err(error) => writeln!(io::stderr().lock(), "{error}")?,
        }
        writeln!(output, "{}", demo.snapshot()?)?;
        output.flush()?;
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if writeln!(io::stderr().lock(), "DEMO failed: {error}").is_err() {
                return std::process::ExitCode::FAILURE;
            }
            std::process::ExitCode::FAILURE
        }
    }
}
