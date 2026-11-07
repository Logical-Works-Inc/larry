---
type: ADR
title: "ADR 0007: fixture repositories run no background git"
description: Why the executor journeys turn off git's automatic maintenance in every origin they build.
resource: /docs/adr/0007-fixture-repos-run-no-background-git.md
tags: [larry, adr, executor, tests, flake]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: GIT_TRACE=1 git commit traced on git 2.54; cargo nextest run --workspace --locked}
status: accepted
stale_after: 2027-02-07
sources: [larry#138, larry#137, https://git-scm.com/docs/git-maintenance]
---

# 0007: Fixture repositories run no background git

Status: accepted, 2026-11-07 (larry#138)

## Context

PR #137 changed three documents, and `a_hostile_command_leaves_the_origin_byte_identical` failed on it while main
passed the same code (run 42340352636). The origin's digest moved before the executor could have touched it.
`git commit` ends by forking `git maintenance run --auto --quiet --detach` (seen with `GIT_TRACE=1`), and every
maintenance run takes a lock on the object database. That lock file can sit under `.git/objects` while
`tree_digest` walks the origin, so the before and after digests disagree with no executor fault.

## Decision

- `tests/support/exec_cli.rs` sets `maintenance.auto=false` and `gc.auto=0` in each origin's own config right
  after `git init`, so every later git call on that origin, the fixture's commit included, starts no background
  writer. `gc.auto=0` covers gits that still run `gc --auto` instead.
- Rejected: a retry until two digests agree (a wall-clock race left in place), `-c` flags on the helper only (a
  test that runs git on the origin directly would still fork), and process-wide `GIT_CONFIG_*` variables (they
  mutate the environment under threaded tests and leak to the `larry` child).

## Consequences

- `a_fixture_origin_starts_no_background_maintenance` commits in a fixture origin under `GIT_TRACE=1` and fails
  if any `maintenance run` child starts; without the two keys it fails on git 2.54 and later.
- The executor itself is unchanged: its own git calls never run auto-maintenance on the user's origin.
