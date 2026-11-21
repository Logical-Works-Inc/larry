---
type: ADR
title: "ADR 0070: a workspace copy writes through no symlink the folder holds"
description: Why the copy checks every ancestor with checked_target before each write, and why a brief refuses .git in any case.
resource: /docs/adr/0070-no-write-through-a-shadowing-symlink.md
tags: [larry, adr, executor, workspace, cwe-59]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#335 failing sim on main, then green with checked_target, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#335, /INVARIANTS.md, https://cwe.mitre.org/data/definitions/59.html, https://git-scm.com/docs/git-ls-files]
---

# 0070: a workspace copy writes through no symlink the folder holds

Status: accepted, 2026-11-21 (larry#335).

## Context

`git ls-files --cached --others --exclude-standard` can list both an untracked symlink
`shadow` and a tracked file `shadow/inside.txt` when the directory was replaced by a link
after the file was committed. The listing is sorted, so `shadow` is copied first as a link,
and the copy of `shadow/inside.txt` then follows it. `stays_beneath` refused only a lexical
`..`, root or any-case `.git`; it does not see that an ancestor already in the copy is a
link. A copy that follows the link writes outside the folder (CWE-59).

`executor::author::checked_target` already walks a brief path with `symlink_metadata` and
refuses a component that is a link. The brief check compared `.git` case-sensitively while
the snapshot compared it with `eq_ignore_ascii_case`, so `.GIT/hooks/x` passed a brief on a
case-insensitive file system.

## Decision

- `copy_entries` calls `checked_target` on each listed path before it writes, so every
  ancestor beneath the tree root is checked with `symlink_metadata`. The check is the same
  one the brief uses, made `pub(crate)` and generic over its path, never a second guard.
- `is_writable_path` compares `.git` with `eq_ignore_ascii_case`, matching the snapshot's
  own rule, so any case of the component is refused.
- The tests are `sim_snapshot_refuses_a_symlink_shadowing_a_tracked_directory` and the
  existing malformed-brief sim. Both failed on main and pass with this change.

## Consequences

- A folder whose tracked directory name is shadowed by an untracked link refuses the copy
  typed, and a sentinel outside the tree stays byte-identical.
- A brief path reaches `.git` in no case on a case-insensitive file system.
- The copy costs one `symlink_metadata` per path component, bounded by the existing file and
  depth limits.

## Rejected

- A second lexical check. Lexical is the defect; it cannot see a link already copied.
- `std::fs::canonicalize`. It resolves and follows the link the check exists to refuse.

Reversal: if a copy needs to follow a link on purpose, this ADR reopens with that case.
