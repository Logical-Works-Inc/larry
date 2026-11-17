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
    // The test lane hung in the same download on PR #165's second run. Its
    // nextest line is the one `cargo local-ci` runs (governance_journey's
    // ci_runs_the_test_lane_local_ci_runs).
    let ci = workflow("ci.yml")?;
    assert_lane(
        &ci,
        "test",
        "cargo nextest run --workspace --locked",
        "cargo test --workspace --all-targets --locked",
    )?;
    Ok(())
}

/// The four workflow files, in the order this test reads them.
fn workflows() -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let mut all = Vec::new();
    for name in ["ci.yml", "sim.yml", "governance.yml", "mutants.yml"] {
        all.push((name.to_owned(), workflow(name)?));
    }
    Ok(all)
}

#[test]
fn every_workflow_cancels_a_superseded_pull_request_run_only() -> Check {
    for (name, text) in workflows()? {
        assert!(
            text.contains(
                "concurrency:\n  group: ${{ github.workflow }}-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n"
            ),
            "{name}: one concurrency group per workflow and ref, cancelled for pull requests only \
             (a main run is the proof of the landed commit)"
        );
    }
    Ok(())
}

#[test]
fn every_job_owns_its_cache_and_saves_it_only_from_main() -> Check {
    // Each job's own key, saved only from main (larry#307). A pull request
    // restores a key and never writes what main restores.
    let own = [
        "shared-key: lint-${{ github.event.repository.private && 'self-hosted' || 'hosted' }}",
        "shared-key: rerank-lint-${{ github.event.repository.private && 'self-hosted' || 'hosted' }}",
        "shared-key: test-${{ github.event.repository.private && 'self-hosted' || 'hosted' }}",
        "shared-key: simulation-${{ github.event.repository.private && 'self-hosted' || 'hosted' }}",
        "shared-key: journey-${{ github.event.repository.private && 'self-hosted' || 'hosted' }}",
        "shared-key: governance-${{ github.event.repository.private && 'self-hosted' || 'hosted' }}",
        "shared-key: journey-macos",
    ];
    let mut seen = 0;
    for (name, text) in workflows()? {
        let caches = text.matches("uses: Swatinem/rust-cache@").count();
        let saves = text.matches("          save-if:").count();
        assert_eq!(
            caches, saves,
            "{name}: every cache step saves under one condition"
        );
        for key in own {
            if text.contains(key) {
                seen += 1;
            }
        }
    }
    assert_eq!(
        seen,
        own.len(),
        "each of the seven jobs owns its own shared-key: {own:?}"
    );
    Ok(())
}

#[test]
fn a_pull_request_never_writes_the_cache_main_restores() -> Check {
    for (name, text) in workflows()? {
        if !text.contains("uses: Swatinem/rust-cache@") {
            continue;
        }
        let saves = text
            .matches("save-if: ${{ github.ref == 'refs/heads/main' }}")
            .count();
        assert!(saves > 0, "{name}: the cache saves only from main");
        assert!(
            !text.contains("save-if: true") && !text.contains("save-if: false"),
            "{name}: no cache step saves unconditionally or never"
        );
    }
    Ok(())
}

// The mutation lane (larry#310, ADR 0064). Its verdict logic ran against synthetic outcomes while
// it was written; these tests hold the text that makes it fail closed, so an edit cannot drop one.

#[test]
fn the_mutants_lane_passes_a_diff_with_no_rust_lines() -> Check {
    let text = workflow("mutants.yml")?;
    let shard = job(&text, "shard")?;
    assert!(
        shard.contains("if: ${{ needs.plan.outputs.shards != '0' }}"),
        "zero shards skips the shard job, whose empty matrix GitHub would refuse"
    );
    assert!(
        text.contains("(.[0] // []) | length"),
        "an empty diff prints nothing, not an array, and counts as zero mutants"
    );
    let verdict = job(&text, "verdict")?;
    assert!(
        verdict.contains("if [ \"$SHARDS\" = 0 ]; then\n            echo \"no mutants:"),
        "zero shards is a pass that says no mutants"
    );
    Ok(())
}

#[test]
fn the_mutants_verdict_fails_closed_on_a_lost_shard() -> Check {
    let text = workflow("mutants.yml")?;
    let verdict = job(&text, "verdict")?;
    assert!(
        verdict.contains("needs: [plan, shard]")
            && verdict.contains("if: ${{ always() && !github.event.repository.private }}"),
        "the verdict runs after any outcome of the plan and the shards"
    );
    for refusal in [
        "if [ \"$PLAN\" != success ] || [ -z \"$SHARDS\" ]; then",
        "if [ \"$SHARD\" != success ]; then",
        "outcomes files for $SHARDS shards (fail closed)",
        "mutants have an outcome but the plan listed $COUNT (fail closed)",
        "mutants have an outcome this verdict does not know (fail closed)",
    ] {
        assert!(
            verdict.contains(refusal),
            "the verdict keeps its refusal: {refusal}"
        );
    }
    let shard = job(&text, "shard")?;
    assert!(
        shard.contains("0 | 2 | 3) echo") && shard.contains("exit \"$rc\""),
        "a shard treats exit 0, 2 and 3 as outcomes and every other code as a shard that did not run"
    );
    Ok(())
}

