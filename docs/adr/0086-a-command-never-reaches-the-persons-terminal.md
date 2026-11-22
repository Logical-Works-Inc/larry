---
type: ADR
title: "ADR 0086: a command never reaches the person's controlling terminal"
description: Why each command starts in a session of its own on Linux and why the macOS profile refuses /dev/tty for reading and writing.
resource: /docs/adr/0086-a-command-never-reaches-the-persons-terminal.md
tags: [larry, adr, confinement, security, terminal]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: the terminal journeys on macOS and Linux, failing on main then green, 2026-11-21}
status: accepted
stale_after: 2027-05-21
sources: [larry#323, /INVARIANTS.md, util-linux setsid(1), POSIX setsid(2), Apple sandbox-exec(1), Linux landlock(7)]
---

# 0086: a command never reaches the person's controlling terminal

Status: accepted, 2026-11-21 (larry#323).

## Context

Bare `larry` draws the question, the diff and the answer on the person's
terminal, and reads their keys from it. A command a person approves ran in
the person's own session, so `/dev/tty` named that terminal: a command could
paint bytes past the sanitizer, as a fake approval or a fake diff, and read
the keys the person typed. The session's pane sanitizes what it draws, but a
command writing the device directly never passes through the pane. On macOS
the profile's write allowlist named `/dev/tty`, so the write was granted; on
Linux Landlock refused the write but the session was still shared.

## Decision

- Linux: each command starts through util-linux `setsid`
  (`/usr/bin/setsid`), so it runs in a session of its own with no
  controlling terminal, and an open of `/dev/tty` is refused `ENXIO`. The
  wrapper comes first: `setsid setpriv … <command>`, and every exec keeps
  the session. util-linux `setsid` execs in place when the caller is not a
  process-group leader, so the spawned pid stays the session leader and the
  process-group leader the ledger records and `killpg` names.
  `process_group(0)` is not set when `setsid` is used: it would make the
  child a group leader, `setsid` would then fork, and the recorded pid would
  no longer be the group's.
- macOS: no `setsid` ships, so the profile refuses the device instead. The
  write allowlist names `/dev/null` alone, and
  `(deny file-read* (literal "/dev/tty"))` refuses the read. Linux already
  allowed writes only beneath the tree and the scratch directory.
- The session tool is required, not optional, on Linux: `/usr/bin/setsid`
  arrives with the same util-linux package that `/usr/bin/setpriv` does, and
  a host without `setpriv` already refuses confinement and starts nothing
  (`ConfineError::NoWrapper`).
- A host with no `setsid` (macOS) keeps the process group of its own through
  `process_group(0)`, so the ledger and the kill paths are unchanged.

## Consequences

- A command can neither paint on the person's screen nor read their keys
  through `/dev/tty`, on both kernels.
- A command that wants a terminal now fails and says so in its own stderr,
  which the `Ran` entry draws like any other output; stdout, stderr and
  stdin capture are unchanged.
- The ledger's `pgid` is still the spawned pid, so the timeout kill, the
  cancel kill and the orphan reap name the whole group as before; the
  `executor_journey` group tests stay green.
- Both launch argv spellings are pinned by
  `sim_a_command_starts_in_a_session_of_its_own` and
  `sim_confine_argv_per_platform_is_exact`; the journeys
  `tui_journey_a_command_never_writes_the_persons_terminal` and
  `tui_journey_a_command_never_opens_the_persons_terminal` drive the
  shipped binary in a PTY, and the first runs the same journey with and
  without the write and compares the screen.

## Rejected

- `setsid(2)` in `pre_exec`. It is `unsafe`, and the workspace forbids
  `unsafe` in product code.
- `process_group(0)` alone. It makes a process group in the same session;
  the controlling terminal stays reachable, which the journey showed.
- Denying `/dev/tty` in Landlock on Linux instead of a session. A session is
  what POSIX uses to detach from a terminal, and the estate has no Landlock
  file-read deny for a single device path; the kernel's own `ENXIO` is the
  stronger answer.
- A pseudo-terminal per command. It changes how output is captured and
  sanitized for a terminal the person never needs to see.

Reversal: a safe std or estate call that starts a session lets the wrapper
drop; on macOS the profile rule drops when Apple ships `setsid`.
