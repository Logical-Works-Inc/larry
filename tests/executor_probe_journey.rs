//! Journeys: one environment probe at task start (larry#274), as an operator
//! lives it: a briefed `larry exec` task seals the probe as its first fact,
//! the model's prompt carries the same facts after its byte-stable prefix
//! (ADR 0043), a run that fixes the probe replays its gateway trace (SD-4),
//! and a malformed probe value refuses the task typed.
//!
//! Source: Meta-Harness `agent.py:873-969` (one probe in the first prompt saves
//! turns; a failed probe is never swallowed, unlike `:990`), SD-4 (same seed and
//! same inputs, same trace) and the gateway fault taxonomy the MCP journey
//! drives (a refusal seals nothing). The probe here is the injected fixed value
//! (`LARRY_PROBE`); the live folder probe's machinery is proven by the
//! `executor::probe` simulations that script its seams, and by the replay
//! journeys that hold ADR 0043's prefix invariant with the probe present.
//! Honest limit: the matched-set turn-count comparison rides the #94 runner,
//! which is not built: this proves the single-task half, that the facts arrive
//! in turn one.

#[path = "support/fixed_probe.rs"]
mod fixed_probe;
#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/json_at.rs"]
mod json_at;
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;
#[path = "support/sealed_report.rs"]
mod sealed_report;

use json_at::At;
use larry_cli::{spawn_larry_args, stderr_of};
use lgwks_std::json::{self, Value};
use mock_model::{Config, Fault, Handle};
use origin::fixture;
use sealed_report::{denial_lines, flag_of, report_of};
use std::ffi::OsString;

use fixed_probe::FIXED_PROBE;

/// What every test returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The task document: the origin at HEAD, one file for the model, one command.
fn brief_task(repo: &std::path::Path) -> String {
    format!(
        r#"{{"repo":"{}","base":"HEAD","goal":"write the probe note","files":["note.txt"],"acceptance":[["true"]]}}"#,
        repo.display()
    )
}

/// The task document with no brief: one acceptance command, no model step, so
/// a probe refusal is the first thing a run can meet.
fn plain_task(repo: &std::path::Path) -> String {
    format!(
        r#"{{"repo":"{}","base":"HEAD","acceptance":[["true"]]}}"#,
        repo.display()
    )
}

