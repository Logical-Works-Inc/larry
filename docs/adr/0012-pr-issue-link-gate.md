---
type: ADR
title: "ADR 0012: CI fails a pull request that closes no issue"
description: Why the issue-link job reads GitHub's closingIssuesReferences instead of matching the PR body.
resource: /docs/adr/0012-pr-issue-link-gate.md
tags: [larry, adr, ci, traceability]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: the issue-link job on its own PR, with and without Closes in the body}
status: accepted
stale_after: 2027-02-07
sources: [larry#103, Director order 2026-11-03, GitHub GraphQL PullRequest.closingIssuesReferences]
---

# 0012: CI fails a pull request that closes no issue

Status: accepted, 2026-11-07 (larry#103)

## Context

Every PR must close at least one issue (Director 2026-11-03: "work based on each pr carries GH issues").
INVARIANTS.md held that rule as review-only, and `main` has no branch protection, so nothing stopped a PR
that carried no issue.

## Decision

- An `issue-link` job in `.github/workflows/governance.yml` runs on every PR open, push, reopen and body
  edit. It reads `closingIssuesReferences` with the run's read-only token and fails when the list is empty.
- GitHub's own link decides, not a regex over the body. A keyword GitHub does not honour, or `Closes #N`
  inside a code block, links nothing, so a regex would pass a PR whose merge closes nothing.
- It runs on the same runner as the other jobs, chosen by visibility (larry#116).

## Consequences

- A PR without `Closes #N` shows a red `issue-link` check. Editing the body reruns it.
- The merge is still gated by the green run plus the guard, because `main` has no protection to require
  the check.
