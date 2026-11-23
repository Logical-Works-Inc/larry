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

/// The compiler every box and every job builds larry with (Director 2026-11-23: "rust 1.100
/// upgrade mandated for all repos", larry#406).
const RUST: &str = "1.100.0";

/// Requirement: one baseline compiler (NIST SP 800-53 rev. 5 CM-2). `rust-toolchain.toml`
/// pins 1.100.0, the workspace `rust-version` is 1.100, and every toolchain step of the four
/// workflows installs 1.100.0, so the Mac, Oracle (default 1.98.0), the x86_64 box (only
/// 1.100.0) and the hosted runners compile one way. Catches a lane left on `stable`, a step
/// that names another release, and a manifest or toolchain file that drifts.
#[test]
fn the_toolchain_is_the_mandated_rust() -> Check {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toolchain = std::fs::read_to_string(root.join("rust-toolchain.toml"))?;
    assert!(
        toolchain
            .lines()
            .any(|line| line.trim() == format!("channel = \"{RUST}\"")),
        "rust-toolchain.toml pins {RUST}: {toolchain}"
    );
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))?;
    assert!(
        manifest
            .lines()
            .any(|line| line.trim() == "rust-version = \"1.100\""),
        "the workspace rust-version is 1.100"
    );
    for name in ["ci.yml", "governance.yml", "mutants.yml", "sim.yml"] {
        let text = workflow(name)?;
        let steps = text.matches("uses: dtolnay/rust-toolchain@").count();
        let pinned = text.matches(&format!("toolchain: {RUST}\n")).count();
        assert!(steps > 0, "{name} installs a toolchain");
        assert_eq!(
            pinned, steps,
            "{name}: every toolchain step installs {RUST}"
        );
        assert_eq!(
            text.matches("toolchain: ").count(),
            steps,
            "{name}: no other toolchain is named"
        );
    }
    Ok(())
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
        // governance.yml alone listens on `edited`, and puts those runs in a group of their own.
        let suffix = if name == "governance.yml" {
            "${{ github.event.action == 'edited' && '-body' || '' }}"
        } else {
            ""
        };
        assert!(
            text.contains(&format!(
                "concurrency:\n  group: ${{{{ github.workflow }}}}-${{{{ github.ref }}}}{suffix}\n  cancel-in-progress: ${{{{ github.event_name == 'pull_request' }}}}\n"
            )),
            "{name}: one concurrency group per workflow and ref, cancelled for pull requests only \
             (a main run is the proof of the landed commit)"
        );
    }
    Ok(())
}