/// Runs the briefed task of `origin` against the mock at `handle` to its end,
/// with `extra` settings beside the model's own.
fn run_briefed(
    state: &std::path::Path,
    origin: &std::path::Path,
    handle: &Handle,
    extra: &[(&'static str, &'static str)],
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let path = std::env::var("PATH")?;
    let url = format!("http://{}/v1", handle.addr());
    let task = brief_task(origin);
    let policy = std::ffi::OsStr::new("file.read,file.write,process.spawn,model.invoke");
    let state_dir = state.as_os_str().to_owned();
    let path = std::ffi::OsStr::new(path.as_str()).to_owned();
    let url = std::ffi::OsStr::new(url.as_str()).to_owned();
    let owned: Vec<std::ffi::OsString> = extra.iter().map(|pair| OsString::from(pair.1)).collect();
    let mut env: Vec<(&str, &std::ffi::OsStr)> = vec![
        ("LARRY_STATE_DIR", &state_dir),
        ("PATH", &path),
        ("LARRY_POLICY", policy),
        ("LARRY_MODEL_URL", &url),
        ("LARRY_MODEL", std::ffi::OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", std::ffi::OsStr::new("mock-key")),
        ("LARRY_MODEL_BACKOFF_MS", std::ffi::OsStr::new("0")),
    ];
    for (pair, value) in extra.iter().zip(owned.iter()) {
        env.push((pair.0, value.as_os_str()));
    }
    Ok(spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?)
}

/// The briefed task of `origin` in `state` against `handle`, with the fixed
/// probe riding its prompt: the one shape every probe journey drives
/// (larry#274).
fn briefed_probe_run(
    handle: &Handle,
    state: &std::path::Path,
    origin: &std::path::Path,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    run_briefed(state, origin, handle, &[("LARRY_PROBE", FIXED_PROBE)])
}

/// A briefed task with the fixed probe seals that probe as its first fact and
/// shows the same facts to the model after its byte-stable prefix (larry#274).
#[test]
fn an_env_probe_is_sealed_as_receipt_zero_and_reaches_the_model_first() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(274), |handle| {
        let output = briefed_probe_run(handle, state.path(), origin.path())?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "a passing briefed task exits 0: {}",
            stderr_of(&output)
        );
        assert_eq!(handle.served(), 1, "one file takes one completion");
        assert!(
            handle
                .authorizations()
                .iter()
                .all(|header| header.as_deref() == Some("Bearer mock-key")),
            "the completion carries the task's key"
        );
        assert!(!handle.trace_hash().is_empty(), "the run leaves its trace");
        let report = report_of(&output)?;
        assert!(flag_of(&report, "accepted")?, "the run is accepted");
        check_probe(&report)?;
        check_prompt(handle, origin.path())?;
        let verified = spawn_larry_args(
            &["verify"],
            &[("LARRY_STATE_DIR", state.path().as_os_str())],
            &output.stdout,
        )?
        .wait_with_output()?;
        assert_eq!(
            verified.status.code(),
            Some(0),
            "the receipt with its probe verifies as accepted: {}",
            stderr_of(&verified)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}

/// Checks the sealed probe: it is exactly the value the run was given, every
/// section typed, nothing denied, no environment value in the record.
fn check_probe(report: &Value) -> TestResult {
    let expected: Value = json::from_str(FIXED_PROBE)?;
    let probed = report.at("/probe")?;
    assert_eq!(
        probed, &expected,
        "the sealed probe is the value the run started with"
    );
    assert_eq!(
        denial_lines(report)?,
        Vec::<String>::new(),
        "an accepted run denies nothing"
    );
    let sealed = report.to_string();
    assert!(
        !sealed.contains("mock-key") && !sealed.contains("LARRY_MODEL"),
        "no environment value reaches the sealed record"
    );
    Ok(())
}

/// Checks the mock's first request: the probe rides the user turn after the
/// byte-stable prefix, never inside it (ADR 0043), and names the fixed facts,
/// not this machine's.
fn check_prompt(handle: &Handle, origin: &std::path::Path) -> TestResult {
    let asked = handle
        .requests()
        .into_iter()
        .next()
        .ok_or("a first request")?;
    let body = json::from_slice::<Value>(&asked)?;
    let content = body
        .at("/messages/1/content")?
        .as_str()
        .ok_or("a user turn")?;
    assert!(
        content.contains("Environment probe"),
        "the first prompt carries the probe"
    );
    let write = content
        .find("Write this file now: note.txt")
        .ok_or("the file line")?;
    let probe = content.find("Environment probe").ok_or("the probe line")?;
    let context = content
        .find("write the probe note")
        .ok_or("the brief's context")?;
    assert!(
        context < probe && probe < write,
        "the probe rides after the byte-stable prefix and before the file line"
    );
    assert!(
        content.ends_with("Write this file now: note.txt"),
        "the turn ends with the file line, as every completion always has"
    );
    let origin = origin.display().to_string();
    assert!(
        !content.contains(&origin),
        "the prompt carries the probe the run was given, not this machine"
    );
    Ok(())
}

/// The same seed with the same probe value replays the same gateway trace and
/// the same sealed facts (SD-4): two runs of one task in one state directory,
/// each against its own mock, differ in nothing the prompt carries (larry#274).
#[test]
fn the_same_seed_with_the_same_probe_replays_the_same_trace() -> TestResult {
    let (state, origin) = fixture()?;
    let mut traces = Vec::new();
    let mut probes = Vec::new();
    let mut diffs = Vec::new();
    for _ in 0..2 {
        let trace = mock_model::run(Config::seeded(278), |handle| {
            let output = briefed_probe_run(handle, state.path(), origin.path())?;
            assert_eq!(
                output.status.code(),
                Some(0),
                "each replayed run passes: {}",
                stderr_of(&output)
            );
            let report = report_of(&output)?;
            assert!(
                flag_of(&report, "accepted")?,
                "each replayed run is accepted"
            );
            probes.push(report.at("/probe")?.clone());
            diffs.push(report.at("/diff")?.clone());
            Ok::<_, Box<dyn std::error::Error>>(handle.trace_hash())
        })?;
        traces.push(trace);
    }
    assert_eq!(
        traces.first(),
        traces.get(1),
        "the same seed with the same probe replays the same gateway trace"
    );
    assert_eq!(
        probes.first(),
        probes.get(1),
        "the same seed with the same probe seals the same probe"
    );
    assert_eq!(
        diffs.first(),
        diffs.get(1),
        "the same seed with the same probe seals the same diff"
    );
    Ok(())
}

/// A `LARRY_PROBE` that is not a probe value refuses the task typed before
/// anything runs and seals nothing: not JSON, not an object, and over the
/// bound each name their reason, and none falls back to the live probe
/// (larry#274).
#[test]
fn a_malformed_probe_value_refuses_the_task_typed_and_seals_nothing() -> TestResult {
    let (state, origin) = fixture()?;
    let oversized = format!("\"{}\"", "p".repeat(65_537));
    let refused: [(&str, &str); 3] = [
        ("not json", "not valid JSON"),
        ("[1]", "not a JSON object"),
        (&oversized, "over 65536 bytes"),
    ];
    for (value, needle) in refused {
        let output = spawn_larry_args(
            &["exec"],
            &[
                ("LARRY_STATE_DIR", state.path().as_os_str()),
                (
                    "PATH",
                    std::ffi::OsStr::new(std::env::var("PATH")?.as_str()),
                ),
                ("LARRY_PROBE", std::ffi::OsStr::new(value)),
            ],
            plain_task(origin.path()).as_bytes(),
        )?
        .wait_with_output()?;
        assert_ne!(
            output.status.code(),
            Some(0),
            "a malformed probe value never passes"
        );
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("LARRY_PROBE") && stderr.contains(needle),
            "the refusal names the variable and its reason: {stderr}"
        );
        assert!(
            output.stdout.is_empty(),
            "a refused probe value seals nothing"
        );
    }
    Ok(())
}

