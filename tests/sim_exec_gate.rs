//! Seeded simulation of the executor's gate scope rule (larry#164).
//!
//! Source: OWASP ASVS 4.0.3 chapter V4 (access control) — a request is
//! admitted exactly when the policy grants every scope the action needs, and
//! is denied by default otherwise, with the deficit named; plus the estate's
//! seeded-simulation rule (AGENTS.md: one seed, same seed same trace hash).
//! Requirement proved: across the policy subset space and both brief shapes,
//! the shipped `larry exec` binary admits exactly the tasks the independent
//! oracle admits, a denial names exactly the oracle's first refused scope in
//! `Scope` order, an admitted task is accepted, and the same seed replays the
//! same verdict sequence with byte-identical model requests.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;
#[path = "support/sealed_report.rs"]
mod sealed_report;
#[path = "../src/sim_lanes.rs"]
mod sim_lanes;

use std::error::Error;
use std::ffi::OsStr;
use std::num::TryFromIntError;
use std::path::Path;

use larry_cli::spawn_larry_args;
use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;
use mock_model::Config;
use origin::{TempDir, fixture};
use sealed_report::{denial_lines, flag_of, report_of};

/// How many cases the simulation drives, each drawn from the one seeded
/// stream on [`SIM_SEED`], and the whole matrix run twice so both passes must
/// match. Two 256-case passes ran serially in 7.5 s; they now run on bounded
/// lanes, each with its own state directory and origin repository.
const SEEDS: usize = 256;

/// Most connections the mock admits at once: four per lane at the lane cap, so
/// no concurrent case is turned away with a 503.
const MOCK_CONNECTIONS: usize = 64;

/// The seed every case draws from, identical in both passes.
const SIM_SEED: u64 = 0x5EED_0001;

/// The five policy names, in `Scope` order, each once.
const NAMES: [&str; 5] = [
    "file.read",
    "file.write",
    "net.egress",
    "process.spawn",
    "model.invoke",
];

/// Every fault the mock endpoint can answer with, named here so scheduling
/// one in a gate run is a deliberate act that extends this list first.
const EVERY_FAULT: [Option<mock_model::Fault>; 9] = [
    Some(mock_model::Fault::Delay),
    Some(mock_model::Fault::RateLimit {
        retry_after_secs: 1,
    }),
    Some(mock_model::Fault::MalformedJson),
    Some(mock_model::Fault::TruncatedBody),
    Some(mock_model::Fault::ServerError { status: 500 }),
    Some(mock_model::Fault::PaddedDone),
    Some(mock_model::Fault::HostileRefusal { status: 401 }),
    Some(mock_model::Fault::Finish { reason: "length" }),
    Some(mock_model::Fault::EmptyContent),
];

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The scopes one task requires, in `Scope` order: read and spawn always,
/// write, model invoke and network egress when the task carries a brief.
fn required(brief: bool) -> Vec<&'static str> {
    match brief {
        true => Vec::from([
            "file.read",
            "file.write",
            "net.egress",
            "process.spawn",
            "model.invoke",
        ]),
        false => Vec::from(["file.read", "process.spawn"]),
    }
}

/// The independent oracle: the verdict the binary must produce. A task is
/// admitted exactly when the policy admits every scope it requires; otherwise
/// the denial names the first required scope, in `Scope` order, the policy
/// refuses.
fn oracle(policy: &[&str], brief: bool) -> String {
    for name in required(brief) {
        if !policy.contains(&name) {
            return format!("gate: policy_refuses: {name}");
        }
    }
    String::from("allowed")
}

/// One task document: a brief the model writes `note.txt` for, or a plain one,
/// each with one passing acceptance command.
fn document(origin: &Path, brief: bool) -> String {
    let goal = match brief {
        true => r#""goal":"write the note","files":["note.txt"],"#,
        false => "",
    };
    format!(
        r#"{{"repo":"{}","base":"HEAD",{goal}"acceptance":[["true"]]}}"#,
        origin.display()
    )
}

