//! The hosted test lanes of `.github/workflows/sim.yml` and `ci.yml` never pass or cancel without running their
//! tests. Source: PR #165, whose e2e/journey job hung 15 minutes in the nextest download and was
//! cancelled before a test ran; GitHub Actions workflow syntax (`timeout-minutes` and
//! `continue-on-error` on a step, `steps.<id>.outcome`). Requirement: each lane's install is
//! bounded and may fail, nextest runs only after it succeeded, and `cargo test` runs the same
//! target otherwise.

use std::path::Path;

/// Failures propagate instead of panicking.
type Check = Result<(), Box<dyn std::error::Error>>;

/// One job of the workflow: its text from its key to the next job's key.
fn job<'a>(text: &'a str, key: &str) -> Result<&'a str, String> {
    let marker = format!("\n  {key}:\n");
    let (_, rest) = text
        .split_once(marker.as_str())
        .ok_or(format!("the workflow has no {key} job"))?;
    Ok(match rest.split_once("\n  # ") {
        Some((body, _)) => body,
        None => rest,
    })
}

/// Asserts the bounded install and both runners for one lane: `nextest` is its nextest command
/// and `fallback` its `cargo test` command, each as the workflow spells it.
fn assert_lane(text: &str, key: &str, nextest: &str, fallback: &str) -> Check {
    let body = job(text, key)?;
    assert!(
        body.contains("id: nextest\n        continue-on-error: true\n        timeout-minutes: 3\n"),
        "{key}: the nextest install is bounded at 3 minutes and may fail"
    );
    assert!(
        body.contains(&format!(
            "if: ${{{{ !github.event.repository.private && steps.nextest.outcome == 'success' }}}}\n        run: {nextest}"
        )),
        "{key}: nextest runs only after its install succeeded"
    );
    assert!(
        body.contains(&format!(
            "if: ${{{{ github.event.repository.private || steps.nextest.outcome != 'success' }}}}\n        run: {fallback}"
        )),
        "{key}: cargo test runs the lane whenever nextest is missing"
    );
    Ok(())
}

/// The text of the workflow file `name` under `.github/workflows`.
fn workflow(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github/workflows")
        .join(name);
    Ok(std::fs::read_to_string(path)?)
}

#[test]
fn every_hosted_lane_runs_its_tests_even_when_the_nextest_install_hangs() -> Check {
    let sim = workflow("sim.yml")?;
    assert_lane(
        &sim,
        "simulation",
        "cargo nextest run --locked -p larry-runtime",
        "cargo test --locked -p larry-runtime",
    )?;
    assert_lane(
        &sim,
        "e2e-journey",
        "cargo nextest run --locked -p larry-demo --test e2e_journey",
        "cargo test --locked -p larry-demo --test e2e_journey",
    )?;
    // The rust lane hung in the same download on PR #165's second run. Its nextest line is the
    // one `cargo local-ci` runs (governance_journey's ci_runs_the_test_lane_local_ci_runs).
    let ci = workflow("ci.yml")?;
    assert_lane(
        &ci,
        "rust",
        "cargo nextest run --workspace --locked",
        "cargo test --workspace --all-targets --locked",
    )?;
    Ok(())
}
