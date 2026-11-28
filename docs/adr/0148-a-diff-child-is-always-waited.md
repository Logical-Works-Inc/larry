---
type: ADR
title: "ADR 0148: the diff waits for its git child on every return"
description: "One supervise step reads git's two streams and waits for it, whatever failed."
resource: /docs/adr/0148-a-diff-child-is-always-waited.md
tags: [larry, adr, executor, workspace, process]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#620, CWE-404, CWE-772, INVARIANTS.md]
---

## Status

Accepted.

## Context

The workspace diff returned early when its stream or its stderr reader failed. It
sent a kill on a stream fault and never waited, and did neither on a stderr
fault, so the git child stayed in the process table until the host exited.
`larry serve`, `larry mcp` and `larry acp` are long-lived (CWE-772).

## Decision

The executor makes both pipes with `std::io::pipe` and hands the child and the
two read ends to one `supervise` step. That step reads both streams, kills the
child only when the diff stream failed, waits for it, and only then answers.

## Consequences

The arm for pipes that were never opened and its `reap` helper are gone; they
could not run.

Rejected: a wait added to each early return. It is three copies of one cleanup.
Reversed by: a git diff that must outlive its failed reader.
