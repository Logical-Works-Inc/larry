---
type: ADR
title: "ADR 0089: a confined macOS command cannot ask another process to act"
description: Why the macOS Seatbelt profile keeps (allow default) and adds denies for mach-lookup, signal outside the command's own process group, and process-exec of open, launchctl and osascript.
resource: /docs/adr/0089-a-confined-macos-command-cannot-ask-another-process.md
tags: [larry, adr, confinement, security, executor, macos]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: the three channel simulations on the Director's Mac, failing on main and green after, 2026-11-21}
status: accepted
stale_after: 2027-05-21
sources: [larry#326, larry#321, /INVARIANTS.md, ADR 0046, ADR 0081, Apple sandbox profile language (`sandbox-exec(1)`), CWE-269]
---

# 0089: a confined macOS command cannot ask another process to act

Status: accepted, 2026-11-21 (larry#326).

## Context

The macOS confinement of a command is a Seatbelt profile starting
`(version 1)(allow default)` (larry#243, ADR 0046). The profile denied direct
writes outside the workspace, the person's `/dev/tty` and the kernel interface
that returns another process's arguments and environment. Everything else was
inherited, and three channels stayed open:

- The command could exec `/usr/bin/open`, `/bin/launchctl` or
  `/usr/bin/osascript` and ask a system service to start or drive a process
  outside the profile, with the person's own authority.
- The command could look up any Mach launch service (`mach-lookup`), so it
  could ask a daemon to act for it.
- The command could signal any process of the same user (`signal`), not only
  its own children.

Writes were refused; these three routes were not, so "the kernel confines the
command" held only for the filesystem.

## Decision

The profile keeps `(allow default)` and adds three rules, each naming the
operation of Apple's sandbox profile language (`sandbox-exec`):

- `(deny mach-lookup)` denies a lookup of every launch service. The
  allowlist is empty, and that is the measured answer: the shell verbs the PTY
  journeys run (`printf`, `sleep`, `kill`) and `git` resolve no launch service,
  so no service is left open on an assumption. A future journey that needs one
  fails as a typed request, and the service is added with its own measurement.
- `(deny signal (require-not (require-any (target self) (target pgrp))))` lets
  the command signal itself and its own process group, and refuses every
  process outside it. A shell still ends its own children; a command cannot
  reach the person's processes.
- `(deny process-exec (literal "/usr/bin/open") (literal "/bin/launchctl")
  (literal "/usr/bin/osascript"))` refuses the three launchers. `literal` names
  each path the kernel resolves, and a path that is a copy of one of them is
  not covered: the profile names programs, not their effects.

The Linux path is unchanged: Landlock grants filesystem rights and the wrapper
`setsid` already gives each command its own session.

## Consequences

- The three channels are refused by the kernel, and the folder beside the tree
  stays byte-identical through each attempt. `src/executor/sim_confine_macos.rs`'s
  three macOS simulations prove each channel; each failed on main.
- The PTY journeys stay green on macOS: `cargo nextest run -p larry-demo
  --test e2e_journey` passes with the new profile.
- The added cost is the profile text and three kernel rules. The wrapper
  measures p50 6 ms per command before and p50 6 ms after (p95 8 ms before,
  6 ms after; p99 9 ms before, 6 ms after), 30 samples each, by
  `bench_confine_overhead_is_measured_against_the_unconfined_spawn` on this
  Mac. The spread is host noise; no sample grew.
- A command that only writes its tree behaves as before, so no existing
  journey changes its shape.

## Rejected

- Start from `(deny default)` with an allowlist. It would have to rebuild the
  read grant over the whole host, the write grant, the network grant ADR 0046
  deliberately leaves open, and every process and system call a shell needs.
  The PTY journeys would then rest on an allowlist no measurement covers, and
  one missing entry would break a user's journey for no boundary gained: the
  three channels are closed by the denials above.
- Deny `launchctl` only. `open` and `osascript` start or drive processes the
  same way, and a command that reached any one of them left the profile.
- Allow the Mach services a shell "usually" needs. None was needed by the
  measured journeys, so an allowlist would have opened services for no proven
  need; an allowlist is added with the journey that needs it.

Reversal: Nova's sandboxed execution (logicalworks-crates#846) replaces the
Seatbelt profile for these commands; the channels it must close stay.
