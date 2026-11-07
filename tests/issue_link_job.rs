//! The `issue-link` job of `.github/workflows/governance.yml` (larry#103, ADR 0012), run as CI runs
//! it: its `run:` step is cut from the workflow and executed by `sh` with a stand-in `gh` on
//! `PATH` that answers what GitHub's `closingIssuesReferences` would. Source: the Director's
//! 2026-11-03 order that every PR carries an issue; GitHub GraphQL
//! `PullRequest.closingIssuesReferences`. Requirement: a PR that closes no issue fails, one that
//! closes any number passes and names them, and the job reruns on every PR event that can change
//! the link.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::seeded::Seeded;

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Seeds the simulation draws issue lists for.
const SEEDS: u64 = 64;

/// Distinct scratch directories made by this test binary.
static SCRATCHES: AtomicU64 = AtomicU64::new(0);

/// The workflow text.
fn workflow() -> Check<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/governance.yml");
    Ok(fs::read_to_string(path)?)
}

/// The `run: |` block of the `issue-link` job, dedented, as the runner hands it to the shell.
fn issue_link_script(text: &str) -> Check<String> {
    let job = text
        .split_once("\n  issue-link:\n")
        .map(|(_, rest)| rest)
        .ok_or("governance.yml has no issue-link job")?;
    let job = match job.split_once("\n  docs-check:\n") {
        Some((head, _)) => head,
        None => job,
    };
    let (_, after) = job
        .split_once("        run: |\n")
        .ok_or("the issue-link job has no run block")?;
    let mut script = String::new();
    for line in after.lines() {
        if !line.is_empty() && !line.starts_with("          ") {
            break;
        }
        script.push_str(line.get(10..).ok_or("short line")?);
        script.push('\n');
    }
    Ok(script)
}

/// A scratch directory holding the script and a stand-in `gh` that prints `answer` and records
/// its arguments, removed when dropped.
struct Runner {
    /// The directory.
    dir: PathBuf,
}

impl Runner {
    /// Writes the job's script and a `gh` answering `answer`.
    fn new(script: &str, answer: &str) -> Check<Self> {
        let serial = SCRATCHES.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("larry-issue-link-{}-{serial}", std::process::id()));
        fs::create_dir(&dir)?;
        let runner = Self { dir };
        fs::write(runner.dir.join("step.sh"), script)?;
        fs::write(runner.dir.join("answer"), answer)?;
        let gh = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\ncat '{}'\n",
            runner.dir.join("args").display(),
            runner.dir.join("answer").display()
        );
        fs::write(runner.dir.join("gh"), gh)?;
        let chmod = Command::new("chmod")
            .arg("755")
            .arg(runner.dir.join("gh"))
            .status()?;
        if !chmod.success() {
            return Err("chmod refused the stand-in gh".into());
        }
        Ok(runner)
    }

    /// Runs the step as the runner does, for PR `number` of `repo`.
    fn run(&self, number: u64, repo: &str) -> Check<Output> {
        let path = format!("{}:/usr/bin:/bin", self.dir.display());
        Ok(Command::new("sh")
            .arg("-e")
            .arg(self.dir.join("step.sh"))
            .env_clear()
            .env("PATH", path)
            .env("GH_TOKEN", "token")
            .env("PR", number.to_string())
            .env("REPO", repo)
            .output()?)
    }

    /// The arguments the stand-in `gh` was called with.
    fn args(&self) -> Check<String> {
        Ok(fs::read_to_string(self.dir.join("args"))?)
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // Best effort: a leftover scratch directory never fails a check.
        drop(fs::remove_dir_all(&self.dir));
    }
}

#[test]
fn a_pr_that_closes_no_issue_fails_with_the_fix_named() -> Check {
    let script = issue_link_script(&workflow()?)?;
    let runner = Runner::new(&script, "")?;
    let output = runner.run(7, "Logical-Works-Inc/larry")?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(
        output.status.code(),
        Some(1),
        "an empty link fails: {stdout}"
    );
    assert!(
        stdout.contains("::error::PR #7 closes no issue: put 'Closes #N' in its body"),
        "the annotation names the PR and the fix: {stdout}"
    );
    let args = runner.args()?;
    assert!(
        args.contains("pr view 7 --repo Logical-Works-Inc/larry --json closingIssuesReferences"),
        "GitHub's own relation is read: {args}"
    );
    Ok(())
}

#[test]
fn the_job_reruns_on_every_event_that_can_change_the_link() -> Check {
    let text = workflow()?;
    assert!(
        text.contains("types: [opened, synchronize, reopened, edited]"),
        "a body edit reruns the check"
    );
    assert!(
        text.contains("if: github.event_name == 'pull_request'"),
        "the job runs on PRs only"
    );
    assert!(
        text.contains("pull-requests: read"),
        "the token can read the relation"
    );
    Ok(())
}

#[test]
fn sim_any_closed_issue_list_passes_and_is_named() -> Check {
    let script = issue_link_script(&workflow()?)?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(5).map_err(|cause| format!("{cause:?}"))?;
        let mut issues = Vec::with_capacity(count);
        for _ in 0..count {
            let number = rng.index(100_000).map_err(|cause| format!("{cause:?}"))?;
            issues.push(format!("#{}", number.saturating_add(1)));
        }
        let answer = issues.join(" ");
        let pr = seed.saturating_add(1);
        let runner = Runner::new(&script, &answer)?;
        let output = runner.run(pr, "owner/repo")?;
        let stdout = String::from_utf8(output.stdout)?;
        if issues.is_empty() {
            assert_eq!(output.status.code(), Some(1), "seed {seed}: no issue fails");
        } else {
            assert!(
                output.status.success(),
                "seed {seed}: {answer} passes: {stdout}"
            );
            assert_eq!(
                stdout,
                format!("PR #{pr} closes {answer}\n"),
                "seed {seed}: the closed issues are named"
            );
        }
    }
    Ok(())
}
