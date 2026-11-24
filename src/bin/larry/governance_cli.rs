//! `larry governance` and `larry tracker`, the document gate (larry#53, ADR 0010): their
//! arguments, their output and the typed reason either stops without a verdict. Split from
//! `src/bin/larry.rs` by responsibility when the help table took the dispatch (larry#295,
//! ADR 0065); the table rows in that file call [`governance_once`] and [`tracker_once`].

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use larry_demo::governance;
use lgwks_macros::Error;

use super::diagnose;

/// Why `larry governance` or `larry tracker` stopped without its verdict; each keeps its cause.
#[derive(Debug, Error)]
pub(super) enum GovernanceRun {
    /// The arguments were not `[FLAG] [--root DIR]`.
    #[error(
        "usage: larry governance [--report] [--root DIR] | larry tracker [--check] [--root DIR] (got {0:?})"
    )]
    Usage(Vec<OsString>),
    /// No root was given and the working directory is unreadable.
    #[error("governance: no working directory: {0}")]
    NoDirectory(#[source] io::Error),
    /// Standard output could not be written.
    #[error("governance: stdout unwritable: {0}")]
    Stdout(#[source] io::Error),
    /// Standard error could not be written.
    #[error("governance: stderr unwritable: {0}")]
    Stderr(#[source] io::Error),
    /// `--check` found the committed page differs from the render.
    #[error("FAIL: tools/tracker/index.html is stale — run `larry tracker`")]
    Stale,
    /// The tracker could not read, render or write.
    #[error("FAIL: {0}")]
    Tracker(#[source] governance::TrackerError),
}

/// The root and flag `larry governance` and `larry tracker` take: `[FLAG] [--root DIR]`, in any
/// order, the root defaulting to the working directory.
fn governance_args(rest: &[OsString], flag: &str) -> Result<(bool, PathBuf), GovernanceRun> {
    let usage = || GovernanceRun::Usage(rest.to_vec());
    let mut set = false;
    let mut root: Option<PathBuf> = None;
    let mut items = rest.iter();
    while let Some(item) = items.next() {
        if item == flag && !set {
            set = true;
        } else if item == "--root" && root.is_none() {
            let Some(directory) = items.next() else {
                return Err(usage());
            };
            root = Some(PathBuf::from(directory));
        } else {
            return Err(usage());
        }
    }
    let root = match root {
        Some(root) => root,
        None => std::env::current_dir().map_err(GovernanceRun::NoDirectory)?,
    };
    Ok((set, root))
}

/// Writes each line to standard output and flushes.
fn print_lines(lines: &[&str]) -> Result<(), GovernanceRun> {
    let mut out = io::stdout().lock();
    for line in lines {
        out.write_all(line.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .map_err(GovernanceRun::Stdout)?;
    }
    out.flush().map_err(GovernanceRun::Stdout)
}

/// `larry governance [--report] [--root DIR]`: the structural checker (ADR 0010). Prints the
/// PASS block or the JSON report, or each error as a `FAIL:` line on standard error; exits 1
/// on any error.
pub(super) fn governance_once(rest: &[OsString]) -> Result<ExitCode, GovernanceRun> {
    let (report_wanted, root) = governance_args(rest, "--report")?;
    let report: governance::Report = governance::check(&root);
    if report_wanted {
        print_lines(&[&report.pretty()])?;
    } else if report.ok() {
        print_lines(&[
            "PASS: document structure and governance inventory",
            &report.counts_line(),
            "NOT CHECKED BY THIS GATE: Mermaid rendering",
            "NOT RUN: runtime, model evaluations, hardware and human trials",
        ])?;
    } else {
        for error in report.errors() {
            diagnose(&format!("FAIL: {error}")).map_err(GovernanceRun::Stderr)?;
        }
    }
    Ok(if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `larry tracker [--check] [--root DIR]`: writes `tools/tracker/index.html` atomically, or with
/// `--check` exits 1 unless the committed page equals the render.
pub(super) fn tracker_once(rest: &[OsString]) -> Result<ExitCode, GovernanceRun> {
    let (check_only, root) = governance_args(rest, "--check")?;
    if check_only {
        let current: Result<bool, governance::TrackerError> =
            governance::tracker::is_current(&root);
        return match current {
            Ok(true) => print_lines(&["PASS: tools/tracker/index.html is current"])
                .map(|()| ExitCode::SUCCESS),
            Ok(false) => Err(GovernanceRun::Stale),
            Err(error) => Err(GovernanceRun::Tracker(error)),
        };
    }
    let written: Result<usize, governance::TrackerError> = governance::tracker::write(&root);
    match written {
        Ok(bytes) => print_lines(&[&format!("wrote tools/tracker/index.html ({bytes} bytes)")])
            .map(|()| ExitCode::SUCCESS),
        Err(error) => Err(GovernanceRun::Tracker(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::GovernanceRun;

    /// Every refusal that has a cause keeps it as a source, never an empty `Error` impl
    /// (larry#343).
    #[test]
    fn every_governance_refusal_keeps_its_source() {
        use std::error::Error as _;

        let missing =
            GovernanceRun::NoDirectory(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(missing.source().is_some(), "no working directory");
        let stdout = GovernanceRun::Stdout(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        assert!(stdout.source().is_some(), "stdout");
        let stderr = GovernanceRun::Stderr(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        assert!(stderr.source().is_some(), "stderr");
        assert!(
            GovernanceRun::Stale.source().is_none(),
            "a stale page has no cause"
        );
    }
}
