//! The shipped executor's repository-policy journey.

#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::process::{Command, Output};

use larry_cli::{spawn_larry_args, stderr_of};
use temp_dir::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn git(dir: &std::path::Path, args: &[&str]) -> TestResult {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {:?} failed", args).into())
    }
}

fn unprovisioned() -> TestResult<(TempDir, TempDir)> {
    let state = TempDir::new()?;
    let origin = TempDir::new()?;
    std::fs::write(origin.path().join("README"), "origin\n")?;
    git(origin.path(), &["init", "--quiet"])?;
    git(origin.path(), &["config", "user.name", "Larry Test"])?;
    git(
        origin.path(),
        &["config", "user.email", "larry-test@example.invalid"],
    )?;
    git(origin.path(), &["add", "README"])?;
    git(origin.path(), &["commit", "--quiet", "-m", "init"])?;
    Ok((state, origin))
}

fn fixture() -> TestResult<(TempDir, TempDir)> {
    let (state, origin) = unprovisioned()?;
    larry_demo::executor::provision_tenant(
        |_| Some(state.path().to_string_lossy().into_owned()),
        "file.read,file.write,process.spawn,model.invoke,model.command",
        &[origin.path()],
    )?;
    Ok((state, origin))
}

fn task_json(repo: &std::path::Path) -> String {
    format!(
        r#"{{"repo":{:?},"base":"HEAD","acceptance":[["true"]]}}"#,
        repo.to_string_lossy()
    )
}

fn run_exec(state: &std::path::Path, task: &str) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(spawn_larry_args(
        &["exec"],
        &[("LARRY_STATE_DIR", state.as_os_str())],
        task.as_bytes(),
    )?
    .wait_with_output()?)
}

fn assert_refused(output: &Output, text: &str) {
    assert_eq!(output.status.code(), Some(1), "{text}: {output:?}");
    assert!(
        stderr_of(output).contains(text),
        "{text}: {}",
        stderr_of(output)
    );
    assert!(
        output.stdout.is_empty(),
        "{text}: a refusal seals no report"
    );
}

#[test]
fn exec_rejects_relative_and_disallowed_repositories_before_clone() -> TestResult {
    let (state, _origin) = fixture()?;
    let relative = r#"{"repo":".","base":"HEAD","acceptance":[["true"]]}"#;
    let output = run_exec(state.path(), relative)?;
    assert_refused(&output, "repository path is invalid");
    assert!(!state.path().join("ws").try_exists()?);

    let outside = TempDir::new()?;
    let output = run_exec(state.path(), &task_json(outside.path()))?;
    assert_refused(&output, "not in the tenant allowlist");

    let inside_state = state.path().join("ws");
    std::fs::create_dir_all(&inside_state)?;
    let output = run_exec(state.path(), &task_json(&inside_state))?;
    assert_refused(&output, "inside the state directory");
    assert!(inside_state.read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn serve_provision_persists_the_remote_root_before_an_exec() -> TestResult {
    let (state, origin) = fixture()?;
    let root = origin.path().to_string_lossy().into_owned();
    let policy = std::ffi::OsStr::new("file.read,file.write,process.spawn,model.invoke");
    let output = spawn_larry_args(
        &["serve", "provision", &root],
        &[
            ("LARRY_STATE_DIR", state.path().as_os_str()),
            ("LARRY_POLICY", policy),
        ],
        b"",
    )?
    .wait_with_output()?;
    assert!(
        output.status.success(),
        "serve provision failed: {}",
        stderr_of(&output)
    );
    let task = task_json(origin.path());
    let run = run_exec(state.path(), &task)?;
    assert_eq!(run.status.code(), Some(0), "the provisioned task succeeds");
    Ok(())
}

/// A local `larry exec` in a state with no provisioned root runs any absolute
/// repository outside the state directory; once `larry serve provision` stores
/// its first root, the same task outside that root is refused and told how to
/// allow it (larry#207).
#[test]
fn a_local_exec_in_an_unprovisioned_state_runs_and_a_provisioned_root_binds_it() -> TestResult {
    let (state, origin) = unprovisioned()?;
    let task = task_json(origin.path());
    let first = run_exec(state.path(), &task)?;
    assert_eq!(
        first.status.code(),
        Some(0),
        "an unprovisioned local exec runs: {}",
        stderr_of(&first)
    );
    let other = TempDir::new()?;
    let policy = std::ffi::OsStr::new("file.read,file.write,process.spawn,model.invoke");
    let provided = other.path().to_string_lossy().into_owned();
    let output = spawn_larry_args(
        &["serve", "provision", &provided],
        &[
            ("LARRY_STATE_DIR", state.path().as_os_str()),
            ("LARRY_POLICY", policy),
        ],
        b"",
    )?
    .wait_with_output()?;
    assert!(
        output.status.success(),
        "serve provision failed: {}",
        stderr_of(&output)
    );
    let second = run_exec(state.path(), &task)?;
    assert_refused(&second, "not in the tenant allowlist");
    assert!(
        stderr_of(&second).contains("larry serve provision"),
        "the refusal names the fix: {}",
        stderr_of(&second)
    );
    Ok(())
}
