//! Two tenants on one state directory, key rotation by key id and a moved state
//! directory (larry#211): each tenant seals under its own key, verifies only its
//! own receipts, a rotated key verifies old receipts by key id and seals new
//! ones under the new id, and moving the directory keeps every tenant id.

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/larry_guard.rs"]
mod larry_guard;
#[path = "support/origin.rs"]
mod origin;
#[path = "support/sealed_report.rs"]
mod sealed_report;

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Output;

use larry_cli::{spawn_larry_args, stderr_of};
use larry_guard::Larry;
use lgwks_std::json::Value;
use origin::{TempDir, fixture};
use sealed_report::{denial_lines, flag_of, report_of};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// A task document naming `repo`, `base` and the acceptance commands as raw JSON.
fn task_json(repo: &Path, base: &str, acceptance: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"{}","acceptance":{}}}"#,
        repo.display(),
        base,
        acceptance
    )
}

/// Runs `larry exec` with `env` set, pipes `task` in and closes stdin.
fn spawn_exec(env: &[(&str, &OsStr)], task: &str) -> Result<Larry, Box<dyn Error>> {
    spawn_larry_args(&["exec"], env, task.as_bytes())
}

/// Runs `larry exec` to its end with `task` on stdin and returns its output.
fn run_exec(state: &Path, task: &str, timeout_ms: Option<u64>) -> Result<Output, Box<dyn Error>> {
    let timeout = timeout_ms.map(|ms| ms.to_string());
    let mut env: Vec<(&str, &OsStr)> = Vec::new();
    env.push(("LARRY_STATE_DIR", state.as_os_str()));
    if let Some(ref ms) = timeout {
        env.push(("LARRY_EXEC_TIMEOUT_MS", OsStr::new(ms)));
    }
    Ok(spawn_exec(&env, task)?.wait_with_output()?)
}

/// The member `key` of `value` as text.
fn text_of<'a>(value: &'a Value, key: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the report names {key}").into())
}

/// A refusal exits 1 with exactly one stderr line naming `needle`.
fn assert_refused(output: &Output, needle: &str) {
    let stderr = stderr_of(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a refusal must exit 1; stderr: {stderr:?}"
    );
    assert_eq!(
        stderr.lines().count(),
        1,
        "a refusal must print exactly one stderr line, got {stderr:?}"
    );
    assert!(
        stderr.contains(needle),
        "stderr {stderr:?} must name {needle}"
    );
}

/// Runs `larry` with `args` in `state` as tenant `tenant`.
fn run_as(
    args: &[&str],
    state: &Path,
    tenant: &str,
    input: &[u8],
) -> Result<Output, Box<dyn Error>> {
    run_where(
        args,
        state.as_os_str(),
        &std::env::current_dir()?,
        tenant,
        input,
    )
}

/// Runs `larry` with `args` in `cwd`, naming the state directory `state_arg`.
/// A relative `state_arg` resolves in the child's directory, so one directory
/// is driven through its relative, absolute and symlink spellings (larry#211).
fn run_where(
    args: &[&str],
    state_arg: &OsStr,
    cwd: &Path,
    tenant: &str,
    input: &[u8],
) -> Result<Output, Box<dyn Error>> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let tenant = OsStr::from_bytes(tenant.as_bytes()).to_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state_arg)
        .env("LARRY_TENANT", tenant.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(cwd);
    coverage::forward(&mut command);
    let mut child = Larry::spawn(&mut command)?;
    child
        .stdin
        .take()
        .ok_or("the child keeps its stdin")?
        .write_all(input)?;
    Ok(child.wait_with_output()?)
}

/// Runs `larry exec` in `state` as tenant `tenant`.
fn exec_as(state: &Path, tenant: &str, task: &str) -> Result<Output, Box<dyn Error>> {
    run_as(&["exec"], state, tenant, task.as_bytes())
}

/// Runs `larry verify` in `state` as tenant `tenant`.
fn verify_as(state: &Path, tenant: &str, report: &[u8]) -> Result<Output, Box<dyn Error>> {
    run_as(&["verify"], state, tenant, report)
}

/// The receipt's key id member as a number.
fn key_id_of(report: &Value) -> Result<u64, Box<dyn Error>> {
    report
        .get("receipt")
        .and_then(|receipt| receipt.get("kid"))
        .and_then(Value::as_u64)
        .ok_or_else(|| "the receipt names its key id".into())
}

