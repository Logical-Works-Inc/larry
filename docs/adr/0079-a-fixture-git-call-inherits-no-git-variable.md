---
type: ADR
title: "ADR 0079: a fixture git call inherits no GIT_ variable"
description: Why every test git call removes the GIT_* variables it inherits, after a commit hook's GIT_DIR made fixtures write the host repository.
resource: /docs/adr/0079-a-fixture-git-call-inherits-no-git-variable.md
tags: [larry, adr, tests, git, portable]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: a_hook_git_dir_never_reaches_the_host_repository red before the scrub and green after, on Oracle, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#375, larry#340, larry#269, /INVARIANTS.md]
---

# 0079: a fixture git call inherits no GIT_ variable

Status: accepted, 2026-11-21 (larry#375, split from #340).

## Context

A commit hook runs the tests with `GIT_DIR` and `GIT_INDEX_FILE` set. In a git
worktree they are absolute paths into the host repository. `git_command` in
`tests/support/origin.rs` cut the host's config files (larry#269) but kept these
variables. So, on 2026-11-21, with three workers committing from worktrees:

- a fixture `git init` set `core.bare=true` on `~/larry/.git`, and the main
  checkout refused every command;
- the fixture's `maintenance.auto` and `gc.auto` settings landed in the host
  config;
- a fixture commit landed on a worker branch (`0cafd33` on `ds/issue-366`).

## Decision

- `git_command` removes every `GIT_*` variable the test process inherited, then
  sets its own. The list comes from `std::env::vars_os`, so a variable git adds
  later is removed too.
- Production git calls (`executor::workspace::Git`, `executor::sim_tree::git`)
  already start from `env_clear`, so they need no change.
- Test: `a_hook_git_dir_never_reaches_the_host_repository` builds a fixture with
  both variables pointing at a second repository. That repository keeps
  `core.bare=false` and its one commit, and the fixture gets its own repository.

## Consequences

- A test run from a commit hook, in a checkout or a worktree, cannot write the
  repository that runs it.
- A test that wants a `GIT_*` variable must set it on the command after
  `git_command` builds it.

## Rejected

- `env_clear` on every test git call. It drops `PATH` and `HOME`, which the
  hermetic tool lookup and the hostile-home test rely on.
- Removing only `GIT_DIR` and `GIT_INDEX_FILE`. `GIT_WORK_TREE`,
  `GIT_COMMON_DIR` and `GIT_OBJECT_DIRECTORY` point at the host the same way.

Reversal: none expected. If git ever needs a `GIT_*` variable to run at all, it
is set by `git_command` itself, by name.
