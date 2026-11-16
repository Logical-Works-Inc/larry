---
type: ADR
title: "ADR 0051: one temp-path guard per test side"
description: Why test code names a temp directory only through one guard, and why larry governance refuses a process id in a temp path.
resource: /docs/adr/0051-temp-path-guard.md
tags: [larry, adr, tests, flakiness, governance]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: larry-oracle all on the larry#267 branch, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#267, larry#243, POSIX mkstemp(3), CWE-377, /docs/adr/0010-governance-checker-in-rust.md]
---

# 0051: one temp-path guard per test side

Status: accepted, 2026-11-16 (larry#267, larry#243)

## Context

Fixtures named `<label>-<pid>` under the system temp dir ran in whatever tree the last process with
that id left. Six fixtures were killed before their cleanup, the system reused the id, and the next
run failed on a dead run's stale file: larry#243 measured 3,566 leftover directories in `$TMPDIR`.

- A name carrying the process id is not unique: the id is reused, and a container can pin it.
- A fixture that names a directory without owning it leaves the cleanup optional, so a killed test
  leaks and a later test inherits.
- Each site cleaned up its own way, so a fix per site leaves the pattern free to return.

## Decision

- One guard per side of the workspace split: `executor::sim_tree::scratch` (widened to `pub(crate)`)
  for the crate's tests, and `TempDir` in `tests/support/temp_dir.rs` for the integration tests,
  re-exported by `origin`, `serve_cli` and `route_cli` and included by path elsewhere.
- Each guard removes whatever sits at its path before it makes the directory, and removes the
  directory when it drops. The process id may appear in a name only inside those two files.
- `governance::temp_paths` reads every `.rs` file under `src/` and `tests/`, treats a file as test
  code when it lives under `tests/`, is a `sim_*` or `*_tests` module, is a `tests` module or holds
  `#[cfg(test)]`, and refuses `std::process::id()` in code whose statement also names a temp
  directory. The allowlist is exactly the two guard files.
- The scanner carries string-literal state across lines, so a test that writes the old pattern as
  text is data, never a refusal.
- `larry governance` runs the rule with the rest of the checker, so CI and pre-commit both refuse it.

## Consequences

- A reused process id can no longer land a test in a dead run's tree: the guard empties the
  directory first, and the test that pre-creates the old leftover passes with it present.
- The four earlier `Scratch` copies (`route_cli`, `governance::sim_support`, `demo_journey`,
  `eval_journey`) moved onto the two guards in the same change, so the checker has two entries.
- The rule reads source text, never a running process: a temp path built without `std::process::id()`
  is out of its reach, and the guard's own test is the control that the refusal still fires.
- A new fixture that needs a temp directory adds no guard; it takes one of the two.