/// Drives one case through the shipped binary and returns its verdict line:
/// `allowed` for an accepted run, else the exact denial line the report seals.
fn run_case(
    state: &Path,
    origin: &Path,
    url: &str,
    policy: &[&str],
    brief: bool,
) -> Result<String, Box<dyn Error>> {
    let policy_text = policy.join(",");
    let task = document(origin, brief);
    let env: [(&str, &OsStr); 5] = [
        ("LARRY_STATE_DIR", state.as_os_str()),
        ("LARRY_POLICY", OsStr::new(policy_text.as_str())),
        ("LARRY_MODEL", OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_URL", OsStr::new(url)),
    ];
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    let report = report_of(&output)?;
    let denials = denial_lines(&report)?;
    if flag_of(&report, "accepted")? {
        assert!(
            denials.is_empty(),
            "an accepted run seals no denial: {denials:?}"
        );
        return Ok(String::from("allowed"));
    }
    let line = denials
        .first()
        .ok_or("a denied report names its first denial")?;
    Ok(String::from(line))
}

/// One case: the policy subset and whether the task carries a brief.
type Case = (Vec<&'static str>, bool);

/// Why one case could not be driven to a verdict inside its lane.
#[derive(Debug, lgwks_macros::Error)]
enum CaseFault {
    /// The binary could not be spawned or its report not read; the cause as text, because a
    /// boxed error cannot cross a lane.
    #[error("case run: {0}")]
    Run(String),
    /// A lane or case index names a fixture or case that is not there.
    #[error("case or lane index out of range")]
    Index,
    /// A lane or case index does not fit `usize`.
    #[error("case or lane index too wide: {0}")]
    Width(#[source] TryFromIntError),
}

/// Every case, drawn in order from a fresh stream on [`SIM_SEED`]: the same sequence in both
/// passes, whatever the lanes.
fn draw_cases() -> Result<Vec<Case>, Box<dyn Error>> {
    let mut rng = Seeded::from_seed(SIM_SEED);
    let mut cases = Vec::with_capacity(SEEDS);
    for _ in 0..SEEDS {
        let mut policy: Vec<&'static str> = Vec::new();
        for name in NAMES {
            if rng.below(2)? == 1 {
                policy.push(name);
            }
        }
        cases.push((policy, rng.below(2)? == 1));
    }
    Ok(cases)
}

/// One pass over the seeded case matrix: every case is driven through the binary on a bounded
/// lane (lane `n` owns `fixtures[n]`, one per `sim_lanes::lanes()`, the same lane for a case in
/// both passes), each verdict is
/// asserted against the oracle in case order, and the verdict sequence is folded into `trace`.
fn run_pass(
    fixtures: &[(TempDir, TempDir)],
    cases: &[Case],
    url: &str,
    trace: &mut Hasher,
) -> TestResult {
    let count = u64::try_from(cases.len())?;
    let verdicts = sim_lanes::sweep(count, |lane, index| {
        let at = |position: u64| usize::try_from(position).map_err(CaseFault::Width);
        let fixture = fixtures.get(at(lane)?).ok_or(CaseFault::Index)?;
        let case = cases.get(at(index)?).ok_or(CaseFault::Index)?;
        run_case(fixture.0.path(), fixture.1.path(), url, &case.0, case.1)
            .map_err(|cause| CaseFault::Run(cause.to_string()))
    })?;
    for (index, (verdict, case)) in verdicts.into_iter().zip(cases).enumerate() {
        let verdict = verdict?;
        let expected = oracle(&case.0, case.1);
        assert_eq!(
            verdict,
            expected,
            "seed {SIM_SEED} case {index} (policy {:?}, brief {}): the binary must match the oracle",
            case.0.join(","),
            case.1
        );
        trace.write_framed(verdict.as_bytes());
    }
    Ok(())
}

#[test]
fn the_gate_simulation_schedules_no_fault() {
    // The oracle reads only the scope rule; a fault would make a verdict
    // about the model client, which its own simulations own.
    let config = Config::seeded(SIM_SEED);
    assert!(
        config.script.is_empty(),
        "the gate simulation scripts no fault"
    );
    assert_eq!(
        config.fault_per_mille, 0,
        "the gate simulation rolls no unscripted fault"
    );
    assert_eq!(
        EVERY_FAULT.len(),
        9,
        "every fault the endpoint can answer with is named"
    );
}

#[test]
fn sim_exec_gate_matches_the_scope_rule_for_every_seed() -> TestResult {
    // One state directory and origin repository per lane, shared by both passes.
    let lanes = sim_lanes::lanes();
    let fixtures = (0..lanes)
        .map(|_| fixture())
        .collect::<Result<Vec<_>, _>>()?;
    let cases = draw_cases()?;
    let config = Config {
        max_connections: MOCK_CONNECTIONS,
        ..Config::seeded(SIM_SEED)
    };
    mock_model::run(config, |mock| -> TestResult {
        let url = format!("http://127.0.0.1:{}/v1", mock.addr().port());
        let empty_chain = Hasher::new().finalize().to_hex();
        assert_eq!(
            mock.trace_hash(),
            empty_chain,
            "an endpoint that has served nothing carries the empty chain"
        );
        let mut verdicts = Hasher::new();
        run_pass(&fixtures, &cases, &url, &mut verdicts)?;
        let served_first = mock.served();
        let mut first_bodies = mock.requests();
        let mut first_keys = mock.authorizations();
        let mut replayed = Hasher::new();
        run_pass(&fixtures, &cases, &url, &mut replayed)?;
        assert_eq!(
            verdicts.finalize().to_hex(),
            replayed.finalize().to_hex(),
            "same seed, same trace: the verdict sequence must replay identically"
        );
        let served_both = mock.served();
        assert_eq!(
            served_both,
            served_first.saturating_mul(2),
            "the second pass must make exactly the first pass's model requests"
        );
        // Cases now reach the endpoint from several lanes, so arrival order within
        // a pass is the scheduler's; the request bodies of a pass are compared as
        // sorted multisets, which still proves the second pass sent exactly the
        // first pass's requests, byte for byte. The verdict trace above stays
        // ordered by case.
        let both_bodies = mock.requests();
        assert_eq!(
            usize::try_from(served_both),
            Ok(both_bodies.len()),
            "every served request is recorded"
        );
        let mut replayed_bodies = both_bodies
            .get(first_bodies.len()..)
            .ok_or("the second pass is recorded after the first")?
            .to_vec();
        first_bodies.sort();
        replayed_bodies.sort();
        assert_eq!(
            replayed_bodies, first_bodies,
            "same seed, same trace: the second pass's request bodies replay the first pass byte for byte"
        );
        let both_keys = mock.authorizations();
        let mut replayed_keys = both_keys
            .get(first_keys.len()..)
            .ok_or("the second pass is recorded after the first")?
            .to_vec();
        first_keys.sort();
        replayed_keys.sort();
        assert_eq!(
            replayed_keys, first_keys,
            "same seed, same trace: the second pass's authorizations replay the first pass"
        );
        assert_eq!(
            mock.trace_hash() == empty_chain,
            mock.served() == 0,
            "the chain moves exactly when a request is served"
        );
        // No reply in this simulation is ever delayed; draining once at the
        // end is a no-op that documents the gate never held one.
        mock.release_delayed();
        Ok(())
    })?;
    Ok(())
}