#[test]
fn the_mutants_lane_caps_its_shards_and_bounds_each_one() -> Check {
    let text = workflow("mutants.yml")?;
    assert!(
        text.contains("MUTANTS_MAX_SHARDS: 16"),
        "the matrix is capped at 16 shards"
    );
    let shard = job(&text, "shard")?;
    assert!(
        shard.contains("timeout-minutes: 15\n    strategy:\n      fail-fast: false\n"),
        "an overrunning shard ends at its timeout and never stops its siblings"
    );
    assert!(
        shard.contains("--timeout 300"),
        "a mutant that loops is a timeout outcome, not a held job"
    );
    Ok(())
}

#[test]
fn the_mutants_lane_never_writes_the_cache_main_restores() -> Check {
    let text = workflow("mutants.yml")?;
    assert_eq!(
        text.matches("uses: Swatinem/rust-cache@").count(),
        1,
        "only the shard builds, so only the shard restores a cache"
    );
    assert!(
        text.contains("shared-key: test-hosted")
            && text.contains("save-if: ${{ github.ref == 'refs/heads/main' }}"),
        "the lane restores the test lane's key and saves only from main, which it never runs on"
    );
    assert!(
        text.contains("cancel-in-progress: ${{ github.event_name == 'pull_request' }}"),
        "a second push cancels only a pull request run"
    );
    Ok(())
}

#[test]
fn the_mutants_lane_pins_every_action_and_shares_one_diff() -> Check {
    let text = workflow("mutants.yml")?;
    let mut pinned = 0;
    for line in text.lines().map(str::trim_start) {
        let Some(action) = line
            .strip_prefix("- uses: ")
            .or_else(|| line.strip_prefix("uses: "))
        else {
            continue;
        };
        let (_, rest) = action
            .split_once('@')
            .ok_or(format!("an action with no pin: {action}"))?;
        let sha = rest
            .get(..40)
            .ok_or(format!("a pin shorter than a commit: {action}"))?;
        assert!(
            sha.chars().all(|digit| digit.is_ascii_hexdigit()),
            "every action is pinned to a full commit: {action}"
        );
        pinned += 1;
    }
    assert!(pinned >= 8, "the lane reads its actions: {pinned}");
    assert_eq!(
        text.matches("uses: actions/upload-artifact@").count(),
        text.matches("overwrite: true").count(),
        "a re-run of a failed shard replaces its artifact instead of failing on it"
    );
    assert_eq!(
        text.matches("name: mutants-diff").count(),
        2,
        "the plan uploads one diff and the shards download that one"
    );
    assert!(
        text.contains("--in-diff pr.diff --shard \"${SHARD_K}/${SHARD_N}\"")
            && text.contains("--in-place --baseline skip --test-tool nextest --no-shuffle"),
        "a shard runs the flags the issue names"
    );
    assert_eq!(
        text.matches("--cargo-arg=--locked").count(),
        2,
        "the plan and the shards both build with the lockfile as it is"
    );
    Ok(())
}

#[test]
fn the_mutants_lane_runs_hosted_only_on_pull_requests() -> Check {
    let text = workflow("mutants.yml")?;
    assert!(
        text.contains("\non:\n  pull_request:\n\npermissions:") && !text.contains("push:"),
        "the lane runs on pull requests and on nothing else"
    );
    assert_eq!(
        text.matches("runs-on: ubuntu-latest").count(),
        3,
        "plan, shard and verdict all run hosted"
    );
    assert!(
        !text.contains("runs-on: ${{"),
        "no job picks the self-hosted Mac by a runner expression"
    );
    let plan = job(&text, "plan")?;
    assert!(
        plan.contains("if: ${{ !github.event.repository.private }}"),
        "a private repository runs no job of the lane"
    );
    Ok(())
}

#[test]
fn the_mutants_verdict_names_each_missed_mutant_and_the_denominator() -> Check {
    let text = workflow("mutants.yml")?;
    let verdict = job(&text, "verdict")?;
    assert!(
        verdict.contains("\"missed \\(.file):\\(.line): \\(.replacement)"),
        "each missed mutant prints as file:line: replacement"
    );
    assert!(
        verdict.contains("left out of the denominator: unviable $unviable, timeout $timeouts"),
        "the mutants left out of the score are printed beside it"
    );
    assert!(
        verdict.contains("FLOOR: 60")
            && verdict.contains("[ $(( caught * 100 )) -lt $(( FLOOR * viable )) ]"),
        "caught over caught plus missed is held to a 60% floor in whole numbers"
    );
    Ok(())
}
