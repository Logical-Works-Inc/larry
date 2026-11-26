---
type: ADR
title: "ADR 0123: Local CI receipt directory"
description: Why local CI receipts use the requested state or checkout-owned directory.
resource: /docs/adr/0123-local-ci-receipt-directory.md
tags: [larry, adr, local-ci, receipts]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: cargo nextest run and cargo clippy on Oracle, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#440, larry#434]
---

# ADR 0123: Local CI receipt directory

## Context

The local CI binary used a fixed `$HOME/wwfd/state` path. Worktrees and
operators could not select the receipt directory, and one home path could hold
receipts from separate runs.

## Decision

`cargo local-ci --receipts <dir>` writes `local-ci-receipt.json` in `<dir>`.
Without the flag, `LARRY_STATE_DIR` supplies the directory. Without either,
`CARGO_TARGET_DIR/local-ci` does, and with none of them the checkout root's
`target/local-ci`. A chosen path is made absolute against the caller's
directory, so a relative one means where the caller stood, not the root.

The directory is resolved before git or any lane runs. An empty value, a
non-UTF-8 `LARRY_STATE_DIR` or a `--receipts` without one directory exits 1
naming its source and writes nothing. The flag's path is kept as an
operating-system path.

## Consequences

Receipts stay with the selected state or checkout target. Operators can direct
the artifact to a shared durable directory when they need it. A malformed
state environment fails before any lane runs, so it cannot produce an
unaccountable receipt.

## Rejected

`$HOME/wwfd/state` was rejected. It ignores the caller's checkout and state
selection, writes outside the requested run, and makes separate worktrees
share one receipt.

## Proof

`tests/local_ci_journey.rs` drives the shipped `larry-local-ci` from a folder
that is not a checkout: a non-UTF-8 or empty `LARRY_STATE_DIR`, an empty
`--receipts` and a `--receipts` with no or two words each exit 1 naming their
source before git runs, and the folder stays empty. The binary's unit tests
hold the order, the absolute paths, the root default and the written receipt
under a `TempDir`. A green run of every lane is the receipt's own journey;
it is NOT RUN here, because it builds and tests the whole workspace.
