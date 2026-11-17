---
type: ADR
title: "ADR 0062: the temp-path rule parses on a cursor-fed scoped pool"
description: Why larry governance hands every Rust source to one scoped lane pool and keys each answer to its path, so the worker count never changes a refusal.
resource: /docs/adr/0062-temp-rule-scoped-pool.md
tags: [larry, adr, governance, tests, performance]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: larry-oracle nextest for the rule's controls and larry-oracle clippy on the larry#316 branch, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#316, larry#293, larry#311, /docs/adr/0061-temp-rule-on-lgwks-ast.md, https://doc.rust-lang.org/stable/std/thread/fn.scope.html]
---

# 0062: the temp-path rule parses on a cursor-fed scoped pool

Status: accepted, 2026-11-17 (larry#316)

## Context

The temp-path rule parses every Rust source under the root's `src` and `tests` with
`lgwks_ast`. That parse is the whole cost of the rule: the text reader it replaced measured
p50 25.5 ms per check, and the parsed rule measured p50 279.5 ms on the 4-core Oracle box. The
tree is not evenly priced: `src/tui` and `src/executor` hold the largest sources, and the walk
returns them together.

larry#315 cut the check with contiguous chunks: each scoped thread took a fixed slice of the
path-ordered list and the slices were joined in spawn order. One lane therefore owned most of
the large files, and the check waited on that lane while the others finished early. larry#311
widens the walk to every workspace member, so the files only grow, and the 100-sample cost
control must still fit the pre-commit mutation gate's 300 s budget.

## Decision

- One `AtomicUsize` cursor hands each scoped thread the next path of the path-ordered list.
  Every answer is pushed with its index and stored at that index after the scope joins, so the
  slowest lane waits for one file, not for one slice.
- The lane count is the host's available parallelism, at least one and at most the pending file
  count. A host that cannot report its cores checks every file on one lane, in path order.
- The scoped pool comes from `std::thread::scope`, so no thread outlives the check and the paths
  are borrowed without an `Arc`.
- The control `governance::sim_temp_paths::sim_one_worker_and_many_workers_give_the_same_report`
  checks one tree on 1, 0, 2, 3, 8 and the host's own lane count and asserts byte-identical
  refusals.
- Every bound and message stays: 2 MiB per source, 64 levels of walk, and the typed refusals
  `SourceTooLarge`, `Unreadable`, `SourceNotUtf8` and `Source`.

## Rejected

- Contiguous chunks (larry#315's shape) leave one lane with every large source of its range.
- A fixed pool of four (the `LANES` constant) leaves cores idle on a wider host.
- `rayon`: a new dependency whose early-drop and pool-install semantics add state this loop does
  not need; `std::thread::scope` joins at the scope's end by contract.

## Consequences

- The rule's latency follows the longest single parse instead of the longest slice, and a wider
  host uses its cores without a code change.
- The cursor is one atomic per file; the merged answers are read in path order, so no map or lock
  sits on the parse.
- A lane that panics reports nothing; every path it took keeps an empty slot and is refused by
  name, never passed.
