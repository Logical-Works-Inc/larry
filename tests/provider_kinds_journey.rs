//! Journeys of the provider adapters (larry#205, larry#546): the same briefed
//! task through a mock of each provider kind (OpenAI chat, Anthropic Messages,
//! OpenAI Responses). Each receipt names its kind and maps the kind's usage to
//! the one usage shape; a route with no temperature sends no temperature
//! member; a route's prompt template is sealed by its hash, never its text.
//!
//! Source: the Anthropic Messages API (`x-api-key`, `anthropic-version`,
//! `max_tokens`, `system`, `stop_reason`), the OpenAI Responses API (`input`,
//! `output`, `status`), RFC 6750 section 2.1 (the bearer key travels only in
//! the `Authorization` header), POSIX exit status. Requirement proved: one
//! adapter per kind maps path, auth, body and usage; temperature and template
//! are per route; structured kinds answer the file inside one JSON object.
//! Every run drives the shipped binary with a cleared environment against the
//! seeded mock endpoint, so no test makes a paid model call.

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/larry_guard.rs"]
mod larry_guard;

#[path = "support/route_journey.rs"]
mod route_journey;

use std::error::Error;
use std::ffi::OsStr;
use std::process::Output;

use lgwks_std::hash::Hasher;
use lgwks_std::json::{self, Value};
use route_journey::json_at::At;
use route_journey::larry_cli::stderr_of;
use route_journey::mock_model::{Config, Fault, Handle, run};
use route_journey::origin::{TempDir, fixture};
use route_journey::route_key::key_env;
use route_journey::sealed_report::{denial_lines, flag_of, report_of};
use route_journey::{
    NewRoute, TestResult, add_route, briefed_task, exec_with_key, host_of, larry_route,
    start_with_key,
};

/// The key the journey's routes name, in the child's environment.
const KEY: &str = "secret-k";

/// `exec_with_key` over `task` under `state` with the journey's key.
fn exec(state: &TempDir, task: &str) -> Result<Output, Box<dyn Error>> {
    let variable = key_env(state.path(), "K")?;
    exec_with_key(state.path(), &variable, OsStr::new(KEY), task, &[])
}

/// Stores the journey's route `main` pointing at `mock`: the `model-k` model in
/// `CA` over `kind`, with `prompt` and `temperature` when given.
fn add_main_route(
    state: &TempDir,
    mock: &Handle,
    kind: Option<&str>,
    prompt: Option<&str>,
    temperature: Option<&str>,
) -> TestResult {
    let key_env = key_env(state.path(), "K")?;
    let url = format!("http://{}/v1", host_of(mock));
    add_route(
        state,
        &NewRoute {
            url: &url,
            id: "main",
            model: "model-k",
            key_env: &key_env,
            residency: "CA",
            kind,
            prompt,
            temperature,
        },
    )
}

/// The same briefed task through a mock of `kind`: the run is accepted, the
/// receipt names the kind and maps the kind's usage to the one shape, and the
/// request carried the kind's credential scheme.
fn through_kind(kind: &str, seed: u64) -> TestResult {
    let (state, origin) = fixture()?;
    run(Config::seeded(seed), |mock| -> TestResult {
        add_main_route(&state, mock, Some(kind), None, None)?;
        let output = exec(&state, &briefed_task(&origin, None, None)?)?;
        let report = report_of(&output)?;
        assert!(flag_of(&report, "accepted")?, "{}", stderr_of(&output));
        assert!(denial_lines(&report)?.is_empty(), "no denial was sealed");
        let route = report.at("/authored/0/route")?;
        assert_eq!(
            route.at("/kind")?.as_str(),
            Some(kind),
            "the receipt names its kind"
        );
        assert_eq!(
            route.at("/prompt_hash")?,
            &Value::Null,
            "no template seals as null"
        );
        let prompt_tokens = report
            .at("/authored/0/prompt_tokens")?
            .as_u64()
            .ok_or("prompt tokens are a count")?;
        let completion_tokens = report
            .at("/authored/0/completion_tokens")?
            .as_u64()
            .ok_or("completion tokens are a count")?;
        assert!(
            prompt_tokens > 0 && completion_tokens > 0,
            "the kind's usage maps to the one shape: {prompt_tokens}/{completion_tokens}"
        );
        let bodies = mock.requests();
        assert_eq!(mock.served(), 1, "the task sends once");
        assert_eq!(bodies.len(), 1, "the one request is recorded");
        let sent: Value = json::from_slice(bodies.first().ok_or("one request")?)?;
        assert_eq!(
            sent.get("model").and_then(Value::as_str),
            Some("model-k"),
            "{kind}: the route's model reaches the gateway"
        );
        assert!(
            sent.get("stream").is_none(),
            "{kind}: no streaming key is sent"
        );
        if kind == "anthropic-messages" {
            assert_eq!(
                mock.authorizations(),
                [Some(String::from(KEY))],
                "messages carries the x-api-key and no bearer scheme"
            );
        } else {
            assert_eq!(
                mock.authorizations(),
                [Some(format!("Bearer {KEY}"))],
                "{kind}: the bearer key travels in the Authorization header"
            );
        }
        Ok(())
    })
}

