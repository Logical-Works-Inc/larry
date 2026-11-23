---
type: ADR
title: "ADR 0092: the Linux read walk takes the estate's 4,096-rule bound and grants files"
description: Why a state directory under a crowded home folder now confines, and why a file beside it is a read rule of its own.
resource: /docs/adr/0092-the-linux-read-walk-takes-the-estate-bound-and-grants-files.md
tags: [larry, adr, confinement, security, landlock]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: sim_confine_a_crowded_home_confines_and_its_files_stay_readable failing on main then green, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#382, larry#322, ADR 0081, logicalworks-crates#1114, Linux landlock(7), util-linux setpriv(1)]
---

# 0092: the Linux read walk takes the estate's bound and grants files

Status: accepted, 2026-11-23 (larry#382).

## Context

ADR 0081 grants a Linux command every sibling on the walk from `/` to the
state directory, because Landlock is an allowlist and cannot take the state
directory out of a granted parent. The walk had two faults.

- The bound was 59: the estate's `LandlockRules` held 64 rules. A state
  directory under a home folder with 500 entries was refused with
  `TooManyRoots`, so bare `larry`, a briefed `larry exec` and every
  `larry_submit_task` refused on a common Linux layout.
- Only directories were granted. A file beside the state directory, such as
  `~/.gitconfig`, was unreadable, so `git` in the copy failed when the state
  directory was under the home folder.

## Decision

- `MAX_READ_ROOTS` is `LandlockRules::MAX_RULES` (4,096 since lgwks_bot
  10.0.0, crates#1114) less the five fixed rules. Larry keeps no bound of
  its own.
- A regular file beside the walk is a read root. Its rule carries the file
  rights only (`execute,read-file`), because Landlock refuses a directory
  right on a file. A device, FIFO or socket is still no root.
- The estate's `validate` still refuses a list past 4,096 rules or 1 MiB of
  path bytes, so the argv stays under `ARG_MAX`.

## Rejected

- Move the key store out of the state directory, so one deny holds it. It
  migrates every state directory, and the session's copy beneath the state
  directory must stay readable anyway.
- `Confinement::DenyBeneath` (bwrap, a mount namespace). It cannot keep the
  copy beneath the state directory readable, and the estate spawns it, while
  Larry spawns through std (crates#1075).

## Consequences

- A home folder with 500 entries confines in 503 rules (measured in the sim).
- Test: `executor::confine::sim::sim_confine_a_crowded_home_confines_and_its_files_stay_readable`.

Reversal: an estate call that denies one subtree and keeps a path beneath it
open replaces the walk. Measure with this sim and the kernel read checks.