/// Provisions `tenant`'s policy and repository root in `state`: named tenants
/// start with no rows, so a journey names what its tenants may run.
fn provision_as(state: &Path, tenant: &str, root: &Path) -> TestResult {
    larry_demo::executor::provision_tenant(
        |name| match name {
            "LARRY_STATE_DIR" => Some(state.to_string_lossy().into_owned()),
            "LARRY_TENANT" => Some(tenant.to_owned()),
            _ => None,
        },
        "file.read,file.write,process.spawn,model.invoke,model.command",
        &[root],
    )?;
    Ok(())
}

/// Two tenants on one state directory each verify only their own receipts.
#[test]
fn sim_two_tenants_on_one_state_directory_verify_only_their_own() -> TestResult {
    let (state, origin) = fixture()?;
    provision_as(state.path(), "alpha", origin.path())?;
    provision_as(state.path(), "beta", origin.path())?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let alpha = exec_as(state.path(), "alpha", &task)?;
    assert_eq!(
        alpha.status.code(),
        Some(0),
        "alpha runs: {}",
        stderr_of(&alpha)
    );
    let beta = exec_as(state.path(), "beta", &task)?;
    assert_eq!(
        beta.status.code(),
        Some(0),
        "beta runs: {}",
        stderr_of(&beta)
    );
    let alpha_report = report_of(&alpha)?;
    let beta_report = report_of(&beta)?;
    assert_ne!(
        text_of(&alpha_report, "task_id")?,
        text_of(&beta_report, "task_id")?,
        "two tenants never share a run id"
    );
    let verified = verify_as(state.path(), "alpha", &alpha.stdout)?;
    assert_eq!(verified.status.code(), Some(0), "alpha verifies its own");
    let verified = verify_as(state.path(), "beta", &beta.stdout)?;
    assert_eq!(verified.status.code(), Some(0), "beta verifies its own");
    let refused = verify_as(state.path(), "beta", &alpha.stdout)?;
    assert_refused(&refused, "untrusted");
    let refused = verify_as(state.path(), "alpha", &beta.stdout)?;
    assert_refused(&refused, "untrusted");
    Ok(())
}

/// A rotated key verifies old receipts by key id and seals new ones under the new id.
#[test]
fn a_rotated_key_verifies_old_receipts_by_key_id_and_seals_new_ones_under_the_new_id() -> TestResult
{
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let first = run_exec(state.path(), &task, None)?;
    assert_eq!(first.status.code(), Some(0), "the first run seals");
    let before = report_of(&first)?;
    let before_id = key_id_of(&before)?;
    let rotated = run_as(&["key", "rotate"], state.path(), "", b"")?;
    assert_eq!(
        rotated.status.code(),
        Some(0),
        "rotate seals nothing: {}",
        stderr_of(&rotated)
    );
    let second = run_exec(state.path(), &task, None)?;
    assert_eq!(second.status.code(), Some(0), "the second run seals");
    let after = report_of(&second)?;
    let after_id = key_id_of(&after)?;
    assert_ne!(before_id, after_id, "the new seal names the new key id");
    let old = verify_as(state.path(), "", &first.stdout)?;
    assert_eq!(old.status.code(), Some(0), "the old receipt still verifies");
    let new = verify_as(state.path(), "", &second.stdout)?;
    assert_eq!(new.status.code(), Some(0), "the new receipt verifies");
    Ok(())
}

/// A denied tenant seals its refusal and another tenant still runs: policy is
/// per tenant even when the ledger is shared.
#[test]
fn a_denied_tenant_seals_its_refusal_and_another_still_runs() -> TestResult {
    let (state, origin) = fixture()?;
    provision_as(state.path(), "alpha", origin.path())?;
    provision_as(state.path(), "beta", origin.path())?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let mut env: Vec<(&str, &OsStr)> = vec![("LARRY_STATE_DIR", state.path().as_os_str())];
    env.push(("LARRY_TENANT", OsStr::new("alpha")));
    env.push(("LARRY_POLICY", OsStr::new("")));
    let denied = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    assert_eq!(denied.status.code(), Some(2), "a denied run seals rejected");
    let report = report_of(&denied)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "the denied run is not accepted"
    );
    assert!(
        !denial_lines(&report)?.is_empty(),
        "the sealed report names its refusal"
    );
    let beta = exec_as(state.path(), "beta", &task)?;
    assert_eq!(beta.status.code(), Some(0), "the other tenant runs");
    assert!(flag_of(&report_of(&beta)?, "accepted")?, "and is accepted");
    Ok(())
}
/// Moving the state directory keeps every tenant id: the same keyed task maps to
/// the same run id after the move.
#[test]
fn moving_the_state_directory_keeps_every_tenant_id() -> TestResult {
    let (state, origin) = fixture()?;
    let keyed = format!(
        r#"{{"repo":"{}","base":"HEAD","acceptance":[["true"]],"idempotency_key":"move-keeps-tenant"}}"#,
        origin.path().display()
    );
    let first = run_exec(state.path(), &keyed, None)?;
    assert_eq!(first.status.code(), Some(0), "the keyed run seals");
    let before = text_of(&report_of(&first)?, "task_id")?.to_owned();
    let moved = state.path().with_extension("moved-state");
    fs::rename(state.path(), &moved)?;
    let second = run_as(&["exec"], &moved, "", keyed.as_bytes())?;
    assert_eq!(
        second.status.code(),
        Some(0),
        "the retry after the move seals"
    );
    let retried = report_of(&second)?;
    assert_eq!(
        text_of(&retried, "task_id")?,
        before,
        "the moved directory mints the same run id, so the retry answers from the kept report"
    );
    fs::remove_dir_all(&moved)?;
    Ok(())
}

