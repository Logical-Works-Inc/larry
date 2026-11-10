---
type: Checklist
title: Larry ship checklist
description: Every PR left in the plan, in ship order, with the proof that closes it; each shipping PR ticks its own line.
resource: /docs/product/SHIP.md
tags: [larry, plan, checklist, ship]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: gh issue list, each issue's proof clause and git log read 2026-11-09}
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
- [x] #121 `target/` ignored at any depth: PR #122, a65507a. Repository public 2026-11-06: runner deleted,
  interaction limits, fork-PR approval, private reporting

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
- [x] #123 wire `larry_submit_task` to the executor. Proof: `tests/mcp_stdio_journey.rs` runs one task
  against the mock model and verifies its receipt: PR #140, 8 seeds replayed to one trace and diff, every
  fault refused unsealed, 607 tests in 11.4 s; submitted task p50 108.1 ms, p99 148.3 ms (16 runs, debug, mock).
- [x] #87 matched task set, Claude alone against Claude plus Flash. Proof: cost per accepted task,
  acceptance rate, p50/p95/p99 and peak RSS printed for both arms: PR #141, `larry eval` ships it; 5 binary
  journeys and 5 unit sims, 617 tests in 12.1 s; matched-v1 on the mock p50 97.8 ms, p99 114.9 ms per task,
  peak RSS 12.1 MiB (debug); scored runs are #95.
- [x] #142 README lists the shipped doors and the v0.1.0 install line (part of #124): docs-check green.
- [x] #124 v0.1.0 release with #87's numbers: tag on c98062a, prerelease notes carry #87's card; clean `cargo
  install --tag v0.1.0 --locked` in 28.5 s, `larry mcp` answers `server/discover` (exit 0, peak RSS 7.7 MiB).

## 2. Track 0: estate alignment (parallel with section 1)

- [x] #131 rename Rocco to Larry (Director 2026-11-06). Proof: the Gates green under the new names; 582 tests in
  10.3 s under `larry-demo`, `larry-contracts` and `larry-runtime` (ADR 0005).
- [x] #53 the nine Python tools in Rust: steps 1 #145, 2 #147, 3 #150, 4 #152, 5 #154 (nextest), 6 #155 (split). Proof:
  no tracked `.py`, no Python step in CI or `cargo local-ci`, every `.md` at most 120 lines (`larry governance`).
- [x] #103 `issue-link` fails a PR closing no issue. Proof (#149): run 42364512157 red without `Closes`, 42364569033 green.
- [x] #133 dependency register on lgwks-deps schema 3: PR #135, `lgwks-deps check .` prints OK (was REFUSED).
- [x] #170 hardening: clipboard-win off yanked 5.4.1, 34 locked to newest, appcui_proc_macro held (crates#841): PR #171.
- [x] #178 every document current: the Rocco rename finished, counts, pins, falsifier state, ruleset re-read.
- [ ] #177 gap: pin the ci.yml `rust` job's three actions to commit SHAs (Director 2026-11-09: not yet).

## 3. Track 1: the runtime on lgwks_bot (#68, one row per PR)

- [x] #74 clock: PR #159, determinism green both sides; replay p50 58.1/62.5 us, p99 154.3/108.9 us (256 seeds).
- [x] #75 journal on RecordJournal, group-committed, a dead task recovered unrerun (ADR 0023): PR #172, 775 tests;
  `larry exec` p50 70.0/78.1 ms, p99 96.3/98.3 ms (32 runs, debug), peak RSS 12.5/13.2 MiB.
- [x] #76 fence on broker (ADR 0024): PR #173, 780 tests; `larry exec` p50 75.1/75.4 ms, p99 79.8/80.9 ms, 13.2/13.1 MiB.
- [x] #77 gate on GrantSet and cap::Auth: PR #163, 729 tests; replay p50 49.9/49.7 us, p99 94.5/86.8 us (ADR 0018).
- [x] #78 retry on lgwks_bot::retry, ahead of #75 to #77 (Director 2026-11-08): PR #160, 725 tests; replay
  p50 62.8/49.9 us, p99 125.4/94.5 us (256 seeds); a retry past the authority lease is refused (ADR 0017).
- [x] #79 projection on RecordProjection (8.2.0): PR pending, 798 tests; replay suites 41 pass, cold build 17/31 s.
- [ ] #80 tick and consumer on app scope and tenant_store. Proof: `sim_lifecycle`, `sim_consumer`, `sim_cancel`.
- [x] #68 step 4 (#164): every executor task passes the runtime gate, `LARRY_POLICY` (ADR 0019): PR #165, 734 tests;
  `larry exec` p50 63.6/62.6 ms, p99 111.7/66.1 ms (64 runs, debug), peak RSS 10.9/11.8 MiB.
- [x] #101 E47 protocol negotiation before ingress, the executor its caller (ADR 0020): PR #166, 742 tests.
- [x] #100 E28 durable admission before the acknowledgement, corrections at a safe boundary (ADR 0025): PR #174, 785 tests.
- [x] #99 E71 a restart recharges no offline authority, old settings revive nothing (ADR 0026): PR #175, 790 tests.
- [x] #98 E30 durable scoped questions, expiry never consent (ADR 0027), PR #25 ported: PR #176, 797 tests.

## 4. Track 2: the operator agent, the Copilot answer (#70)

- [x] #88 `larry serve` on lgwks_bot::app, not Loco (ADR 0021): PR #167, 756 tests; cold 45.97 s, 225 crates; probe p99 0.221 ms.
- [ ] #89 tenant keys and human sign-in. Proof: two identities on one run id at once. Service keys: #168, PR #169.
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

- [x] #61 harness as a versioned, content-hashed journal value (ADR 0028, 0029): PR pending, 21 tests; 38217 of 65524 record bytes.
- [x] #62 recursion as journalled steps (production executor owner and RSS wait on part 3): PR pending, 11 tests; 4445 of 65524 record bytes.
- [x] #63 grader-blind learning (type boundary and scan; egress waits on Nova): PR pending, 10 tests.
- [ ] #64 blind seeded selection and a lesson library per tenant.
- [ ] #65 matched benchmark against Harness² on JobBench and LAB.
- [ ] #66 learning the person can see, budget, undo and stop.
- Each issue's Acceptance section is its proof.

## Holds and open design

- Memory (C03) waits for Logical-DB, no substitute store; Nova, Moo and Secure-Authority are test doubles until Track 2.
- #4 shared-context routing and local ML: design only. #56, #57 (logical-cloud) wait for that repository.
