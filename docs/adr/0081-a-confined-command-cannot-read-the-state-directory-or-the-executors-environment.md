---
type: ADR
title: "ADR 0081: a confined command cannot read the state directory or the executor's environment"
description: Why the Seatbelt profile and the Landlock allowlist deny the state directory and the executor's process entry, and why the key stays in the environment.
resource: /docs/adr/0081-a-confined-command-cannot-read-the-state-directory-or-the-executors-environment.md
tags: [larry, adr, confinement, security, state]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: the confine simulations on macOS and Linux, failing on main then green, 2026-11-21}
status: accepted
stale_after: 2027-05-21
sources: [larry#322, /INVARIANTS.md, Apple sandbox-exec(1), Linux landlock(7), util-linux setpriv(1)]
---

# 0081: a confined command cannot read the state directory or the executor's environment

Status: accepted, 2026-11-21 (larry#322).

## Context

Bare `larry` runs each model command in a kernel-confined copy of the
person's folder (ADR 0046). The copy is `<state>/ws/<task>/tree`, so the
state directory is an ancestor of every command's working directory, and it
holds `executor.db`, whose key row is the receipt signing seed. The macOS
profile was `(allow default)` with a write deny only, and the Linux
allowlist granted read beneath `/`. Either way a command could `cat` the
store, recover the seed and seal receipts `larry verify` accepts.

The executor's own environment also holds `LARRY_MODEL_KEY` and every
`LARRY_KEY_*`. `env_clear` on the child hides them from the child's
environment, but a same-user process reads the parent's through
`/proc/<pid>/environ` on Linux and `KERN_PROCARGS2` (`ps e`) on macOS.

## Decision

- macOS: the profile keeps `(allow default)` and adds
  `(deny file-read* (require-all (subpath "<state>") (require-not
  (require-any (subpath "<copy>") (subpath "<tree>") (subpath
  "<scratch>")))))`, so the store and the other workspaces are unreadable
  while the command's own copy stays readable. It also adds
  `(deny sysctl-read (sysctl-name "kern.procargs2"))`, the interface that
  returns a process's arguments and environment.
- Linux: the read root is no longer `/`. The roots walk from `/` down to
  the state directory's parent: at each level every child but the one on
  the path to the state directory, so a sibling stays readable and the
  state directory and everything under it never is (an allowlist cannot
  subtract a child of a granted path). `/proc` is not a root; only
  `/proc/self` and `/proc/sys` are granted. The allowlist holds 64 rules
  and the tree, the scratch directory, the device and the two `/proc`
  entries take five, so `MAX_READ_ROOTS` is 59 and a path that needs more
  is refused with `ConfineError::TooManyRoots` naming both counts. A path
  is never a root when its resolved form is the state directory, beneath
  it or an ancestor of it, so a sibling link into the state grants nothing,
  and a state directory that is the walk's own root is refused
  (`StateAtRoot`) rather than built into a confinement that reads nothing.
  The copy is a read rule of its own, and the tree and scratch directory
  keep their read-write rules, which carry read as well.
- The copy is `<state>/ws/<task>`, the tree's parent: the clone names its
  git directory there (`--separate-git-dir`) so a command cannot commit,
  and every `git` command in the tree reads its way to it. Without the
  copy's rule the whole copy failed to resolve as a repository, which the
  e2e suites caught as a refused `git commit` with the wrong reason.
- The key is still read once at start into the typed settings
  (`process_env`'s snapshot). It is not removed from the process
  environment: Rust 2024 makes `std::env::remove_var` `unsafe`, and the
  workspace forbids `unsafe`. The kernel keeps it from the command instead,
  which is the stronger place: neither `/proc/<pid>/environ` nor
  `KERN_PROCARGS2` reports a removal done inside a live process anyway.
- `Confine::new` takes the state directory beside the tree and the scratch
  directory, and resolves them, because both profiles match the path
  the kernel resolves (`/tmp` is `/private/tmp` on macOS). It refuses a
  tree whose copy is not beneath the state directory, since the exception
  would then open the store itself.

## Consequences

- A confined command can read neither the signing seed nor another
  workspace, and it cannot read the environment of the executor that
  spawned it, on both kernels. Its own copy stays readable, so `git
  status`, `git log` and `git diff` work in the tree and `git commit` is
  still refused: the git directory is outside the writable tree.
- Every other read stays open, a state directory under `/home` included:
  `~/.cargo` and `~/.rustup` remain readable, so a confined `cargo test`
  works. Before the walk, closing the state directory's top component made
  all of `/home` unreadable and the session refused no command it should
  run but failed the ones it should.
- On Linux a command loses every `/proc` entry but its own and the host's
  tunables: `ps` cannot list other processes. On macOS `ps e` reports no
  environment. Every other read and the network stay open (ADR 0046).
- Both spellings are pinned by `sim_confine_argv_per_platform_is_exact`,
  and the kernel itself is the oracle in
  `sim_confine_kernel_refuses_the_state_directory_for_every_seed` and
  `sim_confine_kernel_hides_the_executors_environment`.

## Rejected

- Closing the top-level directory that holds the state directory. With
  `LARRY_STATE_DIR` under `/home/<user>` it made all of `/home`
  unreadable, `~/.cargo` and `~/.rustup` included, so a confined
  `cargo test` failed in every Linux session; CI passed only because its
  state directory sits in `/tmp`.
- Moving the workspace out from under the state directory. It changes
  `Workspace::create`, the sweep, recovery and the journal path for one
  read rule, and the state directory would still hold keys a command can
  reach by absolute path.
- A Landlock allowlist that names every sibling of every ancestor of the
  state. The estate's `LandlockRules` documents the same dead end; its
  `Confinement::deny_beneath` fills it with a mount-namespace overmount,
  which is a second wrapper beside the `setpriv` spelling Larry ships.
- Keeping the model key out of the process environment with a handoff
  through a pipe or a file. A pipe needs a non-CLOEXEC descriptor, which
  safe std cannot make, and a file puts the key on disk, which the route
  rule already forbids.

Reversal: a safe way to scrub a live process's environment, or an estate
helper for it, lets the key leave the environment and the `/proc`
exclusion drop.
