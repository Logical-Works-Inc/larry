---
type: ADR
title: Typed verification gate before session finish
description: A terminal session can end only after a fresh passing check receipt and present artifacts.
resource: /docs/adr/0130-verification-gate.md
tags: [larry, executor, verification, receipt]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: design-gate and seeded executor tests}
status: accepted
stale_after: 2026-12-26
sources: [larry#271, /INVARIANTS.md, /docs/product/SHIP.md]
---

# ADR 0130: Typed verification gate before session finish

## Context

Interactive sessions previously accepted a model-written `done` reply after
any command history. That reply carried no executor-owned proof that the last
change still matched a passing check.

## Decision

The executor parses `finish` as a typed request. The request names a summary,
an exact check command, artifact paths, retained state and servers.

The executor accepts the first request only after the last command is that
exact command, its fresh shell exits zero without timeout or interruption, and
every artifact exists in the session tree. It returns the checklist. The same
request must be sent again to end the session.

Any command after the passing check replaces the last receipt. A model-written
`done` token is not a completion path. The sealed report carries the check,
`fresh_shell: true`, artifact paths, retained state and servers.

## Why

Prompt text cannot prevent a model from claiming success after it edits again.
The executor owns the command receipts and the artifact tree. The gate makes
the success claim depend on those observations.

## Proof

`finish_refuses_without_fresh_passing_check` rejects the old token, a failed
check and a different last command. The seeded reply parser continues to test
bounded hostile replies. The PTY and ACP journeys remain the shipped binary
proof paths; their acceptance cells must send the typed finish request.

Traceability: larry#271. MEASURED and NOT RUN values are printed by the PR
checks. No model token overrides the executor decision.

## Consequences

Models must run a check immediately before the typed finish and repeat the
finish request. Reports gain a bounded verification member. Existing callers
that send `done` receive a typed refusal and must use the new protocol.
