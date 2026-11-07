//! `larry-local-ci` (`cargo local-ci`): the local mirror of every gate AGENTS.md
//! declares, in its order. It replaced `.wwfd/local-ci.sh`, because this
//! repository takes no shell source (larry#53, ADR 0010). Every lane runs every
//! time; a red lane is reported and no receipt is written. The receipt names the
//! head it attests, so it cannot vouch for a later commit. `--list` prints the
//! lanes as JSON and runs nothing.
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use lgwks_std::json::{Value, json};

/// One gate: its name, the program and arguments it runs, extra environment,
/// and, for a journey lane, how many `... ok` lines its output must hold.
struct Lane {
    /// The lane's name in the log.
    name: &'static str,
    /// The program, a `target/debug/` path or a name found on `PATH`.
    program: &'static str,
    /// The program's arguments.
    args: &'static [&'static str],
    /// Environment set for this lane alone.
    env: &'static [(&'static str, &'static str)],
    /// The fewest passing journeys the output must report; `None` for no count.
    journeys: Option<usize>,
}

/// Python stays only in the lanes larry#53 has not ported yet: the reranker
/// controls (step 3) and the demo and native PTY journeys (step 2).
const LANES: [Lane; 11] = [
    Lane {
        name: "build",
        program: "cargo",
        args: &["build", "--bins", "--locked"],
        env: &[],
        journeys: None,
    },
    Lane {
        name: "tests",
        program: "cargo",
        args: &["nextest", "run", "--workspace", "--locked"],
        env: &[],
        journeys: None,
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
        env: &[],
        journeys: None,
    },
    Lane {
        name: "fmt",
        program: "cargo",
        args: &["fmt", "--all", "--", "--check"],
        env: &[],
        journeys: None,
    },
    Lane {
        name: "governance",
        program: "target/debug/larry",
        args: &["governance"],
        env: &[],
        journeys: None,
    },
    Lane {
        name: "tracker",
        program: "target/debug/larry",
        args: &["tracker", "--check"],
        env: &[],
        journeys: None,
    },
    Lane {
        name: "reranker",
        program: "python3",
        args: &["-m", "unittest", "discover", "-s", "tests"],
        env: &[("PYTHONDONTWRITEBYTECODE", "1")],
        journeys: None,
    },
    Lane {
        name: "deps",
        program: "lgwks-deps",
        args: &["check", "."],
        env: &[],
        journeys: None,
    },
    Lane {
        name: "demo-cli",
        program: "python3",
        args: &["tools/test_demo_cli.py"],
        env: &[("PYTHONDONTWRITEBYTECODE", "1")],
        journeys: None,
    },
    // Colour off: a host FORCE_COLOR wraps "ok" in ANSI escapes and the count
    // would read 0 against a green run.
    Lane {
        name: "native-pty",
        program: "python3",
        args: &["tools/test_native_cli.py"],
        env: &[
            ("NO_COLOR", "1"),
            ("PYTHON_COLORS", "0"),
            ("FORCE_COLOR", ""),
            ("PYTHONDONTWRITEBYTECODE", "1"),
        ],
        journeys: Some(2),
    },
    Lane {
        name: "whitespace",
        program: "git",
        args: &["diff", "--check"],
        env: &[],
        journeys: None,
    },
];

