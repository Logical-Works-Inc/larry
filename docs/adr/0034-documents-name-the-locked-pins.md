---
type: ADR
title: "ADR 0034: the documents name the locked estate pins, checked against Cargo.lock"
description: Why Larry moves to lgwks_bot 8.2.0 and lgwks_deps 6.3.0 by naming only those packages, and why a test ties the cargo sweep's pin line to the lock.
resource: /docs/adr/0034-documents-name-the-locked-pins.md
tags: [larry, adr, dependencies, governance]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: cargo update -p, cargo nextest run --workspace --locked (799 passed), 32 larry exec pairs}
status: accepted
stale_after: 2027-02-09
sources: [larry#180, larry#178, logicalworks-crates#897]
---

# 0034: The documents name the locked estate pins

Status: accepted, 2026-11-09 (larry#180). Numbered 0034 because the cloud session's PR #181 holds 0028 to 0033.

## Context

crates.io published lgwks_bot 8.2.0 (with crates#897) and lgwks_deps 6.3.0 on 2026-11-09, hours after larry#178
had written 8.1.1 and 6.2.1 into the documents as newest. The cargo sweep had already drifted once ("Larry ships
on 8.0.1" while 8.1.1 was locked), and only GOVERNANCE.md's lgwks_deps release had a test.

## Decision

- Move the two exact pins and run `cargo update -p lgwks_bot -p lgwks_deps`: four packages move (lgwks_ast 6.0.2
  and lgwks_macros 2.1.1 with them), and appcui stays on its held pair (crates#841). A whole `cargo update` is
  refused because it would move that pair and 83 transitive packages.
- `governance::tests::the_cargo_sweep_names_every_locked_estate_release` requires the sweep's newest-release line
  to list the locked lgwks_std, lgwks_bot, lgwks_ast and lgwks_deps, in that order.

## Consequences

- A pin move that leaves the sweep behind fails the workspace tests; INVARIANTS.md names both pin tests.
- `larry exec` over 32 interleaved pairs against main a471c60: p50 85.1/86.3 ms, p99 167.4/154.0 ms, peak RSS
  13975552/14057472 bytes.
- #79 can build on 8.2.0's `RecordProjection::empty` and `fold_one`.
