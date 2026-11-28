//! The rig the `cargo local-ci` journeys drive the shipped binary in (larry#574): a real fixture
//! checkout and a `PATH` that holds nothing but one stub per lane program, so no journey starts the
//! host's cargo, deps checker or `larry`. Each stub appends its command line to one record, then
//! does what the journey's plan says for that line: pass, exit with a code, die by `SIGKILL`, answer
//! with a line, or block on standard input until the journey releases it. The `git` stub passes
//! every call it was not told about to the real git. Nothing here waits on time.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, BufReader, PipeReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use lgwks_std::json::{self, Value, json};

use crate::origin::{TempDir, fixture, git_command};
use crate::{Check, coverage};

/// Every lane AGENTS.md declares, in order: its name and command line. Written out here rather than
/// read from `--list`, so a lane that is dropped, reordered or reworded fails a journey.
pub(crate) const LANES: [(&str, &str); 10] = [
    ("build", "cargo build --bins --locked"),
    ("tests", "cargo nextest run --workspace --locked"),
    (
        "clippy",
        "cargo clippy --workspace --all-targets --locked -- -D warnings",
    ),
    ("fmt", "cargo fmt --all -- --check"),
    ("governance", "target/debug/larry governance"),
    ("tracker", "target/debug/larry tracker --check"),
    ("mascot", "target/debug/larry mascot --check"),
    (
        "reranker",
        "cargo clippy --locked --features rerank --bin larry-rerank -- -D warnings",
    ),
    ("deps", "lgwks-deps check ."),
    ("whitespace", "git diff --check"),
];

/// The receipt's file name.
pub(crate) const RECEIPT: &str = "local-ci-receipt.json";

/// What a blocking stub prints on the log once it is about to wait.
const HELD: &str = "stub-is-held";

/// What an earlier run's receipt holds in the journeys that must leave it alone.
const EARLIER: &str = "an earlier head's receipt\n";

/// The git call that names the checkout, the first thing a run asks.
pub(crate) const ASK_ROOT: &str = "git rev-parse --show-toplevel";

/// The git call that names the branch, asked only after every lane passed.
pub(crate) const ASK_BRANCH: &str = "git rev-parse --abbrev-ref HEAD";

/// The git call that names the head, asked only after every lane passed.
pub(crate) const ASK_HEAD: &str = "git rev-parse HEAD";

/// One checkout and the stub programs a run in it reaches.
pub(crate) struct Rig {
    /// Holds the stub `bin`, the record of what ran and the plan.
    base: TempDir,
    /// The fixture checkout, one commit on `main`.
    repo: TempDir,
    /// The checkout as git names it: every link resolved.
    root: PathBuf,
}

impl Rig {
    /// A fresh checkout with a stub for every lane program and an empty plan.
    pub(crate) fn new() -> Check<Rig> {
        let (_state, repo) = fixture()?;
        let base = TempDir::new()?;
        let root = fs::canonicalize(repo.path())?;
        fs::create_dir(base.join("bin"))?;
        let rig = Rig { base, repo, root };
        rig.plan(&[])?;
        rig.stub(&rig.bin("cargo"), "cargo", "exit 0")?;
        rig.stub(&rig.bin("lgwks-deps"), "lgwks-deps", "exit 0")?;
        // By its own path: the Darwin `git` shim picks its tool from the name it is started by.
        let pass = format!("exec '{}' \"$@\"", real_git()?.display());
        rig.stub(&rig.bin("git"), "git", &pass)?;
        rig.stub_larry(&rig.root.join("target"))?;
        Ok(rig)
    }

    /// The checkout root as the receipt must name it.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// A path of this rig's own, outside the checkout.
    pub(crate) fn beside(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }

    /// The stub `name` on the rig's `PATH`.
    pub(crate) fn bin(&self, name: &str) -> PathBuf {
        self.base.join("bin").join(name)
    }

    /// Puts the `target/debug/larry` stub under `target`.
    pub(crate) fn stub_larry(&self, target: &Path) -> Check {
        let debug = target.join("debug");
        fs::create_dir_all(&debug)?;
        self.stub(&debug.join("larry"), "target/debug/larry", "exit 0")
    }

