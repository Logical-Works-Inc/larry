---
type: ADR
title: "ADR 0091: a Linux confinement needs setsid, or it refuses"
description: Why setsid is the first word of the Linux wrapper and a host without it is refused typed, instead of running a command that keeps the person's terminal.
resource: /docs/adr/0091-a-linux-command-needs-setsid-for-its-own-session.md
tags: [larry, adr, confinement, security, terminal]
generated: {by: claude-code, at: 2026-11-22}
verified: {by: claude-code, method: sim_confine_linux_needs_setsid_and_starts_its_own_session failing on main then green, 2026-11-22}
status: accepted
stale_after: 2027-05-22
sources: [larry#386, larry#323, ADR 0086, util-linux setsid(1), POSIX setsid(2), Linux landlock(7)]
---

# 0091: a Linux confinement needs setsid, or it refuses

Status: accepted, 2026-11-22 (larry#386).

## Context

ADR 0086 says the session tool is required on Linux. The code did not hold
it: `command_for` put `/usr/bin/setsid` in front of a command only when that
file existed. A Linux host with `setpriv` but no `setsid` ran the confined
command in the person's own session. Landlock grants reads under `/dev`,
which is a sibling on the walk to the state directory, so the command could
open `/dev/tty` and read the person's keys.

## Decision

- `setsid` is part of the Linux wrapper. `Platform::Linux` needs
  `/usr/bin/setsid` and `/usr/bin/setpriv`, in that launch order. The
  Landlock prefix starts `setsid setpriv --landlock-access fs …`.
- `supported` refuses the first tool that is missing with
  `ConfineError::MissingWrapper`, so a host without `setsid` refuses the
  session typed and starts nothing, as a host without `setpriv` does.
- `command_for` takes the session from the wrapper. It adds the host's
  `setsid` only to an open command. A command is launched through exactly
  one `setsid`: a second would find a session leader, fork, and the executor
  would wait on a parent that already left.
- `process_group(0)` is set only when the first word is not `setsid`, as
  before.

## Rejected

- `setsid(2)` in `CommandExt::pre_exec` through `rustix`. `pre_exec` is
  `unsafe`, and the workspace forbids `unsafe_code`.
- A hidden `larry` subcommand that calls `setsid(2)` and execs the command.
  It is a second entry point that trusts `current_exe`, to replace a tool
  that util-linux ships in the same package as `setpriv`.
- Keeping the optional `setsid` and denying `/dev/tty` in Landlock. Landlock
  is an allowlist and cannot subtract one device from a granted `/dev`.

## Consequences

- No Linux command runs with the person's controlling terminal. A host
  without `setsid` is refused before the model is called.
- The probe runs through the same wrapper, so it proves `setsid` runs too.
- Test: `executor::confine::sim::session::sim_confine_linux_needs_setsid_and_starts_its_own_session`
  hides `setsid` through the injected tool check, never the host's `/usr/bin`.

Reversal: a safe std or estate call that starts a session in the child lets
the tool go. Measure with this test and the two `terminal_cases` journeys.
