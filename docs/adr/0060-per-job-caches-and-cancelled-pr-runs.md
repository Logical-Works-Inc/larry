---
type: ADR
title: "ADR 0060: per-job caches and cancelled pull request runs"
description: Why every CI job owns its Rust cache key, one governance lane survives, pull request runs cancel, and lint runs beside test.
resource: /docs/adr/0060-per-job-caches-and-cancelled-pr-runs.md
tags: [larry, adr, ci, github-actions, cache]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: hosted CI run on larry#307 before and after, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#307, larry#116, larry#245, /docs/adr/0048-macos-journey-lane.md, GitHub Actions workflow syntax]
---

# 0060: per-job caches and cancelled pull request runs

Status: accepted, 2026-11-17 (larry#307)

## Context

Larry is public, so its runners are free but its wall time is the price of every
push. Measured on main at 91ee874 (ubuntu-latest):

- `rust` took 4m42s: clippy 9s and 3s warm, nextest compile 98s, test run 133s.
- `governance` (ci.yml) took 3m13s and `docs-check` (governance.yml) took 3m52s
  on the same steps, each building `larry` cold from the `sim-hosted` cache with
  `save-if: false`.
- `docs-check` ran twice per PR, because governance.yml took `push` on every
  branch as well as `pull_request`.
- `e2e/journey (ubuntu)` took 3m16s and shared the `sim-hosted` key with the
  `simulation` job, so each job restored the other's cache and neither saved.
- A superseded PR run ran to the end; nothing cancelled it.

## Decision

- Each job names its own `shared-key`: `lint-`, `rerank-lint-`, `test-`,
  `simulation-`, `journey-`, `journey-macos`, `governance-`, each suffixed
  `self-hosted` or `hosted` by visibility. Every cache step saves only when
  `github.ref == 'refs/heads/main'`, so a pull request restores a key and never
  writes what main restores.
- `governance.yml` owns the governance steps. ci.yml's `governance` job is
  removed, and governance.yml's `push` takes `main` only, so the steps run once
  per event instead of twice per PR.
- Every workflow carries `concurrency: {group: <workflow>-<ref>,
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}}`. A second
  push to one PR cancels the run its first push started. A push to main never
  cancels, because that run is the proof of the landed commit.
- The old `rust` job splits into `lint` (rustfmt and the workspace clippy) and
  `test` (the workspace nextest lane), which run in parallel. The
  feature-gated reranker clippy gets its own `rerank-lint` job: on the first
  run its cold build took 132 s of the `lint` job's 317 s, which put `lint` past
  the 5-minute budget.
- The larry#116 runner rule stays on every `runs-on`: the self-hosted Mac while
  private, ubuntu-latest once public, never self-hosted when public.

## Consequences

- One cold build per job key on a new cache, then a warm restore per job; no
  job evicts another's entry.
- A fork PR cannot write the cache main restores, because `save-if` reads
  `github.ref` and a PR ref is never `refs/heads/main`.
- The governance steps run once per PR, down from two builds of `larry`.
- A red `lint` check arrives in seconds beside the test lane instead of after
  it, and its cold build fits the 5-minute budget now that the reranker build
  is its own job.
- `tests/ci_fallback_job.rs` names the `test` job, because the bounded nextest
  install and its `cargo test` fallback moved there with the lane. Its
  assertions are unchanged, and it holds one key per job.

## Rejected

- Keeping one shared key and only adding `cancel-in-progress`: it leaves both
  cold governance builds, keeps the e2e and simulation jobs on one cache, and a
  cancel on main would kill the run that proves the landed commit.
- One `lint` job holding both clippy calls: measured 5m17s cold on the first
  run of larry#307, past the 5-minute budget.
- Cancelling every event with `cancel-in-progress: true`: a fast follow-up push
  to main would cancel the proof of the commit that just landed.
- A `paths` filter to stop the duplicate governance run: the second copy still
  exists, and a filter that skips a required check blocks the merge.
- Splitting lint into its own workflow: a second workflow pays a second
  checkout and cache download for the same tree, and one workflow's concurrency
  group already covers both jobs.
