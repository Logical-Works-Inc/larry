---
type: ADR
title: "ADR 0111: A task names its shell, base, timeout and deadline under its tenant's ceilings"
description: Why a task carries its own shell, base, per-command timeout and whole-task deadline, and why each is bounded by a ceiling the tenant's executor settings hold.
resource: /docs/adr/0111-task-ceilings.md
tags: [larry, adr, executor, settings, tenant, deadline]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#439, larry#213, larry#200, larry#436, larry#442]
---

# 0111: A task names its shell, base, timeout and deadline under its tenant's ceilings

Status: accepted, 2026-11-24 (larry#439). Second half of larry#213; extends ADR 0106.

## Context

- A planner door (`larry_submit_task`, `larry eval`) fixed `sh -c` and `base: HEAD`. A task
  could not pick its shell or the commit it checks out.
- One command timeout (`LARRY_EXEC_TIMEOUT_MS`) held for the whole process, and a task had no
  deadline: a task of 16 commands could run 16 times that timeout.
- A tenant is its state directory and the settings its executor reads. No setting bounded what
  a tenant's tasks may name.

## Decision

- A task document may name `shell` (1 to 256 bytes, no whitespace, control or leading `-`),
  `timeout_ms` and `deadline_ms` (each 1..=86400000). A named shell runs each acceptance
  command as `<shell> -c <argv...>`. The MCP tool takes the same four, plus `base`.
- `executor::ceiling::Ceilings`, a field of the one `Settings`, holds the ceilings:
  `LARRY_EXEC_MAX_TIMEOUT_MS`, `LARRY_EXEC_MAX_DEADLINE_MS` (each unset: one day),
  `LARRY_EXEC_SHELLS` (unset: `sh`, at most 16) and `LARRY_EXEC_BASES` (unset: every base; at
  most 64, each exact or a `*` prefix). A bad value is `ExecError::BadCeiling` naming the
  variable. An explicit `LARRY_EXEC_TIMEOUT_MS` past the timeout ceiling is refused too.
- `execute` reads the ceilings before `prepare`. A task past one is `ExecError::PastCeiling`
  naming shell, base, timeout_ms or deadline_ms: nothing is cloned, spawned or written.
- A task's deadline starts on the run's one live clock. Each command's timeout is cut to what
  is left. A deadline spent before the clone refuses `ExecError::DeadlinePassed` typed with
  nothing cloned, because nothing ran; a deadline spent after it seals a rejected report keeping
  every result so far (the cut command's, with `timed_out: true`), the diff as it stands and
  `deadline_passed: {"deadline_ms": N}` beside the results, exit 2. `accepted` derives false
  whenever that member is present, and `larry verify` re-derives it, so a receipt claiming
  accepted beside it is `Inconsistent`.

Rejected: one deadline for the whole process in `LARRY_EXEC_DEADLINE_MS`. Two tenants with
different needs could not share a host, and a planner could not ask for less. Rejected too: a
ceiling table in `executor.db`. The settings home is the process environment snapshot (ADR
0107); a second home for settings drifts. A tenant that needs ceilings changed at run time with
no restart reverses this: measure how often an operator restarts to change one.

## Consequences

- `tests/executor_journey/ceiling.rs` runs the shipped binary: two tenants, one admitted and
  one refused; a refused shell, base and deadline with 0 workspaces; a 1.5 s deadline over a
  30 s command seals a rejected report with `deadline_passed`, `accepted: false`, exit 2, one
  `timed_out: true` result, the group reaped and the second command never run; verify re-derives
  the rejection. `executor::receipt::deadline_tests` refuses a forged `accepted: true` beside a
  spent deadline as `Inconsistent`.
- `executor::sim_ceiling` proves every boundary and the deadline cut on a virtual clock.