/// `larry key` and `larry verify` on an empty state directory refuse without
/// creating the store or minting a secret.
#[test]
fn an_empty_state_directory_is_never_given_a_store_or_a_secret() -> TestResult {
    let empty = TempDir::new()?;
    let missing = empty.path().join("no-such-state");
    let key = run_as(&["key"], &missing, "", b"")?;
    assert_refused(&key, "state");
    assert!(
        !missing.exists(),
        "larry key creates no directory on an empty path"
    );
    let report = br#"{"task_id":"x"}"#;
    let verified = run_as(&["verify"], &missing, "", report)?;
    assert_refused(&verified, "state");
    assert!(
        !missing.exists(),
        "larry verify creates no directory on an empty path"
    );
    Ok(())
}

/// One directory through relative, absolute and symlink paths is one tenant:
/// the same keyed task maps to the same run id, the same key verifies it, and
/// a root provisioned through one spelling authorizes the other two (larry#211).
#[test]
fn one_dir_through_relative_absolute_and_symlink_paths_is_one_tenant() -> TestResult {
    let (_fstate, origin) = fixture()?;
    let parent = TempDir::new()?;
    let state = parent.path().join("state");
    std::fs::create_dir_all(&state)?;
    let link = parent.path().join("link");
    std::os::unix::fs::symlink(&state, &link)?;
    // The root is provisioned once, through the absolute spelling.
    larry_demo::executor::provision_tenant(
        |name| match name {
            "LARRY_STATE_DIR" => Some(state.to_string_lossy().into_owned()),
            _ => None,
        },
        "file.read,file.write,process.spawn,model.invoke,model.command",
        &[origin.path()],
    )?;
    let keyed = format!(
        r#"{{"repo":"{}","base":"HEAD","acceptance":[["true"]],"idempotency_key":"one-dir-one-tenant"}}"#,
        origin.path().display()
    );
    let relative = run_where(
        &["exec"],
        OsStr::new("state"),
        parent.path(),
        "",
        keyed.as_bytes(),
    )?;
    assert_eq!(
        relative.status.code(),
        Some(0),
        "the relative path runs: {}",
        stderr_of(&relative)
    );
    let first = text_of(&report_of(&relative)?, "task_id")?.to_owned();
    let absolute = run_as(&["exec"], &state, "", keyed.as_bytes())?;
    assert_eq!(
        absolute.status.code(),
        Some(0),
        "the absolute path runs: {}",
        stderr_of(&absolute)
    );
    let through_link = run_as(&["exec"], &link, "", keyed.as_bytes())?;
    assert_eq!(
        through_link.status.code(),
        Some(0),
        "the symlink runs: {}",
        stderr_of(&through_link)
    );
    for (name, output) in [("absolute", &absolute), ("symlink", &through_link)] {
        assert_eq!(
            text_of(&report_of(output)?, "task_id")?,
            first,
            "{name}: the same keyed task is the same run, so the retry answers from the kept report"
        );
    }
    let mut keys = Vec::with_capacity(3);
    for (name, spelling, cwd) in [
        ("relative", OsStr::new("state"), parent.path()),
        ("absolute", state.as_os_str(), parent.path()),
        ("symlink", link.as_os_str(), parent.path()),
    ] {
        let printed = run_where(&["key"], spelling, cwd, "", b"")?;
        assert_eq!(
            printed.status.code(),
            Some(0),
            "{name}: the key reads: {}",
            stderr_of(&printed)
        );
        keys.push(
            String::from_utf8_lossy(&printed.stdout)
                .trim_end()
                .to_owned(),
        );
    }
    let first_key = keys.first().ok_or("three spellings print three keys")?;
    assert_eq!(first_key.len(), 64, "a key is 64 hex characters");
    assert!(
        keys.iter().all(|key| key == first_key),
        "one directory holds one key whatever the spelling"
    );
    Ok(())
}
