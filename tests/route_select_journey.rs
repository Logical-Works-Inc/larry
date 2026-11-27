//! Binary journeys of semantic route selection (larry#202): a briefed task
//! that names no route is filtered by the tenant's residency rule before any
//! byte leaves the host, the eligible route wins, and the sealed record says
//! how it was chosen. Every run drives the shipped binary with a cleared
//! environment against the seeded mock model endpoint, so no test makes a
//! paid model call. The ranked choice itself is proved in
//! `src/model/sim_select.rs` against a seeded scorer; these journeys prove
//! the run path around it on the default build (no `rerank` engine). The
//! tenants, doors and sealed-report assertions live in the shared
//! `route_journey` harness, so no journey copies them.
//!
//! Source: larry#202 (an explicit route always wins; residency and budget
//! filter before the ranking, never after; the rank, the scores and the
//! ranker's pinned model hash go into the sealed record; without the `rerank`
//! feature the resolver uses the tenant's default and the record says so).
//! Requirement proved: a task whose routes all break the tenant rule is
//! refused before the mock counts a request; the sole eligible route wins and
//! the record says `sole`; the eligible default wins and the record says
//! `default`; a named route wins past the rule and the record says `named`;
//! the same task chooses the same route twice; a delayed or padded gateway
//! still seals the choice; a gateway fault never reads as a selection refusal.

#[path = "support/http_wire.rs"]
mod http_wire;

#[path = "support/route_journey.rs"]
mod route_journey;
#[path = "support/route_tenant.rs"]
mod route_tenant;

use std::error::Error;
use std::ffi::OsStr;
use std::process::Output;

use route_journey::TestResult;
use route_journey::json_at::At;
use route_journey::larry_cli::stderr_of;
use route_journey::larry_route;
use route_journey::mock_model::{Config, Fault, Handle, run};
use route_journey::origin::{TempDir, fixture};
use route_journey::sealed_report::{denial_lines, flag_of, report_of};
use route_tenant::Tenant;

/// The ranker pin every selection seals, from `src/rerank/model.pin`.
const RANKER_MODEL: &str = "Qwen/Qwen3-Reranker-0.6B";
/// The pinned snapshot revision every selection seals.
const RANKER_REVISION: &str = "e61197ed45024b0ed8a2d74b80b4d909f1255473";

/// Two tenants sharing one state, origin and gateway: `de` serves model-de
/// from DE, `ca` serves model-ca from CA. Each stores its route; the caller
/// names the default, since every add marks one.
pub(crate) fn two_routes<'tenant>(
    state: &'tenant TempDir,
    origin: &'tenant TempDir,
    mock: &'tenant Handle,
) -> Result<(Tenant<'tenant>, Tenant<'tenant>), Box<dyn Error>> {
    let de = Tenant {
        state,
        origin,
        mock,
        model: "model-de",
        residency: "DE",
        suffix: "DE",
        key: "secret-de",
    };
    de.add_route("de")?;
    let ca = Tenant {
        model: "model-ca",
        residency: "CA",
        suffix: "CA",
        key: "secret-ca",
        ..Tenant::standard(state, origin, mock)
    };
    ca.add_route("ca")?;
    Ok((de, ca))
}

/// Makes `id` the tenant's default route.
pub(crate) fn make_default(state: &TempDir, id: &str) -> TestResult {
    let output = larry_route(state.path(), &[], &["default", id])?;
    assert!(
        output.status.success(),
        "route default {id}: {}",
        stderr_of(&output)
    );
    Ok(())
}