/// Why the run stopped without a receipt.
#[derive(Debug)]
enum LocalCiError {
    /// An argument other than `--list`.
    Usage(OsString),
    /// `git` ran but could not name the checkout, its branch or its head.
    Git(&'static str),
    /// A lane's program, or `git`, could not be started or read.
    Spawn {
        /// The lane.
        lane: &'static str,
        /// The cause.
        cause: io::Error,
    },
    /// A lane exited non-zero or was killed.
    Red {
        /// The lane.
        lane: &'static str,
        /// Its exit code; `None` when a signal ended it.
        code: Option<i32>,
    },
    /// A journey lane passed while reporting too few journeys.
    Vacuous {
        /// The lane.
        lane: &'static str,
        /// The `... ok` lines found.
        found: usize,
        /// The fewest required.
        needed: usize,
    },
    /// One or more lanes were red; each was reported as it failed.
    Failed(Vec<&'static str>),
    /// The log could not be written.
    Log(io::Error),
    /// `HOME` is unset, so the receipt has no home.
    Home,
    /// The clock reads before 1970.
    Clock(SystemTimeError),
    /// The receipt could not be written.
    Receipt {
        /// The receipt file.
        path: PathBuf,
        /// The cause.
        cause: io::Error,
    },
}

impl fmt::Display for LocalCiError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usage(ref arg) => write!(
                out,
                "unknown argument {arg:?}; usage: larry-local-ci [--list]"
            ),
            Self::Git(what) => write!(out, "git could not report {what}"),
            Self::Spawn { lane, ref cause } => write!(out, "lane {lane} did not start: {cause}"),
            Self::Red {
                lane,
                code: Some(code),
            } => write!(out, "lane {lane} failed with exit {code}"),
            Self::Red { lane, code: None } => write!(out, "lane {lane} was killed by a signal"),
            Self::Vacuous {
                lane,
                found,
                needed,
            } => {
                write!(
                    out,
                    "lane {lane} reported {found} journeys, expected {needed}"
                )
            }
            Self::Failed(ref lanes) => {
                write!(out, "red lanes: {}; no receipt written", lanes.join(", "))
            }
            Self::Log(ref cause) => write!(out, "log unwritable: {cause}"),
            Self::Home => out.write_str("HOME is unset; the receipt has nowhere to go"),
            Self::Clock(ref cause) => write!(out, "the clock reads before 1970: {cause}"),
            Self::Receipt {
                ref path,
                ref cause,
            } => write!(out, "receipt {} unwritable: {cause}", path.display()),
        }
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

/// How many lines of `output` (both streams) end in `... ok`.
fn passing_journeys(output: &Output) -> usize {
    let count = |stream: &[u8]| {
        String::from_utf8_lossy(stream)
            .lines()
            .filter(|line| line.trim_end().ends_with("... ok"))
            .count()
    };
    count(&output.stdout).saturating_add(count(&output.stderr))
}

/// Runs one lane from the checkout root; a journey lane's output is captured,
/// echoed and counted.
fn run(lane: &Lane) -> Result<(), LocalCiError> {
    log(&command_line(lane))?;
    let mut command = Command::new(program(lane));
    command.args(lane.args);
    for &(key, value) in lane.env {
        command.env(key, value);
    }
    let spawned = |cause| LocalCiError::Spawn {
        lane: lane.name,
        cause,
    };
    let (status, found) = match lane.journeys {
        None => (command.status().map_err(spawned)?, None),
        Some(_) => {
            let output = command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(spawned)?;
            io::stdout()
                .lock()
                .write_all(&output.stdout)
                .map_err(LocalCiError::Log)?;
            io::stderr()
                .lock()
                .write_all(&output.stderr)
                .map_err(LocalCiError::Log)?;
            (output.status, Some(passing_journeys(&output)))
        }
    };
    if !status.success() {
        return Err(LocalCiError::Red {
            lane: lane.name,
            code: status.code(),
        });
    }
    match (lane.journeys, found) {
        (Some(needed), Some(found)) if found < needed => Err(LocalCiError::Vacuous {
            lane: lane.name,
            found,
            needed,
        }),
        _ => Ok(()),
    }
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

/// Writes the receipt for `root` at `head` beside the guard's state, atomically.
fn write_receipt(root: &Path, branch: &str, head: &str) -> Result<PathBuf, LocalCiError> {
    let home = std::env::var_os("HOME").ok_or(LocalCiError::Home)?;
    let dir = PathBuf::from(home).join("wwfd").join("state");
    let path = dir.join("local-ci-receipt.json");
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
    fs::create_dir_all(&dir).map_err(failed)?;
    let staged = dir.join(format!("local-ci-receipt.json.tmp-{}", std::process::id()));
    fs::write(&staged, format!("{receipt}\n")).map_err(failed)?;
    fs::rename(&staged, &path).map_err(failed)?;
    Ok(path)
}

/// Runs every lane in order from the checkout root, reporting each red one, then writes the
/// receipt only when every lane passed.
fn local_ci() -> Result<(), LocalCiError> {
    let root = PathBuf::from(git(&["rev-parse", "--show-toplevel"], "the checkout root")?);
    std::env::set_current_dir(&root).map_err(|cause| LocalCiError::Spawn {
        lane: "root",
        cause,
    })?;
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
        return Err(LocalCiError::Failed(red));
    }
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"], "the branch")?;
    let head = git(&["rev-parse", "HEAD"], "the head")?;
    let path = write_receipt(&root, &branch, &head)?;
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

/// Writes one diagnostic line to standard error.
fn diagnose(message: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(b"local-ci: ")?;
    stderr.write_all(message.as_bytes())?;
    stderr.write_all(b"\n")
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let outcome = match (args.next(), args.next()) {
        (None, _) => local_ci(),
        (Some(flag), None) if flag == "--list" => list(),
        (Some(flag), _) => Err(LocalCiError::Usage(flag)),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if diagnose(&error.to_string()).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::FAILURE
        }
    }
}
