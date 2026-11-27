---
type: ADR
title: "ADR 0125: Bounded snapshot cache for governance checks"
description: Reuse exact parsed Rust sources and their verdicts inside one governance workspace snapshot.
resource: /docs/adr/0125-governance-parse-cache.md
tags: [larry, adr, governance, temp-paths, suite-time, fastest]
generated: {by: opencode, at: 2026-11-26}
verified: {by: opencode, method: lgwks-box cargo nextest run --workspace --locked on toronto, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#415, larry#413, larry#246, larry#293, ADR 0049, ADR 0109]
---

# ADR 0125: Bounded snapshot cache for governance checks

## Context

`a_hundred_checks_of_the_tree_print_their_percentiles` runs the real temp-path
rule 100 times. Each check read and parsed the same Rust sources through
`lgwks_ast` and walked every tree again, even though the workspace did not
change. The old path measured 173.421 s for the targeted test on Toronto and
timed out at 300.075 s in the full 1,360-test run. The rule file itself stood
at 744 of the 750 allowed code lines, so the cache could not land in it.

## Decision

The rule is split by responsibility, as `uses.rs` and `syntax.rs` already are:
`temp_paths/snapshot.rs` owns the bounded cache, `temp_paths/follow.rs` owns
follow-name resolution, `temp_paths.rs` keeps the walk and the refusals.

Each cache entry keeps the exact source bytes beside its `lgwks_ast::Parsed`
tree and the pure per-file analysis `scan` derives from them (the test-code
verdict, the follow names, the verdict errors). A hit requires the same
root-relative path and byte sequence plus an analysis covering the need; a
changed source is parsed and analysed again. Resolution still reads the live
tree, so only names are cached, never their targets. The budget counts source
bytes and retained analysis, at most 16 MiB, discarded with the workspace.
No process-global or tenant-shared state.

The parser still runs for every source on a cache miss. The existing typed
reader and parse refusals remain. A marker prefilter was rejected because it
would hide malformed-source refusals. An upstream parser cache was not
available in this issue.

## Consequences

The 100-check test runs in seconds instead of minutes, so the suite's long
pole moves to the remaining slow tests. Every check still reads every source,
so a changed file is never judged from bytes it no longer holds. The rule now
lives in three modules: the walk stays in `temp_paths.rs`, the cache in
`snapshot.rs`, resolution in `follow.rs`. A future check that needs a new
per-file fact adds it to the cached analysis with its own need flag.

## Proof

- RED: the existing 100-check test failed at 173.421 s against its 90 s budget.
- GREEN: the benchmark and changed-source invalidation test passed in 35.566 s
  total on Toronto, with 1,359 other tests filtered out.
- The round trip (pass, refuse, pass again, refuse again) proves a changed
  source is never judged from an old tree in either direction.
- The `sim_temp_gate` equality sims prove the split and the cache change no
  report: gated and ungated runs still agree on every seed.
- Full suite on Toronto after the split: the 100-check test 78.1 s to 11.8 s in the suite, 59.4 s to 4.3 s alone (see the SHIP line); the suite as a whole is still over the 90 s budget, so #415 stays open.
