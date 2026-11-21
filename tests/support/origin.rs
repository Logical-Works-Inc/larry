//! The origin repository and state directory every executor journey starts
//! from (larry#85, larry#86, larry#123): a temp directory that removes itself,
//! and a git repository with one commit and no background maintenance.
//! `tests/support/exec_cli.rs` re-exports both for the `larry exec` journeys;
//! `tests/mcp_stdio_journey.rs` includes this file alone. The temp directory
//! itself is the one guard of the integration tests, `support/temp_dir.rs`
//! (larry#267). Every test git call goes through [`git_command`], so no host
//! configuration, hook, branch default or identity reaches a fixture
//! (larry#269).

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "temp_dir.rs"]
mod temp_dir;
pub(crate) use temp_dir::TempDir;

/// A state dir for the executor and a fresh origin repository, in that order.
pub(crate) fn fixture() -> Result<(TempDir, TempDir), Box<dyn Error>> {
    let state = TempDir::new()?;
    let origin = TempDir::new()?;
    origin_repo(origin.path())?;
    Ok((state, origin))
}

/// One git command for `dir`, with the host out of the way (larry#269): the global file is
/// `/dev/null` and the system file is off, so no user or machine config, template or hook is
/// read; `main` is the branch default, signing is off, an empty `core.hooksPath` runs no hooks,
/// and the author and committer are fixed. A repository variable inherited from the process that
/// runs the suite — a pre-commit hook exports `GIT_DIR` — is cleared, so a fixture commits into
/// its own directory and never into the repository that started it (larry#366). Every test git
/// call is built here.
pub(super) fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "init.defaultBranch=main"])
        .args(["-c", "commit.gpgsign=false"])
        .args(["-c", "core.hooksPath="])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t");
    command
}

/// Runs one git command made by `make` in `dir` and fails with git's stderr when it exits
/// non-zero.
fn run_git(
    make: &dyn Fn(&Path) -> Command,
    dir: &Path,
    args: &[&str],
) -> Result<(), Box<dyn Error>> {
    let output = make(dir).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {args:?} in {} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}

/// The standard output of one git command made by `make` in `dir`, trimmed of its line ending;
/// an error carries git's stderr when it exits non-zero.
fn git_text(
    make: &dyn Fn(&Path) -> Command,
    dir: &Path,
    args: &[&str],
) -> Result<String, Box<dyn Error>> {
    let output = make(dir).args(args).output()?;
    if !output.status.success() {
        return Err(format!(
            "git {args:?} in {} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// Makes `dir` a git repository with one commit holding `README` and `src/lib.rs`.
/// Automatic maintenance is off before the first commit: `git commit` otherwise forks
/// `git maintenance run --auto --detach`, whose object-database lock file can appear
/// under `.git/objects` while a journey hashes the origin (larry#138).
pub(super) fn origin_repo(dir: &Path) -> Result<(), Box<dyn Error>> {
    origin_repo_with(&git_command, dir)?;
    commit_all(dir, "init")
}

/// The empty repository and its two settings over `make`, so one test can hand the whole build
/// a hostile environment (larry#269): every git call is made the same way, whatever the host.
fn origin_repo_with(make: &dyn Fn(&Path) -> Command, dir: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join("README"), "origin readme\n")?;
    fs::write(dir.join("src").join("lib.rs"), "pub fn origin() {}\n")?;
    run_git(make, dir, &["init", "-q"])?;
    run_git(make, dir, &["config", "maintenance.auto", "false"])?;
    run_git(make, dir, &["config", "gc.auto", "0"])
}

/// Stages everything in `dir` and commits it as `message`, through [`git_command`].
pub(super) fn commit_all(dir: &Path, message: &str) -> Result<(), Box<dyn Error>> {
    commit_all_with(&git_command, dir, message)
}

/// [`commit_all`] over `make`: the identity is the builder's, so no host setting changes it.
fn commit_all_with(
    make: &dyn Fn(&Path) -> Command,
    dir: &Path,
    message: &str,
) -> Result<(), Box<dyn Error>> {
    run_git(make, dir, &["add", "-A"])?;
    run_git(make, dir, &["commit", "-qm", message])
}

/// The issue's proof (larry#269): a HOME holding a hostile `.gitconfig` and a failing pre-commit
/// hook cannot reach the fixture, because [`git_command`] cuts the global file and overrides the
/// branch default, signing, hooks and identity.
#[test]
fn a_hostile_home_gitconfig_changes_nothing() -> Result<(), Box<dyn Error>> {
    let home = TempDir::new()?;
    let hooks = home.join("hooks");
    fs::create_dir_all(&hooks)?;
    let marker = home.join("hook-ran");
    let hook = hooks.join("pre-commit");
    fs::write(
        &hook,
        format!("#!/bin/sh\n: > '{}'\nexit 1\n", marker.display()),
    )?;
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))?;
    }
    let template = home.join("template");
    fs::create_dir_all(&template)?;
    fs::write(template.join("HOSTILE-TEMPLATE"), "from the host\n")?;
    fs::write(
        home.join(".gitconfig"),
        format!(
            "[user]\n\tname = Hostile\n\temail = hostile@example.com\n[commit]\n\tgpgsign = true\n[init]\n\tdefaultBranch = trunk\n\ttemplateDir = {}\n[core]\n\thooksPath = {}\n",
            template.display(),
            hooks.display()
        ),
    )?;
    let origin = TempDir::new()?;
    let hostile = |dir: &Path| {
        let mut command = git_command(dir);
        command.env("HOME", home.path());
        command
    };
    origin_repo_with(&hostile, origin.path())?;
    commit_all_with(&hostile, origin.path(), "init")?;
    assert_eq!(
        git_text(
            &hostile,
            origin.path(),
            &["symbolic-ref", "--short", "HEAD"]
        )?,
        "main",
        "the host's trunk never becomes the branch"
    );
    assert_eq!(
        git_text(
            &hostile,
            origin.path(),
            &["log", "-1", "--format=%an%n%ae%n%cn%n%ce"]
        )?,
        "t\nt@t\nt\nt@t",
        "the host's identity never authors or commits the change"
    );
    assert_eq!(
        git_text(&hostile, origin.path(), &["log", "-1", "--format=%G?"])?,
        "N",
        "the host's signing setting never signs the commit"
    );
    assert!(
        !origin.path().join(".git").join("HOSTILE-TEMPLATE").exists(),
        "the host's template reached the repository"
    );
    assert!(!marker.exists(), "the host's pre-commit hook ran");
    Ok(())
}

/// The issue's proof (larry#366): a repository variable a pre-commit hook exports is removed
/// from every fixture git call, so a fixture can never commit into the repository that started
/// the suite.
#[test]
fn an_inherited_git_dir_is_cleared() -> Result<(), Box<dyn Error>> {
    let dir = TempDir::new()?;
    let command = git_command(dir.path());
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        let cleared = command
            .get_envs()
            .any(|(key, value)| key.to_str() == Some(name) && value.is_none());
        assert!(
            cleared,
            "{name} must be removed from every fixture git call"
        );
    }
    Ok(())
}
