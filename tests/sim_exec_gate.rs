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

use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;

use larry_cli::{denial_lines, flag_of, report_of, spawn_larry};
use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;
use mock_model::Config;
use origin::{TempDir, fixture};

/// How many cases the simulation drives, each drawn from the one seeded
/// stream on [`SIM_SEED`], and the whole matrix run twice so both passes must
/// match. Two 256-case passes measured 7.5 s.
const SEEDS: usize = 256;

/// The seed every case draws from, identical in both passes.
const SIM_SEED: u64 = 0x5EED_0001;

/// The five policy names, in `Scope` order.
const NAMES: [&str; 5] = [
    "file.read",
    "file.write",
    "net.egress",
    "process.spawn",
    "model.invoke",
];

/// Every fault the mock endpoint can answer with, named here so scheduling
/// one in a gate run is a deliberate act that extends this list first.
const EVERY_FAULT: [Option<mock_model::Fault>; 7] = [
    Some(mock_model::Fault::Delay),
    Some(mock_model::Fault::RateLimit {
        retry_after_secs: 1,
    }),
    Some(mock_model::Fault::MalformedJson),
    Some(mock_model::Fault::TruncatedBody),
    Some(mock_model::Fault::ServerError { status: 500 }),
    Some(mock_model::Fault::PaddedDone),
    Some(mock_model::Fault::HostileRefusal { status: 401 }),
];

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The scopes one task requires, in `Scope` order: read and spawn always,
/// write and model invoke when the task carries a brief.
fn required(brief: bool) -> Vec<&'static str> {
    match brief {
        true => Vec::from(["file.read", "file.write", "process.spawn", "model.invoke"]),
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
    let env: [(&str, &OsStr); 4] = [
        ("LARRY_STATE_DIR", state.as_os_str()),
        ("LARRY_POLICY", OsStr::new(policy_text.as_str())),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_URL", OsStr::new(url)),
    ];
    let output = spawn_larry("exec", &env, task.as_bytes())?.wait_with_output()?;
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

/// One pass over the seeded case matrix: draws every case from a fresh stream
/// on [`SIM_SEED`], drives the binary, asserts each verdict against the
/// oracle, and folds the verdict sequence into `trace`.
fn run_pass(state: &TempDir, origin: &TempDir, url: &str, trace: &mut Hasher) -> TestResult {
    let mut rng = Seeded::from_seed(SIM_SEED);
    for index in 0..SEEDS {
        let mut policy: Vec<&'static str> = Vec::new();
        for name in NAMES {
            if rng.below(2)? == 1 {
                policy.push(name);
            }
        }
        let brief = rng.below(2)? == 1;
        let verdict = run_case(state.path(), origin.path(), url, &policy, brief)?;
        let expected = oracle(&policy, brief);
        assert_eq!(
            verdict,
            expected,
            "seed {SIM_SEED} case {index} (policy {:?}, brief {brief}): the binary must match the oracle",
            policy.join(",")
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
        7,
        "every fault the endpoint can answer with is named"
    );
}

#[test]
fn sim_exec_gate_matches_the_scope_rule_for_every_seed() -> TestResult {
    let (state, origin) = fixture()?;
    let config = Config::seeded(SIM_SEED);
    mock_model::run(config, |mock| -> TestResult {
        let url = format!("http://127.0.0.1:{}/v1", mock.addr().port());
        let empty_chain = Hasher::new().finalize().to_hex();
        assert_eq!(
            mock.trace_hash(),
            empty_chain,
            "an endpoint that has served nothing carries the empty chain"
        );
        let mut verdicts = Hasher::new();
        run_pass(&state, &origin, &url, &mut verdicts)?;
        let served_first = mock.served();
        let first_bodies = mock.requests();
        let first_keys = mock.authorizations();
        let mut replayed = Hasher::new();
        run_pass(&state, &origin, &url, &mut replayed)?;
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
        // The mock's reply is a pure function of its seed, the request index
        // and the request body, so byte-identical request bodies at the same
        // positions are the endpoint-side half of the replay proof.
        let both_bodies = mock.requests();
        assert_eq!(
            usize::try_from(served_both),
            Ok(both_bodies.len()),
            "every served request is recorded"
        );
        let replayed_bodies = both_bodies
            .get(first_bodies.len()..)
            .ok_or("the second pass is recorded after the first")?;
        assert_eq!(
            replayed_bodies,
            first_bodies.as_slice(),
            "same seed, same trace: the second pass's request bodies replay the first pass byte for byte"
        );
        let both_keys = mock.authorizations();
        let replayed_keys = both_keys
            .get(first_keys.len()..)
            .ok_or("the second pass is recorded after the first")?;
        assert_eq!(
            replayed_keys,
            first_keys.as_slice(),
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
