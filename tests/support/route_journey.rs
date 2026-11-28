//! The binary-driving harness shared by the route journeys (larry#201,
//! larry#205, larry#202): `larry route` and `larry exec` children with a cleared
//! environment, the tenant's key in the child's environment, and the gateway's
//! address as a sealed report names it. The one-tenant driver over these
//! doors lives in `route_tenant`. Test support, never linked into a
//! shipped binary (INVARIANTS: test doubles never ship).

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "json_at.rs"]
pub(crate) mod json_at;
#[path = "larry_cli.rs"]
pub(crate) mod larry_cli;
#[path = "mock_model.rs"]
pub(crate) mod mock_model;
#[path = "origin.rs"]
pub(crate) mod origin;
#[path = "route_key.rs"]
pub(crate) mod route_key;
#[path = "sealed_report.rs"]
pub(crate) mod sealed_report;

use crate::coverage::forward;
use crate::larry_guard::Larry;
use larry_cli::{spawn_larry_args, stderr_of};
use lgwks_std::json::json;
use mock_model::Handle;
use origin::TempDir;

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
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry"));
    command
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state)
        .envs(env.iter().copied());
    forward(&mut command);
    Ok(command.output()?)
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
    /// The prompt template, when set.
    pub prompt: Option<&'a str>,
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
    if let Some(template) = route.prompt {
        args.extend(["--prompt", template]);
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

/// The hub directory the shipped ranker resolves its snapshot under, as the
/// hub's own clients do: `HF_HUB_CACHE`, else `HF_HOME/hub`, else the home
/// directory's cache. An exec child runs with a cleared environment, so the
/// harness passes through what the parent resolved; without it an unnamed task
/// cannot even build the ranker its choice seals beside the route (larry#202).
fn hub_dir() -> Option<OsString> {
    if let Some(hub) = std::env::var_os("HF_HUB_CACHE") {
        return Some(hub);
    }
    if let Some(home) = std::env::var_os("HF_HOME") {
        return Some(PathBuf::from(home).join("hub").into_os_string());
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| {
            PathBuf::from(home)
                .join(".cache")
                .join("huggingface")
                .join("hub")
                .into_os_string()
        })
}

/// Starts `larry exec` on `task` with `key_var` holding `key` in the child's
/// environment, beside `extra`, unwaited.
pub(crate) fn start_with_key(
    state: &Path,
    key_var: &str,
    key: &OsStr,
    task: &str,
    extra: &[(&str, &OsStr)],
) -> Result<Larry, Box<dyn Error>> {
    let mut env: Vec<(&str, &OsStr)> = vec![
        ("LARRY_STATE_DIR", state.as_os_str()),
        ("LARRY_POLICY", OsStr::new(POLICY)),
        (key_var, key),
    ];
    env.extend_from_slice(extra);
    // The ranker the choice seals resolves its snapshot here; the cleared
    // child would otherwise fail every unnamed task before any ranking.
    let hub = hub_dir();
    if let Some(dir) = hub.as_ref() {
        env.push(("HF_HUB_CACHE", dir.as_os_str()));
    }
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
