---
type: ADR
title: "ADR 0088: a test proves the claim its name and invariant make"
description: The seven larry#340 cases check what they say, or the claim is cut.
resource: /docs/adr/0088-tests-prove-what-they-claim.md
tags: [larry, adr, tests, evidence]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: targeted nextest of the seven cases and one clippy run}
status: accepted
stale_after: 2027-02-21
sources: [larry#340, larry#321, ADR 0013, ADR 0042, ADR 0053]
---

# 0088: a test proves the claim its name and invariant make

Status: accepted, 2026-11-21 (larry#340)

## Context

The 2026-11-19 review found seven cases whose assertion is weaker than their name or their
INVARIANTS line: a kill after spawn that never reaches the write, an RSS check that skips
when `ps` takes no sample, a kill point counted in yields, a grid bound true by construction,
a cost printed over a partial sum, a receipt written in place, and a revision constant
presented as a weights check.

## Decision

- A case asserts the event its name names. The tracker kill observes the temporary page and
  kills inside the write; the RSS check fails when no sample arrives; the executor kill point
  is the fixture's fifo-and-ledger seam and the name says it, never a yield count; the chat is
  read right of the pane rule; an incomplete card prints `unknown` per accepted task.
- `larry-rerank` writes its receipt to a temporary sibling and links it into place, so a kill
  leaves the previous state or the whole receipt. Rejected: `rename(2)`, which overwrites a
  receipt the create-exclusive rule refuses.
- The receipt's revision is the snapshot label `model.pin` names, not a digest of the
  weights; the code and INVARIANTS say so. Rejected: pinning a weights digest, because no
  host that runs the gates holds the weights to measure it (SD-5, SD-10).
- larry#375 already removes every inherited `GIT_*` variable from `git_command`; this change
  records that the claim holds and changes nothing there.

## Consequences

- Each case fails on the defect it guards: the cost case fails on the old card (seed 0), the
  receipt case fails on a temporary left behind, the tracker and executor cases observe the
  event they name.
- The weights check is NOT RUN, and the claim is cut rather than left overstated.
