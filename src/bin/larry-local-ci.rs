//! `larry-local-ci` (`cargo local-ci`): the local mirror of every gate AGENTS.md
//! declares, in its order. It replaced `.wwfd/local-ci.sh`, because this
//! repository takes no shell source (larry#53, ADR 0010). Every lane runs every
//! time; a red lane is reported and no receipt is written. The receipt names the
//! head it attests, so it cannot vouch for a later commit, and a head that moved
//! while the lanes ran gets none (ADR 0134). `--list` prints the lanes as JSON
//! and runs nothing.
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use lgwks_macros::Error;
use lgwks_std::json::{Value, json};

/// One gate: its name and the program and arguments it runs.
struct Lane {
    /// The lane's name in the log.
    name: &'static str,
    /// The program, a `target/debug/` path or a name found on `PATH`.
    program: &'static str,
    /// The program's arguments.
    args: &'static [&'static str],
}

/// No lane runs Python (larry#53): the demo and PTY journeys (step 2, ADR 0011)
/// and the reranker's controls (step 3, ADR 0013) run in the tests lane; the
/// reranker lane lints its feature-gated engine (larry#150), and the mascot
/// lane checks the committed render (step 4, ADR 0014).
const LANES: [Lane; 10] = [
    Lane {
        name: "build",
        program: "cargo",
        args: &["build", "--bins", "--locked"],
    },
    Lane {
        name: "tests",
        program: "cargo",
        args: &["nextest", "run", "--workspace", "--locked"],
    },
    Lane {
        name: "clippy",
        program: "cargo",
        args: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    },
    Lane {
        name: "fmt",
        program: "cargo",
        args: &["fmt", "--all", "--", "--check"],
    },
    Lane {
        name: "governance",
        program: "target/debug/larry",
        args: &["governance"],
    },
    Lane {
        name: "tracker",
        program: "target/debug/larry",
        args: &["tracker", "--check"],
    },
    Lane {
        name: "mascot",
        program: "target/debug/larry",
        args: &["mascot", "--check"],
    },
    Lane {
        name: "reranker",
        program: "cargo",
        args: &[
            "clippy",
            "--locked",
            "--features",
            "rerank",
            "--bin",
            "larry-rerank",
            "--",
            "-D",
            "warnings",
        ],
    },
    Lane {
        name: "deps",
        program: "lgwks-deps",
        args: &["check", "."],
    },
    Lane {
        name: "whitespace",
        program: "git",
        args: &["diff", "--check"],
    },
];

