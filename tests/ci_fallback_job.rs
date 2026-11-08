//! The hosted lanes of `.github/workflows/sim.yml` never pass or cancel without running their
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
        .ok_or(format!("sim.yml has no {key} job"))?;
    Ok(match rest.split_once("\n  # ") {
        Some((body, _)) => body,
        None => rest,
    })
}

/// Asserts the bounded install and both runners for one lane and its test target.
fn assert_lane(text: &str, key: &str, target: &str) -> Check {
    let body = job(text, key)?;
    assert!(
        body.contains("id: nextest\n        continue-on-error: true\n        timeout-minutes: 3\n"),
        "{key}: the nextest install is bounded at 3 minutes and may fail"
    );
    assert!(
        body.contains(&format!(
            "if: ${{{{ !github.event.repository.private && steps.nextest.outcome == 'success' }}}}\n        run: cargo nextest run --locked {target}"
        )),
        "{key}: nextest runs only after its install succeeded"
    );
    assert!(
        body.contains(
            "if: ${{ github.event.repository.private || steps.nextest.outcome != 'success' }}\n        run: cargo test --locked"
        ),
        "{key}: cargo test runs the lane whenever nextest is missing"
    );
    Ok(())
}

#[test]
fn every_hosted_lane_runs_its_tests_even_when_the_nextest_install_hangs() -> Check {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/sim.yml"),
    )?;
    assert_lane(&text, "simulation", "-p larry-runtime")?;
    assert_lane(&text, "e2e-journey", "-p larry-demo --test e2e_journey")?;
    Ok(())
}
