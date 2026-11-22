---
type: ADR
title: "ADR 0088: an acceptance command runs confined once a model wrote the tree"
description: Why a briefed task's acceptance commands and every MCP task run behind the session's kernel confinement, and why a no-brief larry exec task keeps today's open place.
resource: /docs/adr/0088-acceptance-runs-confined-after-a-model-writes-the-tree.md
tags: [larry, adr, confinement, security, executor, mcp]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: the two journeys on the Oracle box, failing on main and green after, 2026-11-21}
status: accepted
stale_after: 2027-05-21
sources: [larry#324, larry#243, /INVARIANTS.md, ADR 0046, ADR 0081, OWASP ASVS 4.0.3 §4.2.1, CWE-59]
---

# 0088: an acceptance command runs confined once a model wrote the tree

Status: accepted, 2026-11-21 (larry#324).

## Context

`larry exec` and `larry_submit_task` run a task in three steps: clone the
repository at the base into `<state>/ws/<task>/tree`, let the model write each
file of the brief into that tree, then run each acceptance command in the tree.
Before this decision every acceptance command ran with `Place::Open`, the
tree as it is, with the user's whole filesystem and network reachable. A brief
means the model chose the content of the tree, so an acceptance such as a
build, a test runner or a package script executes model-written code with no
kernel boundary; `larry_submit_task` turns each acceptance string into
`sh -c`, so an MCP peer ran any shell text it named, unconfined, with no person
asked. A terminal session already runs each command behind `Confine`
(larry#243, ADR 0046), so the executor already owned the mechanism.

## Decision

- `executor::run_task` builds one `Confine` over the task's workspace
  (`<state>/ws/<task>/tree`, its scratch directory beside the tree, and the
  state directory it must not read) and runs every acceptance command with
  `Place::Confined`. The command's argv is the platform wrapper plus the
  task's own argv, so the kernel confines it and every process it forks.
- The confinement applies whenever the task carries a brief: the model wrote
  the tree the command executes. It applies to every `larry_submit_task` task
  as well, through `Task::confining_acceptance`, because a remote peer drove
  the run and no person watches the command. A task with no brief through
  `larry exec` keeps `Place::Open`: no model wrote anything, the caller's own
  command is the only one in the loop, and its documented place does not
  change.
- A host that cannot confine refuses the task with the typed
  `ExecError::Confine` the terminal session already uses. The check runs
  before the route, the clone and every command, so nothing is cloned or run
  on such a host.
- After the clone, the host proves it enforces the confinement with
  `Confine::probe` before the first acceptance command. A wrapper that is
  missing, refuses to run or lets a write beside the tree land is the same
  typed refusal; no command runs bare.

## Consequences

- An acceptance command that writes outside the tree is refused by the kernel.
  Its exit code is non-zero, so the run seals as rejected: the report is the
  evidence a patch did not pass, never a silent success (larry#86,
  forge#158).
- The origin stays byte-identical: the task still clones, and the confinement
  now covers the model-written tree the commands execute.
- `larry eval` tasks carry a brief and are confined too; `route` journeys and
  the MCP accepted-task journey run under the same wrapper and stay green.
- The added work per briefed task is one `Confine::new` (the platform argv and
  the Linux read-root walk) and one probe shell. The confinement journey
  prints the briefed path's p50/p99 over 32 debug runs, and the no-brief
  `larry exec` path's own 32-run figure is unchanged.
- The state directory stays unreadable to the command (larry#322, ADR 0081),
  and the command's own copy under it stays readable, so `git` resolves in the
  clone.

## Rejected

- Confine only MCP tasks. `larry exec` with a brief has the same
  model-written tree, and its acceptance would still execute unconfined.
- Confine every task, including a no-brief `larry exec` task. That narrows a
  documented place for the caller's own command, where no model is in the
  loop, and is not what the defect names.
- Wait for Nova's fuller sandbox (logicalworks-crates#846). The session's
  kernel confinement already exists and closes the named hole now; Nova
  replaces the mechanism, not this boundary.
- Run the acceptance before the model writes. The acceptance is what proves
  the model's change, so it must run after it.

Reversal: Nova's sandboxed execution replaces `Confine` for these tasks; the
decision to confine a briefed or remote task stays.
