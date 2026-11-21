---
type: ADR
title: "ADR 0071: an ACP cancel binds to its prompt, and serve survives a failed reload print"
description: Why a gone ACP client abandons every live session, a queued prompt never clears a running cancel, a failed reload line does not stop serve, and argv past 32 is refused.
resource: /docs/adr/0071-acp-cancel-binds-to-its-prompt-and-serve-survives-a-failed-reload-print.md
tags: [larry, adr, acp, serve, cli]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#338 cases on Ubuntu, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#338, /INVARIANTS.md, https://agentclientprotocol.com/protocol/prompt-turn]
---

# 0071: an ACP cancel binds to its prompt, and serve survives a failed reload print

Status: accepted, 2026-11-21 (larry#338).

## Context

The ACP session entry held one cancel bool. The reader cleared it on every
`session/prompt`, so a second prompt queued behind a running one erased the
cancel meant for the running one. End of input and a failed write to the client
cleared nothing, so a client that died left up to 256 model calls and commands
running. `larry serve` returned a SIGHUP reload line's write error, which stopped
the watcher and the server when the log pipe was closed. `larry` read at most 32
arguments with `take(32)` and silently dropped the rest.

## Decision

- The reader numbers each `session/prompt` per session and records a
  `session/cancel` against the number of the prompt it follows, in a bounded set.
  The agent gives the prompt it starts the same number and clears the decisions
  of prompts that ended.
- End of input, a read fault and a write fault mark every live session abandoned,
  so every running prompt stops at its next check.
- A reload line that cannot be written is reported on stderr and the signal
  watcher keeps serving.
- More than 32 arguments is a typed `Refusal::TooMany` naming the bound, never a
  truncation.

## Consequences

- A queued prompt cannot clear a running prompt's cancel; a client that dies
  stops its session; a closed log pipe leaves `larry serve` serving; a command
  line past the bound is refused before anything runs.
- Tests: `a_client_that_dies_mid_prompt_stops_the_session`,
  `a_queued_prompt_does_not_clear_the_running_prompts_cancel`,
  `a_reload_whose_log_line_fails_keeps_serving_and_reports_on_stderr`,
  `more_arguments_than_the_bound_is_refused`.

## Rejected

- One cancel bool cleared per prompt frame. That is the defect: a queued prompt
  erases the running prompt's cancel.
- `take(32)`. A bound that drops input is no bound.
- Stop serve when the reload line cannot be written. The server's job does not
  depend on its log pipe.

Reversal: if ACP gains a per-prompt cancel identity, the numbered set becomes
that identity and this ADR reopens with the protocol change.
