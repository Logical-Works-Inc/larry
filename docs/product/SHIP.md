---
type: Checklist
title: Larry ship checklist
description: Every PR left in the plan, in ship order, with the proof that closes it; each shipping PR ticks its own line.
resource: /docs/product/SHIP.md
tags: [larry, plan, checklist, ship]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: gh issue list and each issue's proof clause read 2026-11-06}
status: live
stale_after: 2026-12-06
sources: [/docs/product/README.md, larry#114, larry#125, /docs/adr/0002-ship-checklist.md]
---

# Ship checklist

## How a session ships the next item

1. Take the first unchecked line of the first section that has one. Lines in a section ship in order unless
   marked parallel. A newer Director order beats this order; record it here in the same PR.
2. Read the issue, then work it as AGENTS.md says: research, plan, build in the run path, the Gates, a PR
   whose body says `Closes #N`, merge on green.
3. In that same PR, tick the line and append `PR #M, <the measured number>`; the squash SHA appears only
   at merge, so `git log` holds it. A line is ticked only when its proof ran and printed.
4. New work enters as an issue and a line here in the PR that files it. Never delete a line: tick it, or
   strike it with the reason and the Director's order.

## Shipped

- [x] #112 newest estate pins, #116 runner by visibility: PR #113, ea3645d
- [x] #117 public-alpha warning, SECURITY.md, CONTRIBUTING.md, licence clause: PR #118, 4ff17c5
- [x] #119 cancellation waits for reconciliation, runtime split: PR #120, 1f46189, 544 tests in 2.03 s
- [x] #121 `target/` ignored at any depth: PR #122, a65507a
- [x] Repository public 2026-11-06: runner deleted, interaction limits, fork-PR approval, private reporting

## 1. Weekend ship: Larry v0.1, the executor (#114, umbrella #71)

- [x] First merge after the flip runs CI on `ubuntu-latest` and is green: PR #126, rust, simulation,
  e2e/journey, governance and docs-check all passing on hosted runners.
- [x] #83 model client to 9router on `lgwks_std::http`, `json`, `retry`. Proof: drives the mock endpoint;
  typed errors on timeout, 429 and malformed body: PR #127, `larry model` ships it; 16 client cases over 64
  seeds and 8 binary journeys, 568 tests in 10.0 s; `larry model` p50 6.4 ms, p99 12.4 ms (32 runs, mock).
- [x] #85 executor workspace and process control. Proof: kill -9 of the executor leaves no orphan process
  group; the origin tree is byte-identical after a hostile command: PR #130, `larry exec` ships it; 7 binary
  journeys and 3 unit sims, 582 tests in 10.4 s; `larry exec` p50 70.8 ms, p99 74.5 ms (32 runs, debug).
- [x] #86 signed receipts over the journal (ed25519). Proof: a tampered receipt is refused; a denied run
  never reports success: PR #134, `larry verify` and `larry key` ship it; 6 binary journeys and 5 unit sims,
  598 tests in 11.0 s; verify p50 4.6 ms, p99 5.0 ms against a rerun at p50 64.7 ms (32 runs, debug).
- [ ] #123 wire `larry_submit_task` to the executor. Proof: `tests/mcp_stdio_journey.rs` runs one task
  against the mock model and verifies its receipt.
- [ ] #87 matched task set, Claude alone against Claude plus Flash. Proof: cost per accepted task,
  acceptance rate, p50/p95/p99 and peak RSS printed for both arms.
- [ ] #124 v0.1.0 release with #87's numbers. Proof: `gh release view v0.1.0`; a clean
  `cargo install --git ... --tag v0.1.0 --locked` serves `larry mcp`.

## 2. Track 0: estate alignment (parallel with section 1)

- [x] #131 rename Rocco to Larry (Director 2026-11-06). Proof: the Gates green under the new names; 582 tests in
  10.3 s under `larry-demo`, `larry-contracts` and `larry-runtime` (ADR 0005).

- [ ] #53 port the nine Python tools to Rust (governance checker and tracker first). Proof: CI and
  `.wwfd/local-ci.sh` run no Python; the over-size documents split.
- [ ] #103 CI fails a PR with no closing issue. Proof: passes on its own PR, fails on one without `Closes`.
- [ ] #101 E47 (PR #20), #100 E28 (PR #21), #99 E71 (PR #22), #98 E30 (PR #25): rebase each onto main and
  merge on green, or close with the reason. Proof: the PR's named test green on main.

## 3. Track 1: the runtime on lgwks_bot (#68, one row per PR)

- [ ] #74 clock. Proof: `tests/determinism.rs` green both sides; replay p50/p95/p99 and RSS both sides.
- [ ] #75 journal on FileJournal, durable across kill -9. Proof: `sim_crash_restore`, `sim_replay` and a
  kill -9 that re-executes the binary mid-append.
- [ ] #76 fence on broker and dist_lease. Proof: `tests/fence.rs`, `sim_fence`, `sim_epochs`.
- [ ] #77 gate on GrantSet and cap::Auth. Proof: `e63_forged_claims`, `sim_denials`.
- [ ] #78 effect lifecycle on lgwks_bot::effect and retry. Proof: `lifecycle`, `sim_reconcile`,
  `sim_duplicates`.
- [ ] #79 projection on the ECS world. Proof: `sim_replay`, `sim_multitenant`.
- [ ] #80 tick and consumer on app scope and tenant_store. Proof: `sim_lifecycle`, `sim_consumer`,
  `sim_cancel`.

## 4. Track 2: the operator agent, the Copilot answer (#70)

- [ ] #88 Loco host behind a `serve` feature. Proof: `/healthz` on production config; cold build under 60 s.
- [ ] #89 tenant keys and human sign-in. Proof: two identities on one run id at once.
- [ ] #90 runtime API over the durable journal. Proof: two tenants under kill -9 of `larry serve`.
- [ ] #91 out-of-band approval mail. Proof: an impersonated approval is refused with a typed error.
- [ ] #115 phone surface over the runtime API (after #90). Proof: a replayed approval is refused.
- [ ] #92 load test and Copilot Studio journey. Proof: 10,000 req/s on 1 to 2 vCPU with p50/p95/p99,
  error rate and peak RSS; the approval count a user sees against Copilot Studio.
- Acceptance cases for this track, not PRs: #27 to #46 (errands and threats), #54, #55, #58, #59.

## 5. Benchmarks (#72)

- [ ] #93 Terminal-Bench 2.0 Harbor adapter. Proof: the shipped binary runs one task on the mock endpoint.
- [ ] #94 Harness-Bench runner, model held constant across five harnesses. Proof: pass rate, cost per
  accepted task, p50/p95/p99, peak RSS per harness.
- [ ] #95 scored runs and the published comparison (spends model money).

## 6. Track 4: learning, against Harness² (#60)

- [ ] #61 harness as a versioned, content-hashed journal value.
- [ ] #62 propose, probe and compose as journalled steps.
- [ ] #63 grader-blind learning.
- [ ] #64 blind seeded selection and a lesson library per tenant.
- [ ] #65 matched benchmark against Harness² on JobBench and LAB.
- [ ] #66 learning the person can see, budget, undo and stop.
- Each issue's Acceptance section is its proof.

## Holds and open design

- Memory (cell C03) waits for Logical-DB; no substitute store. Real owners (Nova, Moo, Secure-Authority)
  arrive through #85 and Track 2; until then every owner is a test double.
- #4 shared-context routing and local ML: design only, no PR yet.
- #56 and #57, the logical-cloud strand, are held here until that repository exists.
