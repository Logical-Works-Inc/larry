---
type: ADR
title: "ADR 0005: rename Rocco to Larry across the repository"
description: Why every package, crate, binary, path and document says Larry, and what keeps its old text until larry#53.
resource: /docs/adr/0005-rename-rocco-to-larry.md
tags: [larry, adr, rename, governance]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: cargo nextest run --workspace --locked (586 passed) on the renamed tree}
status: accepted
stale_after: 2027-02-06
sources: [larry#131, larry#53, logicalworks-crates#872, logicalworks-crates#873]
---

# 0005: Rename Rocco to Larry

Status: accepted, 2026-11-06 (larry#131)

## Context

This repository was named Rocco before it became Larry, the always-on harness organ of Logic OS. Rocco now names a
separate embedded harness, the in-app dock that cannot speak. Package, binary, crate and document names still said
Rocco, and the Director ordered: "Please make sure there's no Larry vs Rocco confusion. Reformat and rename the
code", then "Rename Rocco to Larry".

## Decision

- The root package is `larry-demo` (lib `larry_demo`, demo binary `larry-demo`); the shipped binary is `larry`
  (`larry exec`, `larry model`, `larry mcp`; no argument opens the terminal UI).
- `crates/rocco-contracts` is `crates/larry-contracts` and `crates/rocco-runtime` is `crates/larry-runtime`.
- One case-preserving rewrite covers every tracked text file, with `git mv` for each path. Signed receipts under
  `tools/receipts/`, measured evidence and external schema fixtures keep their bytes.
- Python and shell files keep their text: the repository holds no new Python or shell (2026-10-13), and larry#53
  ports them to Rust, renaming as it goes.
- Dependency order is stated once in `Cargo.toml`: std, then the lgwks_* estate, then Loco.rs for the gaps the estate
  does not own, then any other OSS crate with a cited logicalworks-crates `gap` issue.

## Consequences

- Every Rust import, test binary (`CARGO_BIN_EXE_larry`) and CI step uses the Larry names; 586 tests pass.
- `tools/test_native_cli.py` and `tools/test_demo_cli.py` still name `target/debug/rocco*`. CI does not run them
  (the governance job runs `tests/` only), and larry#53 replaces them.
- The frozen over-size documents (specification.md, log.md, research-ledger.md, docs/workspace-experience.md,
  docs/architecture/visual-pass.md, docs/architecture/runtime.md, docs/dependencies/ADAPTER-MANIFEST.md) keep their
  Rocco text: the size law refuses any edit to them and the Python checker reads them by name, so larry#53 renames
  them as it splits them. Every other document reads Larry; the Git history keeps the original words.
- larry#53 split them without renaming; larry#178 renamed them and their parts (2026-11-09). Names that still exist
  under the old spelling keep it: `srinji-kaggss/Rocco` and `srinji-kaggss/rocco-runtime`, the
  `rocco-macos-arm64` runner, the deleted `tools/rocco_pixel.py`, the `rocco-raccoon` receipts, lgwks-frontend's
  `rocco-rank.js`, quoted Director words and log.md's record of the original naming. Rocco is now the embedded dock.
- Touching each renamed file put it under today's gates whole: over-size files were split (native, workspace,
  authority, the fence tests, three forge-spec documents, WORKSPACE-EVIDENCE) and every `unwrap_or`/`map_or`
  default became an explicit case.
- The parallel suite exposed two executor flakes, fixed here: concurrent first opens of one ledger are serialized
  by a kernel-released file lock (logicalworks-crates#872, #873), and a Darwin `EPERM` from `killpg` is settled
  and then confirmed by the group-gone probe. Measured over 24 parallel runs each: 7 and 2 failures before, 0 after.