/// Source: PRs #317, #318 and #320 (2026-11-17), where a body edit on an unchanged commit
/// cancelled the running docs-check and left it CANCELLED; GitHub Actions concurrency docs (runs
/// in one group cancel each other). Requirement: a body edit re-checks only the issue link.
#[test]
fn a_body_edit_never_cancels_the_code_checks() -> Check {
    let text = workflow("governance.yml")?;
    assert!(
        text.contains("types: [opened, synchronize, reopened, edited]"),
        "a body fix still re-runs the issue link"
    );
    assert!(
        text.contains("group: ${{ github.workflow }}-${{ github.ref }}${{ github.event.action == 'edited' && '-body' || '' }}\n"),
        "an edited run joins its own group, so it cannot cancel a code run"
    );
    let docs = job(&text, "docs-check")?;
    assert!(
        docs.contains("if: github.event.action != 'edited'"),
        "a body edit changes no file, so docs-check does not run on it"
    );
    assert_eq!(
        text.matches("action != 'edited'").count(),
        1,
        "only docs-check skips an edit; issue-link runs on every pull request event"
    );
    for name in ["ci.yml", "sim.yml", "mutants.yml"] {
        assert!(
            !workflow(name)?.contains("edited"),
            "{name} never starts on a body edit"
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
    // The verdict on zero mutants is `larry mutants verdict`'s (larry#387):
    // tests/mutants_verdict_journey.rs holds it.
    Ok(())
}

#[test]
fn the_mutants_lane_caps_its_shards_and_bounds_each_one() -> Check {
    let text = workflow("mutants.yml")?;
    assert!(
        text.contains("MUTANTS_MAX_SHARDS: 96") && text.contains("MUTANTS_SHARD_CAPACITY: 3"),
        "the matrix is capped at 96 shards of at most 3 mutants each"
    );
    let plan = job(&text, "plan")?;
    assert!(
        plan.contains("if [ \"$count\" -gt \"$capacity\" ]; then")
            && plan.contains("exceed the lane's capacity of $capacity")
            && plan.contains("exit 1"),
        "a plan past the capacity fails in the plan job by name, not at a shard's timeout"
    );
    // Run 44638202506: 266 mutants in 96 shards of ceil(266 / 96) left 7 shards empty.
    assert!(
        plan.contains("per=$(( (count + MUTANTS_MAX_SHARDS - 1) / MUTANTS_MAX_SHARDS ))")
            && plan.contains("shards=$(( (count + per - 1) / per ))"),
        "the shard count follows the run cargo-mutants hands each shard, so no shard is empty"
    );
    // PR #342: eight mod.rs moves planned 358 of its 572 mutants on lines no one changed.
    assert!(
        plan.contains("git diff --no-color --no-ext-diff -M ") && !plan.contains("--no-renames"),
        "the plan diffs with rename detection, so a moved file plans only its changed lines"
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
    assert!(
        shard.contains("0 | 2 | 3) echo") && shard.contains("exit \"$rc\""),
        "a shard treats exit 0, 2 and 3 as outcomes and every other code as a shard that did not run"
    );
    assert!(
        plan.contains("MUTANTS_FLOOR: 60") && plan.contains("MUTANTS_PER_SHARD: 2"),
        "the lane publishes the floor and the shard size the verdict reads from main"
    );
    Ok(())
}

#[test]
fn the_mutants_lane_never_writes_the_cache_main_restores() -> Check {
    let text = workflow("mutants.yml")?;
    assert_eq!(
        text.matches("uses: Swatinem/rust-cache@").count(),
        2,
        "the shard restores a cache and the aarch64 warm job saves the one it restores"
    );
    assert_eq!(
        text.matches("save-if: ${{ github.ref == 'refs/heads/main' }}")
            .count(),
        2,
        "both cache steps save only from main, so a pull request never writes what main restores"
    );
    assert!(
        text.contains("'test-hosted'") && text.contains("shared-key: mutants-aarch64"),
        "x86_64 restores the test lane's key; aarch64 restores the key only main writes (ADR 0095)"
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
fn the_mutants_lane_runs_hosted_and_judges_only_pull_requests() -> Check {
    let text = workflow("mutants.yml")?;
    assert!(
        text.contains("\non:\n  pull_request:\n  push:\n    branches: [main]\n\npermissions:"),
        "the lane judges pull requests, and a push to main only warms the aarch64 cache"
    );
    assert_eq!(
        text.matches("runs-on: ubuntu-24.04\n").count(),
        1,
        "the plan runs on the pinned hosted x86_64 image; the verdict is mutants-verdict.yml's"
    );
    assert_eq!(
        text.matches("runs-on: ubuntu-24.04-arm\n").count(),
        1,
        "the warm job runs on the hosted ARM64 image"
    );
    assert_eq!(
        text.matches("runs-on: ${{").count(),
        1,
        "only the shard picks its runner, from the matrix's two hosted images"
    );
    assert!(
        !text.contains("ubuntu-latest") && !text.contains("\"self-hosted\",\"macOS\""),
        "every job names its image, and none runs on the self-hosted Mac"
    );
    let plan = job(&text, "plan")?;
    assert!(
        plan.contains(
            "if: ${{ !github.event.repository.private && github.event_name == 'pull_request' }}"
        ),
        "a private repository runs no job of the lane, and a push to main plans nothing"
    );
    Ok(())
}

/// Source: the Director's order of 2026-11-23 ("add x86 as well in the testing portfolio"),
/// GitHub's hosted runner images (`ubuntu-24.04` is x86_64, `ubuntu-24.04-arm` is ARM64) and
/// cargo-mutants 27.1.0's release assets (x86_64 Linux only). Requirement: every mutant is judged
/// on both architectures, each shard proves which one it ran on, each architecture meets the
/// floor alone, and a mutant the two judge apart fails the verdict (larry#404, ADR 0095).
#[test]
fn the_mutants_lane_judges_every_mutant_on_both_linux_architectures() -> Check {
    let text = workflow("mutants.yml")?;
    let plan = job(&text, "plan")?;
    for leg in [
        r#"{arch: "x86_64", runner: "ubuntu-24.04", host: "x86_64-unknown-linux-gnu"}"#,
        r#"{arch: "aarch64", runner: "ubuntu-24.04-arm", host: "aarch64-unknown-linux-gnu"}"#,
    ] {
        assert!(plan.contains(leg), "the shard matrix holds the leg {leg}");
    }
    assert!(
        plan.contains("| . as $a | range(0; $n) | $a + {k: ., n: $n}]}"),
        "every shard of the plan runs on each architecture"
    );
    let shard = job(&text, "shard")?;
    assert!(
        shard.contains("runs-on: ${{ matrix.runner }}")
            && shard.contains("name: shard ${{ matrix.arch }} ${{ matrix.k }}/${{ matrix.n }}"),
        "a shard runs on its leg's hosted image and names its architecture"
    );
    assert!(
        shard.contains("host=\"$(rustc -vV | sed -n 's/^host: //p')\"")
            && shard.contains("if [ \"$host\" != \"$WANT\" ]; then")
            && shard.contains("it judges the wrong architecture (fail closed)"),
        "a runner that served the other architecture fails its shard"
    );
    assert!(
        shard.contains("name: mutants-outcomes-${{ matrix.arch }}-${{ matrix.k }}"),
        "each architecture uploads its own outcomes"
    );
    assert!(
        shard.contains(
            "shared-key: ${{ matrix.arch == 'x86_64' && 'test-hosted' || 'mutants-aarch64' }}"
        ),
        "x86_64 restores the test lane's cache and aarch64 the warm job's"
    );
    assert!(
        shard.contains("if: ${{ matrix.arch == 'aarch64' }}\n        timeout-minutes: 8\n        run: cargo install cargo-mutants --locked --version 27.1.0"),
        "cargo-mutants ships no aarch64 Linux binary, so that leg builds the pinned release, bounded"
    );
    let warm = job(&text, "warm-aarch64")?;
    assert!(
        warm.contains("if: ${{ !github.event.repository.private && github.event_name == 'push' }}")
            && warm.contains("runs-on: ubuntu-24.04-arm")
            && warm.contains("shared-key: mutants-aarch64")
            && warm.contains("cargo install cargo-mutants --locked --version 27.1.0")
            && warm.contains("cargo nextest run --workspace --locked --no-run"),
        "a push to main warms the aarch64 shards' cache, cargo-mutants included, and judges nothing"
    );
    // Each architecture held to the floor alone, and a mutant judged apart, is
    // `larry mutants verdict`'s rule now (larry#387): tests/mutants_verdict_journey.rs.
    assert!(
        !text.contains("runs-on: [self-hosted") && !text.contains("lgwks-linux"),
        "the x64 org runners never serve this public repository (larry#116)"
    );
    Ok(())
}

#[test]
fn every_hosted_lane_pins_the_nextest_it_installs() -> Check {
    for name in ["ci.yml", "sim.yml", "mutants.yml"] {
        let text = workflow(name)?;
        let mut installs = 0;
        for line in text.lines().map(str::trim) {
            let Some(tools) = line.strip_prefix("tool: ") else {
                continue;
            };
            for tool in tools.split(',') {
                let tool = tool.trim();
                if tool.split('@').next() == Some("nextest") {
                    installs += 1;
                    assert!(
                        tool.starts_with("nextest@"),
                        "{name}: nextest is installed at a pinned version: {tool}"
                    );
                }
            }
        }
        assert!(
            installs > 0,
            "{name}: the lane installs nextest at least once"
        );
    }
    for name in ["ci.yml", "sim.yml"] {
        assert!(
            workflow(name)?.contains("nextest@0.9.152"),
            "{name}: the pinned nextest is the measured one"
        );
    }
    Ok(())
}

#[test]
fn the_advisory_job_checks_the_lockfile_on_every_pull_request() -> Check {
    let text = workflow("ci.yml")?;
    assert!(
        text.contains("on:\n  push:\n    branches: [main]\n  pull_request:\n"),
        "ci runs on every pull request"
    );
    let job = job(&text, "advisories")?;
    assert!(
        job.contains("cargo install cargo-deny --locked --version 0.22.0")
            && job.contains("cargo deny check advisories"),
        "the job installs the pinned checker and scans every advisory (larry#339)"
    );
    let deny = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("deny.toml"))?;
    assert!(
        deny.contains("[advisories]"),
        "deny.toml holds the advisories policy"
    );
    assert!(
        deny.contains("RUSTSEC-"),
        "every accepted advisory is ignored by its id with a reason"
    );
    Ok(())
}