/// The sealed selection method of an accepted exec `output` over `route`,
/// with the ranker pin asserted on every path.
pub(crate) fn selection_method(output: &Output, route: &str) -> Result<String, Box<dyn Error>> {
    let report = report_of(output)?;
    assert!(flag_of(&report, "accepted")?, "{}", stderr_of(output));
    assert!(denial_lines(&report)?.is_empty(), "no denial was sealed");
    assert_eq!(
        report.at("/authored/0/route/id")?.as_str(),
        Some(route),
        "the report names the winning route"
    );
    let selection = report.at("/authored/0/route_selection")?;
    for (member, want) in [("model", RANKER_MODEL), ("revision", RANKER_REVISION)] {
        assert_eq!(
            selection.at(&format!("/{member}"))?.as_str(),
            Some(want),
            "the record names the ranker pin"
        );
    }
    selection
        .at("/method")?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "a selection method".into())
}

/// Runs the unnamed task for the Canadian route under a Canadian rule and
/// returns the serving tenant with the output: the shared setup of the
/// journeys that expect a sealed `sole` choice.
fn sealed_canadian<'tenant>(
    state: &'tenant TempDir,
    origin: &'tenant TempDir,
    mock: &'tenant Handle,
) -> Result<(Tenant<'tenant>, Output), Box<dyn Error>> {
    let (de, ca) = two_routes(state, origin, mock)?;
    make_default(state, "de")?;
    let output = ca.exec_with(
        &de.briefed(None, None)?,
        &[("LARRY_RESIDENCY", OsStr::new("CA"))],
    )?;
    Ok((ca, output))
}

/// A task whose routes all break the tenant rule is refused before any byte
/// leaves the host: neither the German default nor the Canadian route may
/// serve a US-only tenant (larry#202).
#[test]
fn selection_refuses_when_no_route_survives_the_residency_rule() -> TestResult {
    let (state, origin) = fixture()?;
    run(Config::seeded(11), |mock| -> TestResult {
        let (de, _ca) = two_routes(&state, &origin, mock)?;
        make_default(&state, "de")?;
        let output = de.exec_with(
            &de.briefed(None, None)?,
            &[("LARRY_RESIDENCY", OsStr::new("US"))],
        )?;
        assert!(!output.status.success(), "an ineligible field refuses");
        assert!(
            stderr_of(&output).contains("no stored route is eligible"),
            "the refusal names the filter: {}",
            stderr_of(&output)
        );
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
        assert!(mock.authorizations().is_empty(), "no header left the host");
        Ok(())
    })
}

/// The sole eligible route wins without a ranking: under a Canadian rule the
/// Canadian route serves the task the German default cannot, and the record
/// says `sole` (larry#202).
#[test]
fn selection_sends_the_sole_eligible_route_and_seals_sole() -> TestResult {
    let (state, origin) = fixture()?;
    run(Config::seeded(12), |mock| -> TestResult {
        let (ca, output) = sealed_canadian(&state, &origin, mock)?;
        assert_eq!(selection_method(&output, "ca")?, "sole");
        ca.assert_sealed_under(&output, "ca")?;
        Ok(())
    })
}

/// The eligible default wins when nothing is filtered: with no tenant rule
/// the German default serves the unnamed task, and the record says `default`
/// (larry#202).
#[test]
fn selection_sends_the_eligible_default_and_seals_default() -> TestResult {
    let (state, origin) = fixture()?;
    run(Config::seeded(13), |mock| -> TestResult {
        let (de, _ca) = two_routes(&state, &origin, mock)?;
        make_default(&state, "de")?;
        let output = de.exec(&de.briefed(None, None)?)?;
        assert_eq!(selection_method(&output, "de")?, "default");
        de.assert_sealed_under(&output, "de")?;
        Ok(())
    })
}

/// An explicit route always wins: a task naming the German route is served
/// under a Canadian rule that would filter it from any ranking, and the
/// record says `named` (larry#202).
#[test]
fn selection_sends_the_named_route_past_the_rule_and_seals_named() -> TestResult {
    let (state, origin) = fixture()?;
    run(Config::seeded(14), |mock| -> TestResult {
        let (de, _ca) = two_routes(&state, &origin, mock)?;
        make_default(&state, "ca")?;
        let output = de.exec_with(
            &de.briefed(Some("de"), None)?,
            &[("LARRY_RESIDENCY", OsStr::new("CA"))],
        )?;
        assert_eq!(selection_method(&output, "de")?, "named");
        de.assert_sealed_under(&output, "de")?;
        Ok(())
    })
}

