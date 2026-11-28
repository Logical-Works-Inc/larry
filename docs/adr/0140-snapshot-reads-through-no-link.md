---
type: ADR
title: "ADR 0140: a workspace snapshot reads through no link in the folder"
description: "The copy refuses a link above a listed path in the source folder and skips a path whose directory became a file."
resource: /docs/adr/0140-snapshot-reads-through-no-link.md
tags: [larry, adr, executor, snapshot, security]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#592, larry#335, ADR 0046, ADR 0070, INVARIANTS.md]
---

## Status

Accepted.

## Context

`git ls-files` lists a tracked path even when a directory above it is now a
symlink that the ignore rules hide. It never lists the link. The copy read
through the link and put a file from outside the folder into the workspace that
a model reads (CWE-59). A tracked directory replaced by a file gave `ENOTDIR`
and refused the whole snapshot.

## Decision

`copy_entries` walks the parent of each listed path in the source folder with
`checked_target`, the walk the tree side already uses. A link there refuses the
copy. `ENOTDIR` from the stat means the entry is gone, as `ENOENT` does. The
stat runs before the tree check.

## Consequences

A folder with such a link is refused with `AuthorFault::Symlink` naming the
directory. A person removes the link and runs again. Each entry costs one more
`lstat` per directory above it. `git ls-files` flags were rejected: git still
lists the tracked paths beneath an ignored link. A name swapped between this
walk and the read is not closed here.