/// Why the run stopped without a receipt.
#[derive(Debug, Error)]
enum LocalCiError {
    /// An argument outside the local CI command line.
    #[error("unknown argument {0:?}; usage: larry-local-ci [--list | --receipts <dir>]")]
    Usage(OsString),
    /// `--receipts` was supplied without its directory.
    #[error("--receipts needs a directory; usage: larry-local-ci [--list | --receipts <dir>]")]
    MissingReceipts,
    /// `git` ran but could not name the checkout, its branch or its head.
    #[error("git could not report {0}")]
    Git(&'static str),
    /// A lane's program, or `git`, could not be started or read.
    #[error("lane {lane} did not start: {cause}")]
    Spawn {
        /// The lane.
        lane: &'static str,
        /// The cause.
        #[source]
        cause: io::Error,
    },
    /// A lane exited non-zero or was killed.
    #[error("lane {lane} {}", ended(*code))]
    Red {
        /// The lane.
        lane: &'static str,
        /// Its exit code; `None` when a signal ended it.
        code: Option<i32>,
    },
    /// One or more lanes were red; each was reported as it failed.
    #[error("red lanes: {}; no receipt written", lanes.join(", "))]
    Failed {
        /// The red lanes, in lane order.
        lanes: Vec<&'static str>,
    },
    /// A commit, reset or switch moved the head while the lanes ran, so no head was tested.
    #[error("head moved from {started} to {ended} while the lanes ran; no receipt written")]
    Moved {
        /// The head before the first lane.
        started: String,
        /// The head after the last.
        ended: String,
    },
    /// The log could not be written.
    #[error("log unwritable: {0}")]
    Log(#[source] io::Error),
    /// `LARRY_STATE_DIR` is not Unicode, so it cannot be a configured receipt directory.
    #[error("{variable} is not valid UTF-8")]
    InvalidDirectory {
        /// The flag or environment variable that supplied the invalid path.
        variable: &'static str,
    },
    /// A configured receipt directory is empty.
    #[error("{variable} is empty")]
    EmptyDirectory {
        /// The flag or environment variable that supplied the empty path.
        variable: &'static str,
    },
    /// The clock reads before 1970.
    #[error("the clock reads before 1970: {0}")]
    Clock(#[source] SystemTimeError),
    /// The receipt could not be written.
    #[error("receipt {} unwritable: {cause}", path.display())]
    Receipt {
        /// The receipt file.
        path: PathBuf,
        /// The cause.
        #[source]
        cause: io::Error,
    },
}

/// How a red lane ended: with an exit code, or killed by a signal.
fn ended(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("failed with exit {code}"),
        None => "was killed by a signal".to_owned(),
    }
}

/// Writes one log line to standard output.
fn log(line: &str) -> Result<(), LocalCiError> {
    let mut out = io::stdout().lock();
    writeln!(out, "\nlocal-ci: {line}")
        .and_then(|()| out.flush())
        .map_err(LocalCiError::Log)
}

/// The program a lane runs: a `target/` path moves under `CARGO_TARGET_DIR`
/// when that is set, so a relocated target directory still finds the binary.
fn program(lane: &Lane) -> PathBuf {
    match (
        lane.program.strip_prefix("target/"),
        std::env::var_os("CARGO_TARGET_DIR"),
    ) {
        (Some(rest), Some(target)) => PathBuf::from(target).join(rest),
        (Some(_) | None, _) => PathBuf::from(lane.program),
    }
}

/// The lane's command line as one string, for the log and `--list`.
fn command_line(lane: &Lane) -> String {
    let mut line = String::from(lane.program);
    for arg in lane.args {
        line.push(' ');
        line.push_str(arg);
    }
    line
}

/// Runs one lane from the checkout root.
fn run(lane: &Lane) -> Result<(), LocalCiError> {
    log(&command_line(lane))?;
    let status = Command::new(program(lane))
        .args(lane.args)
        .status()
        .map_err(|cause| LocalCiError::Spawn {
            lane: lane.name,
            cause,
        })?;
    if status.success() {
        return Ok(());
    }
    Err(LocalCiError::Red {
        lane: lane.name,
        code: status.code(),
    })
}

/// The one line `git` prints for `args`, trimmed.
fn git(args: &[&str], what: &'static str) -> Result<String, LocalCiError> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|cause| LocalCiError::Spawn { lane: "git", cause })?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.trim();
    if !output.status.success() || line.is_empty() {
        return Err(LocalCiError::Git(what));
    }
    Ok(line.to_owned())
}

/// The receipt's file name in the selected receipt directory.
const RECEIPT: &str = "local-ci-receipt.json";

/// Resolves the receipt directory before git or any lane runs. The flag wins over
/// `LARRY_STATE_DIR`, which wins over `CARGO_TARGET_DIR/local-ci`; each is made absolute against
/// the caller's directory, so a relative one still means where the caller stood once the run
/// moves to the checkout root. `None` is the checkout root's own `target/local-ci`.
fn receipt_directory(
    receipts: Option<&OsStr>,
    state_dir: Option<&OsStr>,
    target_dir: Option<&OsStr>,
) -> Result<Option<PathBuf>, LocalCiError> {
    let chosen = match (receipts, state_dir, target_dir) {
        (Some(path), _, _) if path.is_empty() => {
            return Err(LocalCiError::EmptyDirectory {
                variable: "--receipts",
            });
        }
        (Some(path), _, _) => PathBuf::from(path),
        (None, Some(path), _) => {
            let text = path.to_str().ok_or(LocalCiError::InvalidDirectory {
                variable: "LARRY_STATE_DIR",
            })?;
            if text.is_empty() {
                return Err(LocalCiError::EmptyDirectory {
                    variable: "LARRY_STATE_DIR",
                });
            }
            PathBuf::from(text)
        }
        (None, None, Some(target)) => PathBuf::from(target).join("local-ci"),
        (None, None, None) => return Ok(None),
    };
    std::path::absolute(&chosen)
        .map(Some)
        .map_err(|cause| LocalCiError::Receipt {
            path: chosen,
            cause,
        })
}

/// Writes the receipt for `root` at `head` in `dir`, atomically and durably through the
/// library's one replace (larry#434).
fn write_receipt(
    dir: &Path,
    root: &Path,
    branch: &str,
    head: &str,
) -> Result<PathBuf, LocalCiError> {
    let path = dir.join(RECEIPT);
    let failed = |cause| LocalCiError::Receipt {
        path: path.clone(),
        cause,
    };
    let attested = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(LocalCiError::Clock)?
        .as_secs();
    let receipt: Value = json!({
        "repo_root": root.display().to_string(),
        "branch": branch,
        "head_sha": head,
        "attested_at": attested,
        "ci_script": "cargo local-ci",
    });
    fs::create_dir_all(dir).map_err(failed)?;
    larry_demo::replace_file(dir, RECEIPT, format!("{receipt}\n").as_bytes())
        .map_err(|(path, cause)| LocalCiError::Receipt { path, cause })?;
    Ok(path)
}

/// The local CI command line after bounded parsing.
enum LocalCiCommand {
    /// Run every lane, optionally writing the receipt at this path.
    Run(Option<OsString>),
    /// Print the lane table without running it.
    List,
}

/// Parses the local CI command line without consulting the environment.
fn parse_args(mut args: impl Iterator<Item = OsString>) -> Result<LocalCiCommand, LocalCiError> {
    match args.next() {
        None => Ok(LocalCiCommand::Run(None)),
        Some(flag) if flag == "--list" && args.next().is_none() => Ok(LocalCiCommand::List),
        Some(flag) if flag == "--receipts" => match (args.next(), args.next()) {
            (Some(path), None) => Ok(LocalCiCommand::Run(Some(path))),
            (None, _) => Err(LocalCiError::MissingReceipts),
            (Some(_), Some(extra)) => Err(LocalCiError::Usage(extra)),
        },
        Some(flag) => Err(LocalCiError::Usage(flag)),
    }
}

/// Runs every lane in order from the checkout root, reporting each red one, then writes the
/// receipt only when every lane passed and the head is the one the first lane started on. A head
/// git cannot name before the lanes is refused after them, so every lane still runs.
fn local_ci(receipts: Option<&OsStr>) -> Result<(), LocalCiError> {
    let chosen = receipt_directory(
        receipts,
        std::env::var_os("LARRY_STATE_DIR").as_deref(),
        std::env::var_os("CARGO_TARGET_DIR").as_deref(),
    )?;
    let root = PathBuf::from(git(&["rev-parse", "--show-toplevel"], "the checkout root")?);
    std::env::set_current_dir(&root).map_err(|cause| LocalCiError::Spawn {
        lane: "root",
        cause,
    })?;
    let directory = match chosen {
        Some(directory) => directory,
        None => root.join("target").join("local-ci"),
    };
    let started = git(&["rev-parse", "HEAD"], "the head");
    let mut red = Vec::new();
    for lane in &LANES {
        match run(lane) {
            Ok(()) => {}
            Err(LocalCiError::Log(cause)) => return Err(LocalCiError::Log(cause)),
            Err(error) => {
                log(&format!("RED {error}"))?;
                red.push(lane.name);
            }
        }
    }
    if !red.is_empty() {
        return Err(LocalCiError::Failed { lanes: red });
    }
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"], "the branch")?;
    let head = git(&["rev-parse", "HEAD"], "the head")?;
    let started = started?;
    if started != head {
        return Err(LocalCiError::Moved {
            started,
            ended: head,
        });
    }
    let path = write_receipt(&directory, &root, &branch, &head)?;
    let short = match head.get(..7) {
        Some(prefix) => prefix,
        None => head.as_str(),
    };
    log(&format!(
        "OK, every declared lane passed; receipt {} for {short}",
        path.display()
    ))
}

/// `--list`: every lane as JSON, in order; nothing runs.
fn list() -> Result<(), LocalCiError> {
    let lanes: Vec<Value> = LANES
        .iter()
        .map(|lane| json!({ "name": lane.name, "command": command_line(lane) }))
        .collect();
    let mut out = io::stdout().lock();
    writeln!(out, "{}", Value::Array(lanes))
        .and_then(|()| out.flush())
        .map_err(LocalCiError::Log)
}

fn main() -> ExitCode {
    let outcome = match parse_args(std::env::args_os().skip(1)) {
        Ok(LocalCiCommand::Run(receipts)) => local_ci(receipts.as_deref()),
        Ok(LocalCiCommand::List) => list(),
        Err(error) => Err(error),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if larry_demo::diagnose("local-ci", &error.to_string()).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::FAILURE
        }
    }
}

/// The integration tests' one temp guard (larry#267), so the receipt test writes under a fresh
/// directory it removes, never under the checkout.
#[cfg(test)]
#[path = "../../tests/support/temp_dir.rs"]
mod temp_dir;

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf, absolute};

    use lgwks_std::json::{Value, from_str};

    use super::temp_dir::TempDir;
    use super::{
        LocalCiCommand, LocalCiError, RECEIPT, parse_args, receipt_directory, write_receipt,
    };

    type Check = Result<(), Box<dyn std::error::Error>>;

    /// A hostile branch name (quotes, a newline, a backslash: CWE-116) reaches the receipt as
    /// data: it reads back byte for byte and forges no member.
    #[test]
    fn a_state_dir_receipt_is_written_under_the_requested_directory() -> Check {
        let dir = TempDir::new()?;
        let state = dir.join("state");
        let chosen = receipt_directory(None, Some(state.as_os_str()), None)?;
        assert_eq!(chosen.as_deref(), Some(state.as_path()), "the state dir");
        let hostile = "a\",\"head_sha\":\"forged\n\\";
        let written = write_receipt(&state, Path::new("repo"), hostile, "head")?;
        assert_eq!(written, state.join(RECEIPT), "the receipt's path");
        let receipt: Value = from_str(&std::fs::read_to_string(&written)?)?;
        let member = |name: &str| receipt.get(name).and_then(Value::as_str);
        assert_eq!(member("repo_root"), Some("repo"), "the root");
        assert_eq!(member("branch"), Some(hostile), "the branch, verbatim");
        assert_eq!(member("head_sha"), Some("head"), "the head is not forged");
        assert_eq!(member("ci_script"), Some("cargo local-ci"), "the gate");
        Ok(())
    }

    #[test]
    fn a_receipts_flag_wins_over_the_state_environment() -> Check {
        let chosen = receipt_directory(
            Some(OsStr::new("flag")),
            Some(OsStr::new("state")),
            Some(OsStr::new("target")),
        )?;
        assert_eq!(chosen, Some(absolute("flag")?));
        let chosen = receipt_directory(None, Some(OsStr::new("state")), Some(OsStr::new("t")))?;
        assert_eq!(chosen, Some(absolute("state")?));
        Ok(())
    }

    /// Each relative source means the caller's directory, before the run moves to the root.
    #[test]
    fn a_relative_receipt_directory_is_absolute_against_the_callers_directory() -> Check {
        for (flag, state, target, want) in [
            (Some("r"), None, None, PathBuf::from("r")),
            (None, Some("s"), None, PathBuf::from("s")),
            (None, None, Some("t"), Path::new("t").join("local-ci")),
        ] {
            let chosen = receipt_directory(
                flag.map(OsStr::new),
                state.map(OsStr::new),
                target.map(OsStr::new),
            )?;
            assert_eq!(chosen, Some(std::env::current_dir()?.join(want)));
        }
        Ok(())
    }

    /// With no flag and no variable the checkout root's `target/local-ci` is chosen later.
    #[test]
    fn the_default_receipt_directory_is_left_to_the_checkout_root() -> Check {
        assert_eq!(receipt_directory(None, None, None)?, None);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_non_unicode_state_environment_is_refused_by_name() {
        use std::os::unix::ffi::OsStrExt;

        let result = receipt_directory(None, Some(OsStr::from_bytes(b"state\xff")), None);
        assert!(
            matches!(
                result,
                Err(LocalCiError::InvalidDirectory {
                    variable: "LARRY_STATE_DIR"
                })
            ),
            "the typed refusal"
        );
    }

    #[test]
    fn the_receipts_flag_requires_one_directory() {
        let args = [OsStr::new("--receipts")]
            .into_iter()
            .map(OsStr::to_os_string);
        assert!(matches!(
            parse_args(args),
            Err(LocalCiError::MissingReceipts)
        ));
        let args = [OsStr::new("--receipts"), OsStr::new("state")]
            .into_iter()
            .map(OsStr::to_os_string);
        assert!(
            matches!(parse_args(args), Ok(LocalCiCommand::Run(Some(path))) if path == "state"),
            "the directory is the word after the flag"
        );
    }
}
