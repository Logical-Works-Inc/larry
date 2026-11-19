---
type: ADR
title: "ADR 0066: the mutants lane detects renames and refuses a plan past its capacity"
description: Why the lane diffs with -M, and why it sizes up to 96 shards of 3 mutants and fails in the plan job past that.
resource: /docs/adr/0066-mutants-lane-renames-and-capacity.md
tags: [larry, adr, ci, mutants]
generated: {by: claude-code, at: 2026-11-19}
verified: {by: claude-code, method: cargo mutants 27.1.0 --list on the PR #342 diff with and without -M, and the shard outcomes of run 44578190558, 2026-11-19}
status: accepted
stale_after: 2027-02-19
sources: [larry#341, larry#310, /docs/adr/0064-ci-mutants-lane.md, https://git-scm.com/docs/gitdiffcore, https://mutants.rs/in-diff.html]
---

# 0066: the mutants lane detects renames and refuses a plan past its capacity

Status: accepted, 2026-11-19 (larry#341). Amends ADR 0064's diff and shard cap.

## Context

PR #342 moved eight `X/mod.rs` files to `X.rs` (the `mod_module_files` floor lint). The lane
diffed with `--no-renames`, so each moved file showed as all new lines. The plan listed 572
mutants. 358 of them sat on lines nobody changed: `mascot.rs` moved unchanged and planned 64.

ADR 0064 capped the matrix at 16 shards and made the shard timeout the overflow answer. At 36
mutants a shard, 15 of 16 shards hit the 15-minute timeout. The verdict then failed closed with
311 mutants unjudged. Measured in that run: a viable mutant took 60 to 220 s, an unviable one
about 10 s, and every shard that held 4 or more mutants was cut at the timeout.

## Decision

- The plan diffs with `git diff -M`. A move is a rename, so only its changed hunks plan
  mutants. cargo-mutants reads rename diffs, including moves with no hunk
  (`parse_diff_with_only_moves`, cargo-mutants#580). On the same PR #342 tree, `--list` gives
  266 mutants with `-M` and 572 without.
- The matrix holds at most 96 shards (`MUTANTS_MAX_SHARDS`) of at most 3 mutants each
  (`MUTANTS_SHARD_CAPACITY`). 3 times the slowest measured mutant (220 s) is 660 s, inside the
  15-minute timeout.
- A plan past 96 times 3 (288 mutants) fails in the plan job and names the count and the
  capacity. It no longer runs shards that the timeout will cut.

## Consequences

- A PR past 288 mutants on its changed lines is red within minutes, and says to split.
- A PR near the cap queues up to 96 hosted shards. The wall time grows with the org's job
  concurrency. ESTIMATE, not measured: about 25 minutes at 20 concurrent jobs.
- `tests/ci_fallback_job.rs` holds the `-M` diff, the 96 and 3 bounds and the refusal text.

## Rejected

- Exclude moved files from the lane. That also drops the lines a move did change, so the
  gate would judge less code than changed.
- Raise the shard timeout. A longer shard still fails at some size, and it fails later.

Reversal: if a shard of 3 is cut at its timeout, or the queue for 96 shards pushes the lane
past 30 minutes on a PR, this ADR reopens with that run's numbers.
