---
type: ADR
title: "ADR 0093: a task names at least one acceptance command, and no report is accepted over none"
description: Why an empty acceptance list is refused at parse and why the derived verdict needs at least one command result.
resource: /docs/adr/0093-a-task-names-at-least-one-acceptance-command.md
tags: [larry, adr, receipt, acceptance]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: exec_refuses_a_task_with_no_acceptance_command_and_seals_nothing failing on main then green, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#395, larry#321, ADR 0006, ADR 0038]
---

# 0093: a task names at least one acceptance command

Status: accepted, 2026-11-23 (larry#395).

## Context

ADR 0006 derives `accepted` from the report: no denial, a whole diff, and every command exit 0
in time. "Every" over an empty list is true. `larry exec` took `"acceptance": []`, so a task
with a brief sealed `accepted: true` over whatever the model wrote, and `larry verify` agreed.
The MCP door (`minItems: 1`) and eval sets (`AcceptanceCount`) already refused zero.

## Decision

- `Task::parse` refuses a list of 0 or more than 16 commands with one typed
  `TaskError::CommandCount` that names the bound. It replaces `TooManyCommands`.
- The derivation needs a non-empty `results` array. `larry verify` derives again, so a receipt
  that claims accepted over zero commands is refused as `Inconsistent`.

Rejected: refuse at parse only. A receipt sealed before this change, or a body from a later
door that skips the parser, would still verify accepted over nothing. The derivation is the
one owner of the verdict (ADR 0006), so the rule lives there too.

## Consequences

- A receipt sealed before this change with no command result no longer verifies. Such a report
  checked nothing, so refusing it is correct.
- A test or caller that needs a run with nothing to check runs `true`.
- Falsifiers: `tests/receipt_journey.rs`
  (`exec_refuses_a_task_with_no_acceptance_command_and_seals_nothing`) and
  `executor::receipt::tests::sim_a_report_with_no_command_is_never_accepted_and_verify_agrees`.