/// A fake `rustc` first on PATH that records its argv and environment: the
/// script, its logs and the PATH value a live run needs (larry#274).
fn hermetic_rustc(
    state: &std::path::Path,
) -> Result<(String, std::path::PathBuf, std::path::PathBuf), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt as _;
    let hermetic = state.join("hermetic");
    std::fs::create_dir_all(&hermetic)?;
    let args_log = state.join("rustc-args.log");
    let env_log = state.join("rustc-env.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\necho \"RUSTUP_HOME=$RUSTUP_HOME\" >> '{}'\necho \"CARGO_HOME=$CARGO_HOME\" >> '{}'\necho \"RUSTUP_AUTO_INSTALL=$RUSTUP_AUTO_INSTALL\" >> '{}'\necho \"HOME=$HOME\" >> '{}'\necho 'rustc 9.9.9 probe-test'\nexit 0\n",
        args_log.display(),
        env_log.display(),
        env_log.display(),
        env_log.display(),
        env_log.display(),
    );
    let fake = hermetic.join("rustc");
    std::fs::write(&fake, script)?;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))?;
    let path = format!("{}:{}", hermetic.display(), std::env::var("PATH")?);
    Ok((path, args_log, env_log))
}

/// The sealed `rustc` entry of a live probe report.
fn sealed_rustc(report: &Value) -> Result<&Value, Box<dyn std::error::Error>> {
    let toolchains = report
        .get("probe")
        .and_then(|probe| probe.get("toolchains"))
        .and_then(Value::as_array)
        .ok_or("the sealed probe names its toolchains")?;
    toolchains
        .iter()
        .find(|entry| entry.get("tool").and_then(Value::as_str) == Some("rustc"))
        .ok_or("the probe keeps its rustc entry".into())
}

/// Checks the recorded environment: no auto-install, both toolchain homes
/// resolved from the host's real home, and a HOME beside the tree, never the
/// tree (larry#274, sixth pass).
fn check_recorded_home(env: &str, host_home: &str, cwd: &str) -> TestResult {
    assert!(
        env.contains("RUSTUP_AUTO_INSTALL=0"),
        "the child never auto-installs a toolchain: {env:?}"
    );
    for (name, expected) in [
        ("RUSTUP_HOME", format!("{host_home}/.rustup")),
        ("CARGO_HOME", format!("{host_home}/.cargo")),
    ] {
        assert!(
            env.contains(&format!("{name}={expected}")),
            "{name} stays resolved from the host's real home: {env:?}"
        );
    }
    let home = env
        .lines()
        .find_map(|line| line.strip_prefix("HOME="))
        .ok_or("the fake records its HOME")?;
    assert!(
        home.contains("probe-home"),
        "the probe homes beside the tree it reads: {home:?}"
    );
    assert_ne!(
        home, cwd,
        "the tree never gets a home: HOME is {home:?}, the tree {cwd:?}"
    );
    Ok(())
}

