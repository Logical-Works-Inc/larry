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

Around every model command the turn takes a bounded fingerprint of the session
tree (the island's tree when isolated, the session directory on
`Isolation::Host`): every entry's relative path, kind, length and content
digest (BLAKE3 through `lgwks_std`), walked without following symlinks. The
walk reuses the snapshot's shape and bounds (a codegraph `snapshot` query
before writing it; the snapshot walk lists copy entries without digests and the
sim digest is test-only, so neither is called directly). The bounds are named as constants in
`session_gate`: at most 20,000 entries, 64 directory levels and 268,435,456
content bytes (`MAX_FINGERPRINT_ENTRIES`, `MAX_FINGERPRINT_DEPTH`,
`MAX_FINGERPRINT_BYTES`). Past a bound the check is refused typed, never
passed.

The executor accepts the first request only after the last command is that
exact command, its fresh shell exits zero without timeout or interruption, its
before and after fingerprints match, and every artifact exists in the session
tree. It returns the checklist. The same request must be sent again to end the
session. A check whose after fingerprint differs from its before fingerprint is
no receipt: `VerificationFault::CheckMutated` naming the first changed path in
sorted order.

Any command after the passing check replaces the last receipt. A model-written
`done` token is not a completion path. The sealed report carries the check,
`fresh_shell: true`, artifact paths, retained state and servers.

## Why

Prompt text cannot prevent a model from claiming success after it edits again.
The executor owns the command receipts and the artifact tree. The gate makes
the success claim depend on those observations.

## Proof

`finish_refuses_without_fresh_passing_check` rejects the old token, a failed
check and a different last command.
`sim_generated_tasks_match_the_reference_verdict_for_every_seed` drives the
turn loop's production functions over generated tasks with scripted edits,
checks and finishes: the gate's verdict matches an independent reference rule
for every finish of every seed, and the same seed replays the same trace hash.
`sim_run_one_command_honors_the_persons_answer_for_every_seed` drives the
turn loop's own command step with a scripted person over generated tasks: a
declined or cancelled command runs nothing, and an approved command's stored
proof matches the tree's own before/after diff for every step of every seed.
The bound tests refuse past the entry bound and accept at it, and an empty
artifact name is refused by the gate's own path rule first.
The binary journeys prove the shipped path: a check that writes the tree is
refused with no receipt, an edit after the check voids the finish, and a model
that never checks ends in refusal after three refusals. The seeded session
parser continues to test bounded hostile replies. The PTY journeys run the
edit, then a read-only check, then the finish pair.

Traceability: larry#271. MEASURED and NOT RUN values are printed by the PR
checks. No model token overrides the executor decision.

## Consequences

Models must run a read-only check immediately before the typed finish and
repeat the finish request; an edit, including the check itself when it writes,
is refused as `CheckMutated`. Reports gain a bounded verification member.
Existing callers that send `done` receive a typed refusal and must use the new
protocol.