    /// Writes the stub at `path`: record the call, follow the plan, else run `otherwise`.
    fn stub(&self, path: &Path, name: &str, otherwise: &str) -> Check {
        let script = format!(
            r#"#!/bin/sh
exec 3<&0
line="{name} $*"
printf '%s\n' "$line" >> '{ran}'
while IFS='|' read -r want how; do
  if [ "$want" = "$line" ]; then
    case "$how" in
      killed) kill -9 $$ ;;
      wait) echo {HELD}; read -r _go <&3; exit 0 ;;
      waitfail) echo {HELD}; read -r _go <&3; exit 1 ;;
      say:*) printf '%s\n' "${{how#say:}}"; exit 0 ;;
      saydie:*) printf '%s\n' "${{how#saydie:}}"; exit 1 ;;
      *) exit "$how" ;;
    esac
  fi
done < '{plan}'
{otherwise}
"#,
            ran = self.base.join("ran").display(),
            plan = self.base.join("plan").display(),
        );
        fs::write(path, script)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        Ok(())
    }

    /// Replaces the plan: what the stub does for each named command line. An unnamed one passes.
    pub(crate) fn plan(&self, entries: &[(&str, &str)]) -> Check {
        let lines: Vec<String> = entries
            .iter()
            .map(|&(line, how)| format!("{line}|{how}\n"))
            .collect();
        fs::write(self.base.join("plan"), lines.concat())?;
        Ok(())
    }

    /// The command lines the stubs saw since the last call, in order.
    pub(crate) fn take_ran(&self) -> Check<Vec<String>> {
        let record = self.base.join("ran");
        let text = match fs::read_to_string(&record) {
            Ok(text) => text,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(cause) => return Err(cause.into()),
        };
        fs::remove_file(&record)?;
        Ok(text.lines().map(ToOwned::to_owned).collect())
    }

    /// The shipped binary with `args`, started in `cwd`, seeing only the rig's `PATH`.
    pub(crate) fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let runner = std::env::vars_os().filter(|pair| {
            let name = pair.0.to_string_lossy();
            name.starts_with("NEXTEST_") || name.starts_with("LGWKS_MUTANT")
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_larry-local-ci"));
        command
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(runner)
            .env("PATH", self.base.join("bin"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::null());
        coverage::forward(&mut command);
        command
    }

    /// One whole run from the checkout root with no argument.
    pub(crate) fn run(&self) -> Check<Output> {
        Ok(self.command(&self.root, &[]).output()?)
    }

    /// Starts a run from the checkout root with the log on a pipe and reads it until the stub of
    /// `lane`, which the plan has told to block, says it is waiting on its input. The wait ends on
    /// that line or on the run's end, never on what the binary logs.
    pub(crate) fn hold_at(&self, lane: &str) -> Check<Held> {
        let (reader, writer) = io::pipe()?;
        let child = self
            .command(&self.root, &[])
            .stdin(Stdio::piped())
            .stdout(writer)
            .stderr(Stdio::piped())
            .spawn()?;
        let mut log = BufReader::new(reader);
        let mut line = String::new();
        while line.strip_suffix('\n') != Some(HELD) {
            line.clear();
            if log.read_line(&mut line)? == 0 {
                return Err(format!("the run ended before {lane} blocked").into());
            }
        }
        Ok(Held { child, log })
    }

    /// The head of the checkout.
    pub(crate) fn head(&self) -> Check<String> {
        self.git(&["rev-parse", "HEAD"])
    }

    /// One real git call in the checkout, its output trimmed; a non-zero exit is the error.
    pub(crate) fn git(&self, args: &[&str]) -> Check<String> {
        let output = git_command(self.repo.path()).args(args).output()?;
        if !output.status.success() {
            return Err(format!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    /// The default receipt directory of this checkout.
    pub(crate) fn default_receipts(&self) -> PathBuf {
        self.root.join("target").join("local-ci")
    }

    /// Leaves the receipt of an earlier head in the default directory.
    pub(crate) fn write_earlier_receipt(&self) -> Check {
        let directory = self.default_receipts();
        fs::create_dir_all(&directory)?;
        fs::write(directory.join(RECEIPT), EARLIER)?;
        Ok(())
    }

    /// The earlier receipt is byte for byte as it was, and no temporary lies beside it.
    pub(crate) fn earlier_receipt_stands(&self, context: &str) -> Check {
        let directory = self.default_receipts();
        assert_eq!(
            fs::read_to_string(directory.join(RECEIPT))?,
            EARLIER,
            "{context}: the earlier receipt stands"
        );
        assert_eq!(entries(&directory)?, [RECEIPT], "{context}: no temporary");
        Ok(())
    }
}

/// A run blocked inside one lane.
pub(crate) struct Held {
    /// The running binary.
    pub(crate) child: Child,
    /// The rest of its log.
    pub(crate) log: BufReader<PipeReader>,
}

impl Held {
    /// Releases the blocked lane with the log still read, and waits for the run to end.
    pub(crate) fn release(self) -> Check<Output> {
        let Held { mut child, mut log } = self;
        let mut input = child.stdin.take().ok_or("the held run has no input")?;
        input.write_all(b"go\n")?;
        drop(input);
        let mut rest = Vec::new();
        log.read_to_end(&mut rest)?;
        Ok(child.wait_with_output()?)
    }

    /// Closes the log's only reader, releases the blocked lane and waits for the run to end.
    pub(crate) fn release_with_the_log_closed(self) -> Check<Output> {
        let Held { mut child, log } = self;
        drop(log);
        let mut input = child.stdin.take().ok_or("the held run has no input")?;
        input.write_all(b"go\n")?;
        drop(input);
        Ok(child.wait_with_output()?)
    }
}

/// The git the host's `PATH` finds first; the `git` stub starts it, so the rig's own `PATH` needs
/// no host entry.
fn real_git() -> Check<PathBuf> {
    let path = std::env::var_os("PATH").ok_or("PATH is unset")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|git| git.is_file())
        .ok_or_else(|| "no git on PATH".into())
}

/// Exit status, standard output and standard error, each exact.
pub(crate) fn exactly(output: &Output, code: i32, stdout: &str, stderr: &str) {
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(err, stderr, "standard error, with log: {out}");
    assert_eq!(out, stdout, "standard output");
    assert_eq!(output.status.code(), Some(code), "exit status");
}

/// The log of a run that got through every lane: each command line, then `red`'s line for it.
pub(crate) fn lane_log(red: &dyn Fn(&str, &str) -> Option<String>) -> String {
    let lines: Vec<String> = LANES
        .iter()
        .map(|&(name, line)| match red(name, line) {
            Some(ended) => format!("\nlocal-ci: {line}\n\nlocal-ci: RED lane {name} {ended}\n"),
            None => format!("\nlocal-ci: {line}\n"),
        })
        .collect();
    lines.concat()
}

/// The last log line of a green run.
pub(crate) fn ok_line(directory: &Path, short: &str) -> String {
    format!(
        "\nlocal-ci: OK, every declared lane passed; receipt {} for {short}\n",
        directory.join(RECEIPT).display()
    )
}

/// What a run asks of the stubs up to its last lane: the root, the head it starts on, every lane.
pub(crate) fn lanes_run() -> Vec<String> {
    [ASK_ROOT, ASK_HEAD]
        .into_iter()
        .chain(LANES.iter().map(|&(_, line)| line))
        .map(ToOwned::to_owned)
        .collect()
}

/// What a green run asks of the stubs: [`lanes_run`], then the branch and the head again.
pub(crate) fn whole_run() -> Vec<String> {
    let mut ran = lanes_run();
    ran.push(ASK_BRANCH.to_owned());
    ran.push(ASK_HEAD.to_owned());
    ran
}

/// The receipt in `directory` holds exactly its five members, naming `root`, `branch` and `head`,
/// and is the only file there, so no temporary of the replace is left.
pub(crate) fn attests(directory: &Path, root: &Path, branch: &str, head: &str) -> Check {
    let text = fs::read_to_string(directory.join(RECEIPT))?;
    assert!(
        text.ends_with("}\n") && text.lines().count() == 1,
        "one whole line: {text:?}"
    );
    let receipt: Value = json::from_str(&text)?;
    let attested = receipt
        .get("attested_at")
        .and_then(Value::as_u64)
        .ok_or("attested_at is not a whole number")?;
    assert!(
        attested > 1_700_000_000,
        "attested_at is seconds since 1970: {attested}"
    );
    assert_eq!(
        receipt,
        json!({
            "repo_root": root.display().to_string(),
            "branch": branch,
            "head_sha": head,
            "attested_at": attested,
            "ci_script": "cargo local-ci",
        }),
        "the receipt's members"
    );
    assert_eq!(
        entries(directory)?,
        [OsStr::new(RECEIPT)],
        "nothing but the receipt is in its directory"
    );
    Ok(())
}

/// The names in `directory`, sorted.
pub(crate) fn entries(directory: &Path) -> Check<Vec<OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory)? {
        names.push(entry?.file_name());
    }
    names.sort();
    Ok(names)
}
