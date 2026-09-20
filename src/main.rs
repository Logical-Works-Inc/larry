//! Line-oriented fixture driver, not the requested native AppCUI view.
use rocco_demo::{Demo, MAX_INPUT};
use std::io::{self, BufRead, Read, Write};

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
            .take((MAX_INPUT + 3) as u64)
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
            eprintln!("DEMO failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
