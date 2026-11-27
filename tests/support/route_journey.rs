//! The binary-driving harness shared by the route journeys (larry#201,
//! larry#205): `larry route` and `larry exec` children with a cleared
//! environment, the tenant's key in the child's environment, and the gateway's
//! address as a sealed report names it. Test support, never linked into a
//! shipped binary (INVARIANTS: test doubles never ship).

use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Output};

use crate::larry_cli::{spawn_larry_args, stderr_of};
use crate::mock_model::Handle;
use crate::origin::TempDir;

/// What every route journey returns.
pub(crate) type TestResult = Result<(), Box<dyn Error>>;

/// The scopes a briefed task needs, so no journey depends on the default policy.
pub(crate) const POLICY: &str = "file.read,file.write,process.spawn,model.invoke";

/// Runs `larry route <args>` with only PATH, the state directory and `env` set.
pub(crate) fn larry_route(
    state: &Path,
    env: &[(&str, &OsStr)],
    args: &[&str],
) -> Result<Output, Box<dyn Error>> {
    Ok(std::process::Command::new(env!("CARGO_BIN_EXE_larry"))
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state)
        .envs(env.iter().copied())
        .output()?)
}

/// The host and port a mock listens on, as a sealed report names it.
pub(crate) fn host_of(mock: &Handle) -> String {
    format!("127.0.0.1:{}", mock.addr().port())
}

/// A briefed task over `origin`: write `note.txt`, then prove it, over the
/// route `route` with the idempotency key `key` when each is given.
pub(crate) fn briefed_task(
    origin: &TempDir,
    route: Option<&str>,
    key: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    use lgwks_std::json::json;
    let mut task = json!({
        "repo": origin.path().display().to_string(),
        "base": "HEAD",
        "goal": "write the note",
        "files": ["note.txt"],
        "acceptance": [["true"]],
    });
    let object = task.as_object_mut().ok_or("a task object")?;
    if let Some(id) = route {
        object.insert(String::from("route"), json!(id));
    }
    if let Some(text) = key {
        object.insert(String::from("idempotency_key"), json!(text));
    }
    Ok(task.to_string())
}

/// What [`add_route`] stores: the gateway, the ids and the optional knobs.
pub(crate) struct NewRoute<'a> {
    /// The gateway base URL, with its path.
    pub url: &'a str,
    /// The route id.
    pub id: &'a str,
    /// The model id sent on every request.
    pub model: &'a str,
    /// The name of the variable holding the key.
    pub key_env: &'a str,
    /// The declared data residency.
    pub residency: &'a str,
    /// The provider kind: `openai-chat` unless set.
    pub kind: Option<&'a str>,
    /// The pinned temperature, when set.
    pub temperature: Option<&'a str>,
}

/// Stores the route `id` of [`NewRoute`], as the default. A route without a
/// kind is added without `--kind`, so the CLI's own default applies.
pub(crate) fn add_route(state: &TempDir, route: &NewRoute<'_>) -> TestResult {
    let mut args = vec![
        "add",
        route.id,
        "--url",
        route.url,
        "--model",
        route.model,
        "--key-env",
        route.key_env,
        "--residency",
        route.residency,
    ];
    if let Some(kind) = route.kind {
        args.extend(["--kind", kind]);
    }
    if let Some(degrees) = route.temperature {
        args.extend(["--temperature", degrees]);
    }
    args.push("--default");
    let output = larry_route(state.path(), &[], &args)?;
    assert!(
        output.status.success(),
        "route add {}: {}",
        route.id,
        stderr_of(&output)
    );
    Ok(())
}

/// Starts `larry exec` on `task` with `key_var` holding `key` in the child's
/// environment, beside `extra`, unwaited.
pub(crate) fn start_with_key(
    state: &Path,
    key_var: &str,
    key: &OsStr,
    task: &str,
    extra: &[(&str, &OsStr)],
) -> Result<Child, Box<dyn Error>> {
    let mut env: Vec<(&str, &OsStr)> = vec![
        ("LARRY_STATE_DIR", state.as_os_str()),
        ("LARRY_POLICY", OsStr::new(POLICY)),
        (key_var, key),
    ];
    env.extend_from_slice(extra);
    spawn_larry_args(&["exec"], &env, task.as_bytes())
}

/// Runs `larry exec` on `task` to completion, with the key and `extra` as
/// [`start_with_key`] sets them.
pub(crate) fn exec_with_key(
    state: &Path,
    key_var: &str,
    key: &OsStr,
    task: &str,
    extra: &[(&str, &OsStr)],
) -> Result<Output, Box<dyn Error>> {
    Ok(start_with_key(state, key_var, key, task, extra)?.wait_with_output()?)
}
