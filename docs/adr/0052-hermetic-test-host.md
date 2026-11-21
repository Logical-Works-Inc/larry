---
type: ADR
title: "ADR 0052: tests are independent of the host"
description: Why every test git call, terminal PATH, RSS reading and serving-line read is fixed at one seam.
resource: /docs/adr/0052-hermetic-test-host.md
tags: [larry, adr, tests, flakiness, hermetic]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: larry-oracle all on the larry#269 branch, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#269, git-config(1) GIT_CONFIG_GLOBAL and GIT_CONFIG_NOSYSTEM, POSIX ps(1), /docs/adr/0051-temp-path-guard.md]
---

# 0052: tests are independent of the host

Status: accepted, 2026-11-16 (larry#269)

## Context

The suite read the machine it ran on. Each of these changed a result:

- `git init` took the host's `init.defaultBranch`, `commit.gpgsign` and `init.templateDir`, and
  `git commit` ran the host's hooks and identity. A developer's `~/.gitconfig`, or a machine's
  `/etc/gitconfig`, decided a test's branch, its commit and whether it committed at all.
- The terminal opened by `tests/support/pty.rs` got the host PATH, so a tool the host put first
  ran inside a journey.
- `tests/governance_journey.rs` read peak RSS from `/usr/bin/time`, which not every host ships,
  and parsed a different flag per OS.
- `tests/support/serve_cli.rs` read the serving line with a blocking `read_line`, so a host that
  never printed one hung the test with no message.

## Decision

- One builder, `git_command` in `tests/support/origin.rs`, makes every test git command: the
  global file is `/dev/null`, the system file is off, `main` is the branch default, signing is
  off, `core.hooksPath` is empty (git-config: an empty value runs no hooks), and the author and
  committer are fixed. The origin fixture and `commit_all` take their commands from it, and the
  `git status` call in `tests/e2e_journey.rs` does too. The builder's environment can be replaced
  by a caller, which is how one test hands the whole fixture a hostile `HOME`.
- The terminal's PATH is built from where a hermetic lookup resolved `sh`, `git`, `script`,
  `stty`, `cp` and `sleep`: one fixed directory list in `tests/support/pty.rs` is statted, never
  the host's PATH or shell configuration, and a missing tool is a typed setup error naming it.
- Peak RSS is the probe's own `ps` sample, shared by `tests/support/ps.rs`, taken while the check
  runs. No new crate and no `unsafe` call; `getrusage` would need both.
- The serving line and every later line are read by a named thread the `Serving` owns, and the
  test waits on a bounded channel with `recv_timeout`. A host that never prints fails with the
  bound named; killing the child closes the pipe, and the drop joins the thread.
- A hostile-HOME test proves the git guard: a fixture whose `~/.gitconfig` sets `gpgsign` on,
  `defaultBranch` trunk, a template directory and a failing pre-commit hook still builds a `main`
  repository with an unsigned commit by the fixed identity and no template file.

## Consequences

- A developer's git configuration, a shadowing PATH entry, a missing `/usr/bin/time` or a silent
  host can no longer change a test's result. The hostile-HOME test fails if any git guard moves.
- The empty `core.hooksPath` is the one spelling of "no hooks" that the harness tamper gate
  accepts in a source file; the gate refuses a literal path, and it cannot tell a test fixture
  from a planted commit hook. git-config defines the empty value as no hooks, so the guard holds.
- The two screen-model tests moved from `tests/e2e_journey.rs` to the `support/pty.rs` module they
  exercise, which keeps the journey file inside the 750-line code law while it grows one journey.
- A test that samples RSS reads one point in time, not the kernel's high-water mark. The check is
  short, so a sample may be absent; the budget assertion runs when one was taken.
