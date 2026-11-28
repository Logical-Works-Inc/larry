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

/// The one temp guard of the integration tests (larry#267).
#[cfg(test)]
#[path = "../../../tests/support/temp_dir.rs"]
mod temp_dir;

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;

    use larry_demo::governance::TrackerError;

    use super::temp_dir::TempDir;
    use super::{GovernanceRun, governance_args, governance_once, tracker_once};

    /// Command-line words from string slices.
    fn os(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    /// `flags` followed by `--root root`.
    fn rooted(flags: &[&str], root: &Path) -> Vec<OsString> {
        let mut words = os(flags);
        words.push(OsString::from("--root"));
        words.push(root.as_os_str().to_os_string());
        words
    }

    /// The flag and root parsed from `words`, or the refusal's line.
    fn parsed(words: &[OsString], flag: &str) -> Result<(bool, PathBuf), String> {
        governance_args(words, flag).map_err(|refusal| refusal.to_string())
    }

    /// Requirement: argument parsing admits exactly `[FLAG] [--root DIR]` in any order. Source:
    /// ADR 0010, CWE-88 (argument injection). Adversarial: duplicated flags, a missing value,
    /// the other command's flag, unknown words and a root that is not UTF-8.
    #[test]
    fn sim_governance_args_admit_only_the_documented_shapes() -> std::io::Result<()> {
        let here = std::env::current_dir()?;
        let root = PathBuf::from("/some/root");
        assert_eq!(parsed(&[], "--report"), Ok((false, here.clone())));
        assert_eq!(parsed(&os(&["--check"]), "--check"), Ok((true, here)));
        for words in [
            rooted(&["--report"], &root),
            os(&["--root", "/some/root", "--report"]),
        ] {
            assert_eq!(
                parsed(&words, "--report"),
                Ok((true, root.clone())),
                "{words:?}"
            );
        }
        for words in [
            os(&["--check"]),
            os(&["--report", "--report"]),
            os(&["--root", "a", "--root", "b"]),
            os(&["--root"]),
            os(&["--report", "--root"]),
            os(&["--nope"]),
            os(&["--report", "extra"]),
            os(&[""]),
        ] {
            let refused = governance_args(&words, "--report");
            assert!(
                matches!(&refused, Err(GovernanceRun::Usage(named)) if *named == words),
                "{words:?} must be usage naming itself: {refused:?}"
            );
        }
        // A root that is not UTF-8 is kept byte for byte, never dropped or replaced.
        let raw = OsString::from_vec(vec![0xff, 0xfe]);
        let words = vec![OsString::from("--root"), raw.clone()];
        assert_eq!(parsed(&words, "--report"), Ok((false, PathBuf::from(raw))));
        Ok(())
    }

    /// Requirement: a tree that fails the checker exits 1, plain and `--report`, and a usage
    /// error never runs the checker. Source: ADR 0010. Adversarial: an empty directory, where
    /// every governed document is missing.
    #[test]
    fn sim_governance_once_refuses_a_broken_tree() -> std::io::Result<()> {
        let empty = TempDir::new()?;
        for flags in [&[][..], &["--report"]] {
            let outcome = governance_once(&rooted(flags, &empty));
            assert!(
                matches!(outcome, Ok(code) if code == ExitCode::FAILURE),
                "{flags:?}: {outcome:?}"
            );
        }
        for flags in [&["--bogus"][..], &["--report", "--report"]] {
            let refused = governance_once(&rooted(flags, &empty));
            assert!(
                matches!(refused, Err(GovernanceRun::Usage(_))),
                "{flags:?}: {refused:?}"
            );
        }
        Ok(())
    }

    /// Requirement: the shipped tree passes through the same entry the binary calls. Source:
    /// ADR 0010; CI runs `larry governance` on this tree.
    #[test]
    fn sim_governance_once_passes_the_shipped_tree() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        for flags in [&[][..], &["--report"]] {
            let outcome = governance_once(&rooted(flags, manifest));
            assert!(
                matches!(outcome, Ok(code) if code == ExitCode::SUCCESS),
                "{flags:?}: {outcome:?}"
            );
        }
    }

    /// Requirement: `larry tracker` writes the page, `--check` passes only on the current render,
    /// and a missing input is a typed tracker refusal. Source: ADR 0010. Adversarial: no data
    /// file, a page that was never written and a page a hand edited after the render.
    #[test]
    fn sim_tracker_once_judges_missing_current_and_stale_pages() -> std::io::Result<()> {
        let tree = TempDir::new()?;
        for flags in [&[][..], &["--check"]] {
            let refused = tracker_once(&rooted(flags, &tree));
            assert!(
                matches!(
                    refused,
                    Err(GovernanceRun::Tracker(TrackerError::Read { .. }))
                ),
                "{flags:?}: {refused:?}"
            );
        }
        let refused = tracker_once(&rooted(&["--check", "--check"], &tree));
        assert!(
            matches!(refused, Err(GovernanceRun::Usage(_))),
            "{refused:?}"
        );
        let directory = tree.join("tools").join("tracker");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join("tracker.json"), "{}")?;
        std::fs::write(directory.join("template.html"), "<p>__TRACKER_DATA__</p>")?;
        let judged = |flags: &[&str]| tracker_once(&rooted(flags, &tree));
        let stale = judged(&["--check"]);
        assert!(matches!(stale, Err(GovernanceRun::Stale)), "{stale:?}");
        let wrote = judged(&[]);
        assert!(
            matches!(wrote, Ok(code) if code == ExitCode::SUCCESS),
            "{wrote:?}"
        );
        let page = directory.join("index.html");
        assert_eq!(std::fs::read_to_string(&page)?, "<p>{}</p>");
        let current = judged(&["--check"]);
        assert!(
            matches!(current, Ok(code) if code == ExitCode::SUCCESS),
            "{current:?}"
        );
        std::fs::write(&page, "<p>edited by hand</p>")?;
        let stale = judged(&["--check"]);
        assert!(matches!(stale, Err(GovernanceRun::Stale)), "{stale:?}");
        assert_eq!(std::fs::read_to_string(&page)?, "<p>edited by hand</p>");
        Ok(())
    }

    /// Every refusal prints its exact line, so a renamed variant or a reworded line fails here.
    #[test]
    fn sim_every_governance_cli_refusal_names_its_cause() {
        let broken = || std::io::Error::other("cause");
        let cases = [
            (
                GovernanceRun::Usage(vec![OsString::from("x")]),
                "usage: larry governance [--report] [--root DIR] | larry tracker [--check] [--root DIR] (got [\"x\"])",
            ),
            (
                GovernanceRun::NoDirectory(broken()),
                "governance: no working directory: cause",
            ),
            (
                GovernanceRun::Stdout(broken()),
                "governance: stdout unwritable: cause",
            ),
            (
                GovernanceRun::Stderr(broken()),
                "governance: stderr unwritable: cause",
            ),
            (
                GovernanceRun::Stale,
                "FAIL: tools/tracker/index.html is stale — run `larry tracker`",
            ),
        ];
        for (refusal, line) in cases {
            assert_eq!(refusal.to_string(), line);
        }
    }

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
        assert!(
            GovernanceRun::Tracker(TrackerError::MissingToken)
                .source()
                .is_some(),
            "a tracker refusal"
        );
        assert!(
            GovernanceRun::Usage(Vec::new()).source().is_none(),
            "usage names its arguments, never a cause"
        );
    }
}
