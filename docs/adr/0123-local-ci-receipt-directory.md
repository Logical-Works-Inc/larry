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
the receipt is written under `target/local-ci`, or the configured Cargo target
directory's `local-ci` child.

An invalid `LARRY_STATE_DIR`, including a non-UTF-8 value, fails before lanes
run. The error names `LARRY_STATE_DIR`. The flag has precedence over the
environment and its path is passed through as an operating-system path.

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

The local CI binary resolves the directory before running lanes, then uses the
existing atomic receipt replacement. Unit coverage proves flag precedence, the
target default, and the typed non-UTF-8 refusal. The binary journey runs the
same command with no flag and with `LARRY_STATE_DIR`.