/// The same unnamed task chooses the same route twice: two runs against two
/// identically seeded gateways seal the same method over the same route with
/// the same gateway trace hash, so a replay answers from the record
/// (larry#202).
#[test]
fn selection_is_deterministic_across_runs() -> TestResult {
    fn run_once(
        state: &TempDir,
        origin: &TempDir,
        seed: u64,
    ) -> Result<(String, String), Box<dyn Error>> {
        run(Config::seeded(seed), |mock| {
            let (_ca, output) = sealed_canadian(state, origin, mock)?;
            Ok((selection_method(&output, "ca")?, mock.trace_hash()))
        })
    }
    let (state, origin) = fixture()?;
    let first = run_once(&state, &origin, 15)?;
    let second = run_once(&state, &origin, 15)?;
    assert_eq!(first, second, "one seed seals one choice and one trace");
    assert_eq!(first.0, "sole", "the record says sole");
    Ok(())
}

/// A delayed gateway still seals the choice: the winning route serves the
/// task once the test releases the gate, and the record says `sole`
/// (larry#202).
#[test]
fn selection_seals_through_a_delayed_gateway() -> TestResult {
    let (state, origin) = fixture()?;
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(16)
    };
    run(config, |mock| -> TestResult {
        let (de, ca) = two_routes(&state, &origin, mock)?;
        make_default(&state, "de")?;
        let task = de.briefed(None, None)?;
        let child = ca.start(&task, &[("LARRY_RESIDENCY", OsStr::new("CA"))])?;
        mock.release_delayed();
        let output = child.wait_with_output()?;
        assert_eq!(selection_method(&output, "ca")?, "sole");
        ca.assert_sealed_under(&output, "ca")?;
        Ok(())
    })
}

/// A padded gateway reply still seals the choice: the winning route serves
/// the task through the OpenRouter-style padding, and the record says `sole`
/// (larry#202).
#[test]
fn selection_seals_over_a_padded_reply() -> TestResult {
    let (state, origin) = fixture()?;
    let config = Config {
        script: vec![Some(Fault::PaddedDone)],
        ..Config::seeded(17)
    };
    run(config, |mock| -> TestResult {
        let (ca, output) = sealed_canadian(&state, &origin, mock)?;
        assert_eq!(selection_method(&output, "ca")?, "sole");
        ca.assert_sealed_under(&output, "ca")?;
        Ok(())
    })
}

/// A gateway fault is never a selection refusal: on the winning route a 401,
/// a cut reply and an empty reply each fail the task naming the gateway's
/// reason, never the residency or ranker refusals, and no selection is
/// sealed (larry#202).
#[test]
fn selection_distinguishes_gateway_faults_from_selection_refusals() -> TestResult {
    let (state, origin) = fixture()?;
    for (fault, why) in [
        (
            Fault::HostileRefusal { status: 401 },
            "the model gateway refused with status 401",
        ),
        (Fault::Finish { reason: "length" }, "finish_reason length"),
        (Fault::EmptyContent, "reply is empty"),
    ] {
        let config = Config {
            script: vec![Some(fault)],
            ..Config::seeded(18)
        };
        run(config, |mock| -> TestResult {
            let (_ca, output) = sealed_canadian(&state, &origin, mock)?;
            assert!(!output.status.success(), "{fault:?} fails the task");
            let stderr = stderr_of(&output);
            assert!(stderr.contains(why), "{fault:?}: {stderr}");
            for selection_word in ["no stored route is eligible", "no route ranker"] {
                assert!(
                    !stderr.contains(selection_word),
                    "{fault:?} is a gateway fault, not a selection refusal: {stderr}"
                );
            }
            assert_eq!(mock.served(), 1, "{fault:?} is never retried");
            Ok(())
        })?;
    }
    Ok(())
}
