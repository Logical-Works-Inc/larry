---
type: ADR
title: "ADR 0038: a truncated diff is never accepted, and a command keeps the tail of its output"
description: How `larry exec` sizes the staged diff and the command streams it reports (larry#206), what the sealed report carries, and how `larry verify` re-derives the verdict.
resource: /docs/adr/0038-truncated-diffs-and-log-tails.md
tags: [larry, adr, executor, receipts, diff, capture]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: tests/receipt_journey.rs on the shipped larry binary, unit sims in src/executor/receipt_tests.rs and src/executor/run_tests.rs}
status: accepted
stale_after: 2027-02-14
sources: [larry#206, ADR 0006, git-diff, GNU tail, tail(1), docker container logs, podman logs, BLAKE3]
---

# ADR 0038: truncated diffs and log tails

## Status

Accepted, 2026-11-14, with larry#206.

## Context

A diff over 1 MiB was cut at the limit and sealed as accepted (`workspace.rs` truncated it with no flag,
and `receipt.rs` accepted any string diff). A planner that lands an accepted report then lands a partial
patch. A command's capture kept only its first megabyte, so the tail of a failing log, where the error is,
was lost. git's diff stream was also buffered whole, so memory grew with the change.

The end of a log is the part a reader needs first: `tail(1)` prints the last lines of a file, and
`docker logs --tail` and `podman logs --tail` retrieve the end of a container's output.

## Decision

- The staged diff streams from git's stdout in 8 KiB pieces into the estate's BLAKE3 hasher
  (`lgwks_std::hash`). Only its first `DIFF_LIMIT` (1 MiB) bytes are kept.
- The report carries `diff` (the kept text), `diff_truncated` (true when any byte came past the head),
  `diff_bytes` (the full length) and `diff_digest` (the BLAKE3 of the full diff, 64 hex).
- Only `executor::receipt` derives acceptance. A diff that is cut is never accepted. A report sealed
  before `diff_truncated` existed is cut only when its diff reaches the limit; a non-boolean member is
  cut. `verify` refuses a signed `accepted: true` over a cut diff with `TruncatedDiffAccepted`.
- Each command stream keeps its first 512 KiB and its last 512 KiB, read in 8 KiB pieces. Between them
  sits a marker line, `[larry: N bytes dropped]`, and the report carries `stdout_dropped` and
  `stderr_dropped` with the exact count. `truncated` is true when either is nonzero.
- A diff's stderr is kept to 64 KiB and drained past it, so git never blocks on a full pipe.

## Consequences

- A retry answers from the ledger's sealed bytes, and `Report::parse` re-verifies them through
  `receipt::verify`, so the truncation rule holds for a retried report too (ADR 0037).
- A report gains four members for the diff and two per command. Existing readers that name members by
  key keep working; no member is renamed or removed.
- Memory per diff and per command stream is bounded: at most 1 MiB of diff text, and at most 1 MiB
  plus one 8 KiB piece per stream while it reads.
- Acceptance now refuses a change over 1 MiB. Such a change needs a smaller task, not a larger cut.
- Cold build, dependency count and binary size are NOT RUN in this change; the Oracle gate records them.
