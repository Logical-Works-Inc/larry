---
type: ADR
title: "ADR 0109: The temp-path rule analyses only a source whose text can raise a verdict or a follow"
description: Why the whole-tree check parses every source but runs its verdict and follow passes only on a text that can raise them, and what the measurement says is left.
resource: /docs/adr/0109-the-temp-path-rule-analyses-only-what-it-can-refuse.md
tags: [larry, adr, governance, temp-paths, suite-time, fastest]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, all tests passed, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#415, larry#413, larry#246, larry#293, larry#316, ADR 0049, ADR 0061]
---

# 0109: The temp-path rule analyses only a source whose text can raise a verdict or a follow

Status: accepted, 2026-11-24 (larry#415). Extends ADR 0061 and the larry#316 lanes.

## Context

- `a_hundred_checks_of_the_tree_print_their_percentiles` runs the whole-tree rule 100 times.
  It took 104.2 s of a 117.1 s suite (1329 tests, 8 cores) and was the long pole.
- Measured serially over the 269 sources (3.8 MB, 990,000 nodes) of the three members, idle,
  on toronto: the tree-sitter parse 1.32 to 1.46 s, the checked parse (that plus its
  bounds walk and a copy of the text) 1.49 to 1.75 s, a full traversal 0.24 to 0.30 s, the
  per-function identifier pass 0.53 to 0.58 s, the directory walk and the reads under 25 ms.
  Before the change the whole check was 632 ms p50 on 8 lanes, so about 5 s of CPU.
- `lgwks_ast::try_parse` is the one entry that gives the refusals (`SourceTooLarge`,
  `TimedOut`, `InvalidSyntax`, `AstTooLarge`, `AstTooDeep`); `try_parse_within` only moves
  the deadline and the unchecked `parse` gives none of them. The time is in
  `parser.parse_with_options` (`check.rs:418` of lgwks_ast 6.1.1), the tree-sitter C parser,
  at 2.6 to 2.9 MB/s serial; the owned copy and the bounds walk are under a quarter.

## Decision

- Every source is still parsed with `try_parse`, so a syntax fault, an over-budget tree and a
  deadline refuse a file whatever its text holds.
- The verdict pass runs only when the text holds `process` and one of `temp_dir`, `tmp` or
  `TMPDIR`; the follow pass only when it holds `#[path` or `include`; `is_test_code` is
  computed only when one of them runs.
- Why that is every arm. A verdict needs `first_pid` and `first_temp` in one function.
  `Uses::is_pid` (`uses.rs:83`) answers true for `std::process::id`, a module alias bound by a
  `use` of `std::process`, an imported or renamed `id`, or `id` under a glob of
  `std::process`: each spells `process`. `first_temp` (`syntax.rs:106`) reads `temp_dir` by
  path, alias, import or glob (`is_temp`, `uses.rs:104`), a string whose content starts `/tmp`
  or `/var/tmp` (`starts_temp`, `syntax.rs:19`, raw, byte and plain, and a prefix such as
  `/tmpfile`), `env!("TMPDIR")` and `var("TMPDIR")`: each spells `temp_dir`, `tmp` or `TMPDIR`.
  `follow_names` (`syntax.rs:399`) needs a `#[path` attribute followed by `=` or an `include`
  macro named bare. A name another file re-exports is not resolved by the rule, so the gate
  loses nothing there; `env::var("TMP")` and `/var/folders` are no temp source of the rule.
  The tokens are a superset of what the rule reads, so a text without them raises nothing.
- The gate is proved, not argued: `sim_temp_gate` writes 64 generated trees from 31 fragments
  (every arm, lookalike and syntax fault) and requires the gated report to equal the report of
  every pass over every source, and a table of 39 arms with hand-worked gate answers.

Rejected: skipping the parse of a source without the tokens. `try_parse` refuses invalid
syntax and over-budget trees, so the report would lose refusals. Also rejected: a cache of
trees across the 100 checks, which measures nothing, and lowering `CHECKS`.

## Consequences

- The check keeps its parse floor of about 1.5 s CPU (the 3.8 MB at tree-sitter's speed) and
  drops the roughly 2.8 s of analysis. Idle on toronto, one run alone: p50 632 ms to 349 ms,
  p95 900 ms to 490 ms, p99 1.003 s to 733 ms, the test 66.4 s to 36.6 s.
- The full parallel suite on toronto did not move into the budget: 117.1 s with 1329 tests
  before (the test 104.2 s) and 125.0 s with 1332 tests after (the test 98.3 s), one run each.
  Both runs share the box with 8 cores of other tests, and the after run was the more
  loaded: the `sim_tui` tests, which this change does not touch, took 1.4 times as long
  (49.3 s to 67.9 s). One run each cannot separate that from the change, so the suite is
  still over 90 s and the gain is the idle figure above.
- What is left over budget: the check itself under load, and four `sim_tui` sims of 25 to 68 s
  that the next leaf measures the way this one did.
- The parse is the floor. A cheaper checked entry in lgwks_ast, or a faster parse, is the next
  lever and is filed as a gap on the estate, not worked around here.