/// The same task through a mock of each provider kind; each receipt names its
/// kind and usage maps to one shape. Bug caught: only the chat shape worked,
/// so Claude could never be a route.
#[test]
fn the_same_task_through_each_kind_names_it_and_maps_usage() -> TestResult {
    through_kind("openai-chat", 21)?;
    through_kind("anthropic-messages", 22)?;
    through_kind("openai-responses", 23)
}

/// A route that pins no temperature sends no `temperature` member on any kind:
/// a model that rejects the field answers with a 4xx, which is terminal.
#[test]
fn a_route_without_temperature_sends_no_temperature_member() -> TestResult {
    for (kind, seed) in [
        ("openai-chat", 31_u64),
        ("anthropic-messages", 32_u64),
        ("openai-responses", 33_u64),
    ] {
        let (state, origin) = fixture()?;
        run(Config::seeded(seed), |mock| -> TestResult {
            add_main_route(&state, mock, Some(kind), None, None)?;
            let output = exec(&state, &briefed_task(&origin, None, None)?)?;
            assert!(flag_of(&report_of(&output)?, "accepted")?);
            let bodies = mock.requests();
            assert_eq!(mock.served(), 1, "{kind}: nothing is resent");
            let text = String::from_utf8_lossy(bodies.first().ok_or("one request")?);
            assert!(
                !text.contains("temperature"),
                "{kind}: no temperature member: {text}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

/// The BLAKE3 hex of `template`.
fn template_hash(template: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(template.as_bytes());
    hasher.finalize().to_hex()
}

/// A route's prompt template rides before the system turn and only its hash
/// is sealed: `route resolve` and the receipt name the same hash, and neither
/// carries the template's text.
#[test]
fn a_prompt_template_is_sealed_by_its_hash() -> TestResult {
    let (state, origin) = fixture()?;
    run(Config::seeded(41), |mock| -> TestResult {
        add_main_route(&state, mock, Some("openai-chat"), Some("stay brief"), None)?;
        let resolved = larry_route(state.path(), &[], &["resolve"])?;
        assert!(resolved.status.success(), "{}", stderr_of(&resolved));
        let seal: Value = json::from_slice(&resolved.stdout)?;
        let want = template_hash("stay brief");
        assert_eq!(
            seal.at("/prompt_hash")?.as_str(),
            Some(want.as_str()),
            "resolve seals the hash"
        );
        assert!(
            !String::from_utf8_lossy(&resolved.stdout).contains("stay brief"),
            "resolve never prints the template"
        );
        let output = exec(&state, &briefed_task(&origin, None, None)?)?;
        let report = report_of(&output)?;
        assert!(flag_of(&report, "accepted")?, "{}", stderr_of(&output));
        assert_eq!(
            report.at("/authored/0/route/prompt_hash")?.as_str(),
            Some(want.as_str()),
            "the receipt seals the same hash"
        );
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("stay brief"),
            "the report never carries the template"
        );
        Ok(())
    })
}

/// `route add` refuses an unknown kind and a hostile template, naming the
/// field, and stores nothing.
#[test]
fn a_route_add_with_an_unknown_kind_or_hostile_prompt_is_refused() -> TestResult {
    let state = TempDir::new()?;
    let key_env = key_env(state.path(), "K")?;
    let secret: &OsStr = OsStr::new("operator-secret-bytes");
    let refused = larry_route(
        state.path(),
        &[(&key_env, secret)],
        &[
            "add",
            "x",
            "--url",
            "http://gateway.test:1/v1",
            "--model",
            "m",
            "--key-env",
            &key_env,
            "--residency",
            "CA",
            "--kind",
            "bogus-kind",
        ],
    )?;
    assert_eq!(refused.status.code(), Some(1), "an unknown kind is refused");
    assert!(
        stderr_of(&refused).contains("route kind must be one of"),
        "the refusal names the kind: {}",
        stderr_of(&refused)
    );
    let hostile = larry_route(
        state.path(),
        &[(&key_env, secret)],
        &[
            "add",
            "x",
            "--url",
            "http://gateway.test:1/v1",
            "--model",
            "m",
            "--key-env",
            &key_env,
            "--residency",
            "CA",
            "--prompt",
            "bad\u{1}byte",
        ],
    )?;
    assert_eq!(
        hostile.status.code(),
        Some(1),
        "a hostile template is refused"
    );
    assert!(
        stderr_of(&hostile).contains("route prompt must be"),
        "the refusal names the prompt: {}",
        stderr_of(&hostile)
    );
    let listed = larry_route(state.path(), &[], &["list"])?;
    assert!(
        listed.stdout.is_empty(),
        "no refused route was stored: {}",
        stderr_of(&listed)
    );
    Ok(())
}

/// A gateway fault on a new kind is typed, served once and never retried: a
/// delayed reply the test releases is accepted, while a padded, hostile, cut
/// or empty reply fails the task naming why. Catches an adapter that retries
/// a terminal fault or loses the reply's shape.
#[test]
fn a_gateway_fault_on_a_new_kind_is_typed_and_served_once() -> TestResult {
    let cases: &[(&str, Fault, &str, u64)] = &[
        ("openai-responses", Fault::PaddedDone, "malformed", 51),
        (
            "anthropic-messages",
            Fault::HostileRefusal { status: 401 },
            "401",
            52,
        ),
        (
            "openai-responses",
            Fault::Finish { reason: "length" },
            "did not finish",
            53,
        ),
        ("anthropic-messages", Fault::EmptyContent, "not JSON", 54),
    ];
    for (kind, fault, needle, seed) in cases.iter().copied() {
        let (state, origin) = fixture()?;
        let (output, served) =
            run_scripted(&state, &origin, kind, fault, seed, |state, origin, mock| {
                let output = exec(state, &briefed_task(origin, None, None)?)?;
                let served = mock.served();
                Ok((output, served))
            })?;
        assert!(!output.status.success(), "{kind}: {fault:?} fails the task");
        assert!(
            stderr_of(&output).contains(needle),
            "{kind}: {fault:?} names why: {}",
            stderr_of(&output)
        );
        assert_eq!(served, 1, "{kind}: {fault:?} is never retried");
    }
    Ok(())
}

/// One scripted mock run over the journey's route: the mock scripted with
/// `fault`, the route stored, then `body` drives the task.
fn run_scripted<Out>(
    state: &TempDir,
    origin: &TempDir,
    kind: &str,
    fault: Fault,
    seed: u64,
    body: impl FnOnce(&TempDir, &TempDir, &Handle) -> Result<Out, Box<dyn Error>>,
) -> Result<Out, Box<dyn Error>> {
    let config = Config {
        script: vec![Some(fault)],
        ..Config::seeded(seed)
    };
    run(config, |mock| -> Result<Out, Box<dyn Error>> {
        add_main_route(state, mock, Some(kind), None, None)?;
        body(state, origin, mock)
    })
}

/// A delayed reply on a new kind is released and accepted, and the same seed
/// over the same tenant replays the same trace: the gate, the credential and
/// the shape are deterministic.
#[test]
fn a_delayed_reply_on_a_new_kind_is_released_and_replays() -> TestResult {
    let (state, origin) = fixture()?;
    let mut hashes = Vec::new();
    for _ in 0..2 {
        let (output, served, trace) = run_scripted(
            &state,
            &origin,
            "anthropic-messages",
            Fault::Delay,
            61,
            |state, origin, mock| {
                let variable = key_env(state.path(), "K")?;
                let child = start_with_key(
                    state.path(),
                    &variable,
                    OsStr::new(KEY),
                    &briefed_task(origin, None, None)?,
                    &[],
                )?;
                mock.release_delayed();
                let output = child.wait_with_output()?;
                Ok((output, mock.served(), mock.trace_hash()))
            },
        )?;
        let report = report_of(&output)?;
        assert!(flag_of(&report, "accepted")?, "{}", stderr_of(&output));
        assert_eq!(served, 1, "the delayed reply sends once");
        hashes.push(trace);
    }
    let mut runs = hashes.into_iter();
    let (Some(first), Some(second), None) = (runs.next(), runs.next(), runs.next()) else {
        return Err("both delayed runs recorded their trace".into());
    };
    assert_eq!(first, second, "one seed, one trace hash across runs");
    Ok(())
}
