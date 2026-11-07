---
type: Evidence
title: docs/dependencies/ADAPTER-MANIFEST.md, part 2 of 2
description: Part 2 of /docs/dependencies/ADAPTER-MANIFEST.md, in order; the index lists every part.
resource: /docs/dependencies/adapter-manifest/02-in-workspace-consumption.md
tags: [rocco, adapters, licenses, dependencies, evidence, c01]
generated: {by: opencode, at: 2026-09-16}
updated: {by: agent:claude-code, at: 2026-09-18, note: one-repo consolidation corrected the consumption section}
verified: {by: opencode, method: read sibling and lgwks Cargo.toml license fields, contract/APPROVED.toml and pinned-revision presence checks, ledger receipts and specification Cell 11 allocation lines; no integration executed}
status: active
stale_after: 2026-10-18
sources: [/specification.md, /research-ledger.md, /docs/dependencies/README.md, sibling Cargo.toml manifests read 2026-09-16, contract/APPROVED.toml read 2026-09-18, `lgwks-deps check .` run 2026-09-18]
---
## In-workspace consumption

CORRECTED 2026-09-18: the second repository no longer exists. The contract
crate and the fixture surface are one workspace at the root of this repository
(one `Cargo.toml`, one `Cargo.lock`, one lint set, one `contract/APPROVED.toml`).
The paragraph this section replaces described a separate `srinji-kaggss/
rocco-runtime` repository and a register read at its commit `1154959`; both are
superseded, and the version and key names it recorded no longer match.

- READ 2026-09-18, `contract/APPROVED.toml` in this repository, validated by
  `lgwks-deps check .` (exit 0, "1 semantic approvals, every authored external
  edge is owned"). The register holds `[policy]` and exactly one `[[approved]]`
  row: `thiserror =2.0.20`, tier `boundary`, owner `rocco-contracts`,
  capability `error-derive`, allowed consumer `rocco-contracts`.
- The `lgwks_*` edges carry no row by design: `lgwks_std` is ladder rung 2
  (workspace stdlib+, self-exempt) and `lgwks_deps` is the estate's own
  admission storefront, so the checker reports both as owned without one. The
  former runtime register declared all four in a shape this checker refuses and
  was never schema-checked against it; the merge is the first time it has been.
  Two of its keys (`features`, `default_features`) do not exist in this schema.
- The manifest pins the crates as exact releases (`=`), declares no path, git,
  vendor or fork edge, and the workspace declares `lgwks_bot =0.3.2` in the
  catalog with no member depending on it yet — a catalog entry is not an edge.
- Disclosure, still open: `lgwks_ast` 0.1.3 is observed at that version in the
  upstream Cargo.toml (READ) and named by D21 as the existing parser, but it is
  not a dependency of any member here and has no register row. That is correct
  while nothing depends on it; it becomes a registration obligation the moment
  something does.
- `lgwks_deps` moved `=0.1.7` → `=0.1.8` in the merge, because 0.1.8 is the
  published release carrying the `appcui` feature the native surface builds
  against. The register records that correction.
- forge/forge-harness/forge-sdk remain reference reading only — Director
  correction 2026-09-16 (known hidden bugs): never imported, copied or
  ported. Other agent harnesses are likewise reference only.

## File-specific excellence record

Requirement: C01 exact adapter/API/source-license manifest per Cell 11, R73
adapter boundary, D21 narrow tested contracts, falsifiers E22 and E57. Axes:
decoupled, generalized, portable, multi-tenanted-by-default reuse through
canonical owners. Predicted effect: one evidence surface with zero invented
readiness and zero vendored twins. Verification: license fields read from
sibling Cargo.toml bytes this session; pinned revisions presence-checked by
`git cat-file -t` (all "commit"); gate receipts below. Unresolved: Agent
Skills package-spec pin (ledger 404), Sentinel workspace-root license
UNKNOWN, lgwks_ast not yet in APPROVED.toml, all sibling integration results
NOT RUN.
