---
type: ADR
title: "ADR 0064: a sharded mutation lane on each pull request's changed lines"
description: Why Larry CI runs cargo-mutants on the lines a pull request changes, in a plan, shards and one verdict with a 60% floor.
resource: /docs/adr/0064-ci-mutants-lane.md
tags: [larry, adr, ci, github-actions, mutation-testing]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: cargo-mutants 27.1.0 run locally for list, json and exit codes; the lane's own runs on larry#310, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#310, larry#307, larry#116, /docs/adr/0060-per-job-caches-and-cancelled-pr-runs.md, https://mutants.rs/shards.html, https://mutants.rs/in-diff.html, https://mutants.rs/exit-codes.html]
---

# 0064: a sharded mutation lane on each pull request's changed lines

Status: accepted, 2026-11-17 (larry#310)

## Context

Larry is public, so hosted runners are free, and the Director (2026-11-17) ordered heavy testing
into CI to save cost and speed up. Today only the pre-commit gate on the Mac mutates the changed
lines, and a PR shows no mutation result at all.

`cargo mutants --in-diff` tests only the mutants that overlap a diff. Each mutant costs one
incremental build and one run of its package's tests, so wall time grows with the mutant count.
Measured with cargo-mutants 27.1.0 on this tree: a 4,208-line Rust diff lists 308 mutants in 0.2 s
with no build (`--list --json`), and an empty or Markdown-only diff prints nothing and exits 0.

## Decision

`.github/workflows/mutants.yml`, on `pull_request` only, hosted only (`ubuntu-latest`, never the
self-hosted Mac, larry#116), with the one-run-per-ref cancel of ADR 0060.

- `plan`: checks out with full history, writes `git diff origin/<base>...HEAD -- '*.rs'` to
  `pr.diff`, lists the mutants with `cargo mutants --workspace --in-diff pr.diff --list --json`
  (no build), and emits `count`, `shards` and a matrix. `shards = min(16, ceil(count / K))`, so zero
  mutants is zero shards. It uploads `pr.diff` once: every shard reads that one diff, because shards
  that disagree on the mutant list or on `n` give meaningless results (mutants.rs/shards).
- `shard` (matrix, `fail-fast: false`, 15 minute timeout): restores the cache, then runs
  `cargo mutants --workspace --in-diff pr.diff --shard k/n --in-place --baseline skip --test-tool
  nextest --no-shuffle --timeout 300 --cargo-arg=--locked`. Shards are numbered from 0. Exit codes
  0 (all caught), 2 (a miss) and 3 (a timeout) are data; any other code fails the shard. It uploads
  `mutants.out/outcomes.json` as `mutants-outcomes-<k>`.
- `verdict` (`needs: [plan, shard]`, `if: always()`): merges every `outcomes.json` with `jq` and
  prints caught, missed, unviable and timeout. The score is caught over caught plus missed;
  unviable mutants (they did not build) and timeouts are left out of the denominator and printed.
  Under 60% fails. Each missed mutant prints as `missed file:line: replacement`.
- The verdict fails closed. It is red when the plan did not finish, a shard did not succeed, the
  outcomes files are fewer than the shards, the merged mutants differ from the plan's count, or an
  outcome is none of the four it knows. Zero shards passes with `no mutants`.
- Tools come from `taiki-e/install-action` at a full commit SHA (v2.90.5), `cargo-mutants@27.1.0`
  and `nextest`; every other action is pinned the way ci.yml pins it. Each install is bounded at
  3 minutes and may not fail, because the lane has no fallback without its tools.
- Cache: `Swatinem/rust-cache` restores `shared-key: test-hosted` with `save-if` on main only.
  ADR 0060 gives each job its own key, but this workflow never runs on main, so a key of its own
  would never be written. It reads the test lane's key and saves nothing; a PR still never writes
  what main restores.
- K, mutants per shard, is 2. MEASURED on larry#310: on a cold cache (the shard's cache paths
  missed the test lane's key) one shard of 3 mutants took 7m18s and its pair hit a 10 minute
  timeout; on the restored cache 4 shards of 2 took 4m03s to 5m18s, and the lane 5m43s
  (runs 44197696481 red at 50% naming 4 missed lines, 44198842255 green at 100%).

## Consequences

- A worker reads one check, `verdict`, and its log names each missed mutant by file and line.
  Shards stay green on a miss, so a red shard means a shard that did not run.
- `--baseline skip` trusts ci.yml's `test` lane as the baseline. A tree whose tests already fail
  makes every mutant look caught, so a red `test` check beside a green `verdict` is not a pass.
- `--workspace` mutates every crate in the diff, and each mutant runs only its own package's tests
  (cargo-mutants' default), not the workspace suite. A mutant in `larry-contracts` that only a root
  journey catches counts as missed. That is deliberate: the floor measures a crate's own tests.
- Above 16 shards' worth of mutants a shard holds more than K mutants and may reach its 15 minute
  timeout, which turns the verdict red. A large PR is expected to split, or to raise the cap with a
  measurement.
- governance.yml puts a body-edit run in its own concurrency group and runs only issue-link on it.
  Before, an edit on an unchanged commit cancelled the running docs-check (PRs #317, #318, #320),
  and a CANCELLED check is not green. Test: `a_body_edit_never_cancels_the_code_checks`.
- A PR of only test code, or only deleted lines, lists no mutants and passes with `no mutants`.
  cargo-mutants matches the diff against code under test, not test code (mutants.rs/in-diff).

## Rejected

- One unsharded job: no matrix and no merge, but its wall time is the sum over every mutant. At the
  measured 308 mutants for one large diff it cannot end in 5 minutes. It would be right again
  if a measured full-diff run on the warm cache ended in 5 minutes for the median PR, because
  then the plan and verdict jobs would only add startup cost.
- Failing each shard on a miss (cargo-mutants' own exit 2): no floor, so one miss fails a PR; up to
  16 red checks to read; and a shard that never ran looks like one that missed.
- A cache key of its own with `save-if` true on pull requests: a fork PR could then write what a
  later run restores, which ADR 0060 removed.
- Running the baseline on every shard: it repeats the test lane's work 16 times for one fact.
