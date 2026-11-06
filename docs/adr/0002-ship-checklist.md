---
type: ADR
title: One ship checklist holds order and state
description: Remaining work is one ordered checklist in the plan's home; each shipping PR ticks its own line.
resource: /docs/adr/0002-ship-checklist.md
tags: [larry, plan, checklist, adr]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: issue list and proof clauses read 2026-11-06}
status: accepted
stale_after: 2027-03-21
sources: [larry#125, /docs/product/SHIP.md]
---

# 0002 One ship checklist holds order and state

## Status

Accepted 2026-11-06 (larry#125).

## Context

The order of work lived in issue #114 (weekend only), the plan's track prose and about sixty open issues.
A session had to sweep all of them to find its next PR. The cell tracker counts specification cells, was
last measured 2026-09-22, and renders through Python, which is banned locally until #53.

## Decision

`/docs/product/SHIP.md` lists every remaining PR-sized issue in ship order with its proof. A session takes
the first unchecked line; the PR that ships it ticks the line and appends the PR and measured number (a
squash SHA exists only after merge, so git log holds it). Issues keep the detail, `specification.md` keeps every ID, and the cell tracker stays the cell
dashboard.

## Consequences

- One file read gives a session its next PR.
- A ticked line without its PR and number is a defect a reviewer refuses.
- No CI job checks the ticks yet; the rule is review-only, owner Director, until #103's job can read them.