/// A live probe never asks a toolchain to install itself: with a fake `rustc`
/// first on PATH that records its arguments and environment, the run passes,
/// the recorded argv is exactly `--version` (never `install` or `toolchain`),
/// and the recorded environment carries the no-install switch, the host's
/// toolchain homes and a HOME beside the tree (larry#274, sixth pass). Catches
/// a probe homed in the tree that synced a whole toolchain there (48 s,
/// 780 MB) and a missing toolchain that downloaded one on every task.
#[test]
fn a_live_probe_never_asks_a_toolchain_to_install() -> TestResult {
    let (state, origin) = fixture()?;
    let host_home = std::env::var("HOME")?;
    let (path, args_log, env_log) = hermetic_rustc(state.path())?;
    let output = spawn_larry_args(
        &["exec"],
        &[
            ("LARRY_STATE_DIR", state.path().as_os_str()),
            ("PATH", std::ffi::OsStr::new(path.as_str())),
            ("HOME", std::ffi::OsStr::new(host_home.as_str())),
        ],
        plain_task(origin.path()).as_bytes(),
    )?
    .wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "a live-probed passing task exits 0: {}",
        stderr_of(&output)
    );
    let report = report_of(&output)?;
    assert!(flag_of(&report, "accepted")?, "the run is accepted");
    let rustc = sealed_rustc(&report)?;
    assert_eq!(
        rustc.get("ok"),
        Some(&json::Value::Bool(true)),
        "the fake rustc reports its version"
    );
    assert!(
        rustc
            .get("version")
            .and_then(Value::as_str)
            .is_some_and(|version| version.contains("9.9.9")),
        "the sealed version is the fake's line"
    );
    let args = std::fs::read_to_string(&args_log)?;
    assert!(
        args.lines().all(|line| line.trim() == "--version"),
        "the probe asks rustc only for its version, never an install: {args:?}"
    );
    assert!(
        !args.contains("install") && !args.contains("toolchain"),
        "no probe call names an install: {args:?}"
    );
    let env = std::fs::read_to_string(&env_log)?;
    let cwd = report
        .get("probe")
        .and_then(|probe| probe.get("cwd"))
        .and_then(Value::as_str)
        .ok_or("the probe seals its folder")?;
    check_recorded_home(&env, &host_home, cwd)
}

/// A hostile, cut, filtered, empty or silent gateway refuses the briefed task
/// typed and seals nothing, while a padded 9router reply is accepted with the
/// probe (the exec door's half of the gateway taxonomy).
#[test]
fn scripted_gateway_faults_refuse_the_exec_task_typed_and_seal_nothing() -> TestResult {
    let (state, origin) = fixture()?;
    let refused: [(Fault, &str); 4] = [
        (
            Fault::HostileRefusal { status: 401 },
            "refused with status 401",
        ),
        (Fault::Finish { reason: "length" }, "finish_reason length"),
        (
            Fault::Finish {
                reason: "content_filter",
            },
            "finish_reason content_filter",
        ),
        (Fault::EmptyContent, "the model's reply is empty"),
    ];
    for (fault, needle) in refused {
        let config = Config {
            script: vec![Some(fault)],
            ..Config::seeded(275)
        };
        mock_model::run(config, |handle| {
            let output = briefed_probe_run(handle, state.path(), origin.path())?;
            assert_ne!(
                output.status.code(),
                Some(0),
                "a gateway fault never passes: {}",
                stderr_of(&output)
            );
            assert!(
                stderr_of(&output).contains(needle),
                "the refusal names its cause: {}",
                stderr_of(&output)
            );
            assert!(
                output.stdout.is_empty(),
                "a refused model step seals nothing"
            );
            Ok::<_, Box<dyn std::error::Error>>(())
        })?;
    }
    let padded = Config {
        script: vec![Some(Fault::PaddedDone)],
        ..Config::seeded(276)
    };
    mock_model::run(padded, |handle| {
        let output = briefed_probe_run(handle, state.path(), origin.path())?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "a padded 9router reply parses: {}",
            stderr_of(&output)
        );
        check_probe(&report_of(&output)?)?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    let silent = Config {
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(277)
    };
    mock_model::run(silent, |handle| {
        let output = run_briefed(
            state.path(),
            origin.path(),
            handle,
            &[
                ("LARRY_PROBE", FIXED_PROBE),
                ("LARRY_MODEL_TIMEOUT_MS", "1000"),
                ("LARRY_MODEL_ATTEMPTS", "1"),
            ],
        )?;
        assert_ne!(
            output.status.code(),
            Some(0),
            "a silent gateway never passes"
        );
        assert!(
            stderr_of(&output).contains("timed out"),
            "the refusal names the timeout: {}",
            stderr_of(&output)
        );
        assert!(output.stdout.is_empty(), "a silent gateway seals nothing");
        handle.release_delayed();
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}
