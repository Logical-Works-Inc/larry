---
type: ADR
title: "ADR 0050: governance tests check a fixture tree"
description: Why the governance unit controls and the governance latency journey check a tree the test writes, not the live checkout.
resource: /docs/adr/0050-governance-fixture-tree.md
tags: [larry, adr, governance, tests, flakiness]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: larry-oracle all on the larry#270 branch, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#270, larry#224, /docs/adr/0010-governance-checker-in-rust.md]
---

# 0050: governance tests check a fixture tree

Status: accepted, 2026-11-16 (larry#270, larry#224)

## Context

The governance unit controls ran the checker over the live checkout. A document being edited failed
them for no code reason, and a mutation copy of the tree made the checker read other bytes than the
mutant. The latency journey measured `larry governance` over the live checkout while the whole suite
ran in parallel: p99 was 6.56 s under that load and 0.96 s alone (larry#224), so another test decided
a wall-clock budget.

- The live-tree rule is already enforced where a person drives it: CI's `governance` job, the
  pre-commit `larry governance`, and `larry tracker --check`.
- A fixture copied from the checkout keeps the checkout as an input, so it repeats the flake.
- A fixture read from git history needs git and the history in a unit test.

## Decision

- `governance::fixture` is a cfg(test) module that writes every file `larry governance` reads: the
  seven documents, the specification's four parts, the tracker data and template. The receipts are
  copied from the product's `tools/receipts`, so the checker's pinned identity keeps one home.
- The tests write that tree into a scratch guard that removes a stale directory before it makes one
  and removes the tree when it drops.
- The page is written by the product's one renderer (`larry tracker` or `tracker::write`).
- The unit controls and the governance latency journey run on the fixture. The live tree stays with
  CI's `governance` job and pre-commit.
- The latency test runs alone in the `governance-latency` nextest group (`max-threads = 1`) and
  keeps its p50, p95, p99 and peak RSS print and both assertions.

## Consequences

- A half-edited document no longer fails a unit test or the mutation gate. The live-tree rule is
  enforced by the CI job and pre-commit, which the fixture does not replace.
- Each checker rule still has a control that breaks it and asserts the refusal: the 120-line law, a
  missing document, a symlinked document, and stale tracker data.
- The fixture text must follow the checker's rules. A rule change updates the fixture in the same
  change, or the fixture test fails first.
- Measured on the Oracle box, 32 runs on the fixture: p50 14.5 ms, p95 15.5 ms, p99 18.8 ms, peak
  RSS 10,536 KiB. The 5 s p99 and 256 MiB budgets hold with room.
- The fixture module is also included by path into `tests/governance_journey.rs`, as `sim_lanes` is
  into its journeys, so one implementation serves both crates.
