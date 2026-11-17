---
type: ADR
title: "ADR 0063: the temp-path rule reads every workspace member"
description: Why larry governance reads the workspace members from the root Cargo.toml, refuses a missing root by name, and follows #[path] and include! modules.
resource: /docs/adr/0063-temp-rule-every-member.md
tags: [larry, adr, governance, tests, workspace]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: larry-oracle on the larry#311 branch, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#311, larry#293, larry#267, /docs/adr/0061-temp-rule-on-lgwks-ast.md, https://doc.rust-lang.org/cargo/reference/workspaces.html, https://doc.rust-lang.org/reference/items/modules.html]
---

# 0063: the temp-path rule reads every workspace member

Status: accepted, 2026-11-17 (larry#311)

## Context

ADR 0061 moved the temp-path rule onto `lgwks_ast` and judged one function at a time, but the rule
still read only the root package's `src` and `tests`. Three holes remained:

- `crates/larry-runtime` was never scanned, so its test helpers named journal files
  `larry-durable-<pid>-...` in the system temp directory, exactly the reuse defect larry#267 exists
  for.
- A missing root passed with zero files read (`temp_paths::errors`, the `NotFound` arm), so a moved
  or renamed directory silently stopped the rule.
- A `#[path = "..."]` or `include!("...")` module outside `src` and `tests` was never read, so test
  code could move out of the walk without moving out of the build.

## Decision

- The roots come from the root `Cargo.toml`, read as the repository's own file: the
  `[workspace] members` array, each entry a literal path or a trailing `*` glob expanded to the
  directories that hold a `Cargo.toml`, plus the root package when `[package]` is declared.
- The rule does not call `cargo metadata`. The call needs a writable target directory and a
  lockfile, its answer can change without a source change, and larry#311 forbids it. A manifest that
  is a symlink, is not UTF-8, or names an unsupported member form is a typed `GovernanceError`.
- Every member's `src` and `tests` is read. A missing one is `GovernanceError::MissingRoot` naming
  the directory, never a pass over zero files.
- `#[path = "..."]` and `include!("...")` are followed, resolved beside the naming file and kept
  inside the checked tree; an absolute path, a symlink, a missing file or a path outside the tree is
  a refusal naming the file that names it. A followed module inherits the test-code status of the
  file that names it. An `include!` whose argument is not a string literal is not followed: its
  source is generated outside the tree and cannot be read statically.
- Every pid temp path the wider scan found in `crates/larry-runtime` now runs through the one shared
  guard, `tests/support/temp_dir.rs`, included by `#[path]` from the crate's in-crate unit tests and
  from `tests/support/mod.rs` for its integration tests. The guard makes a fresh directory and
  removes it on drop; the journal file lives inside it.
- The rule splits its workspace sims into `governance::sim_temp_workspace` so no code file passes
  the 750-line law.

## Consequences

- The live tree's member list is the root package and both crates, proved by
  `governance::tests::governance_names_every_workspace_member`.
- Each hole has a seeded simulation: a member crate's leak, a missing root by name, a `#[path]`
  module and an `include!` file refused at their own line, an escaped include refused, a package
  manifest with no workspace section, a glob member, and a manifest refusal.
- The rule now parses the runtime crate's sources too, so one check reads more files. The cost
  control reads the workspace once and prints p50, p95 and p99 over 100 checks.
- A member without an integration-test directory now fails the check. That is deliberate: the rule
  promises to read every member's `tests`, and a silent zero there is the fail-open case this ADR
  removes.

## Rejected

- `cargo metadata --no-deps`: exact member resolution, but a process call, a target directory and a
  lockfile, and larry#311 forbids the call.
- Scan `crates/*` by convention: no manifest read, but a member outside `crates/` hides and the scan
  and the build disagree.
- A TOML crate or a general TOML parser: a new dependency and a gap issue for one array of strings.
- Require only existing roots and skip a missing `tests` directory: that is the fail-open arm the
  order removes.
