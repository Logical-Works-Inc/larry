---
type: ADR
title: "ADR 0035: Larry runs Terminal-Bench 2.0 through Harbor as an ACP agent"
description: Why `larry acp` speaks the Agent Client Protocol on stdio, why a terminal session needs the new model.command scope no default policy holds, and what its receipt does and does not claim.
resource: /docs/adr/0035-larry-as-a-harbor-agent-over-acp.md
tags: [larry, adr, evaluation, acp, harbor, executor]
generated: {by: claude-code, at: 2026-11-11}
verified: {by: claude-code, method: cargo nextest run --workspace --locked, tests/acp_harbor_journey.rs against the shipped binary}
status: accepted
stale_after: 2027-02-11
sources: [larry#93, larry#164, larry#75, larry#86, agentclientprotocol.com protocol v1, harborframework.com agents and tasks]
---

# 0035: Larry runs Terminal-Bench 2.0 through Harbor as an ACP agent

Status: accepted, 2026-11-11 (larry#93).

## Context

Terminal-Bench 2.0 runs through Harbor, whose installed agents (Claude Code, Codex CLI, OpenHands) are Python
classes. Python is banned in product repositories (2026-10-13). Harbor also ships a generic `acp` agent that runs
any Agent Client Protocol agent: newline-delimited JSON-RPC 2.0 on stdio, `initialize`, `session/new` with an
absolute `cwd`, `session/prompt` streaming `session/update` notifications to one `stopReason`, and
`session/cancel`. A task is `instruction.md`, `task.toml`, an `environment/` image and a `tests/test.sh`
verifier that writes `/logs/verifier/reward.txt`. Until now Larry's executor denied every model-chosen command
(INVARIANTS: "model commands stay denied until Nova").

## Decision

- `larry acp` is an ACP v1 agent in the default binary. It reuses `larry mcp`'s framing, its 1 MiB frame bound
  and its JSON-RPC envelope checks, now in one owner, `mcp::wire`. A reader thread keeps reading while a prompt
  runs, so a cancel lands mid-command. It clears and sets each session's cancel flag in stream order.
- A prompt runs `executor::run_session`. Each model reply is one JSON object: `{"command": ...}` runs under
  `sh -c` in the session directory, and `{"done": ...}` ends the turn. Each command is the executor's own
  `run_command`: its own process group, the executor timeout, a ledger row the next start reaps. The session
  stops at 48 commands, at three refused replies in a row, at a gateway refusal, or at a cancel checked before
  each model call and each command.
- A session is one displayed `WorkProposal` through the executor gate (ADR 0019). It needs every executor scope,
  plus the new `model.command`, which `DEFAULT_POLICY` never holds. An operator names it in `LARRY_POLICY` only
  where something outside Larry isolates the commands, such as Harbor's task container. A briefed task's model
  still never runs a command, and Nova (logicalworks-crates#846) remains the in-Larry sandbox.
- The sealed report records each command as the model saw it (the first 4 KiB of each stream). It carries no
  diff, so `larry verify` checks its seal and reports it unaccepted (exit 2). The reward is the task verifier's,
  never Larry's.

## Consequences

- Harbor's `acp` agent installs an agent from the ACP Registry by its binary distribution and passes settings
  with `--agent-env`: here `LARRY_POLICY` naming `model.command`, `LARRY_STATE_DIR` and the gateway settings
  (docs/evaluation/README.md). No Python is involved. Listing Larry in the registry publishes it, so that waits
  for the Director to name it (AGENTS.md: nothing is published without the Director).
- `tests/acp_harbor_journey.rs` runs a Harbor-shaped task through the shipped binary and its verifier against
  the seeded mock model. It also covers refusal, the step bound, cancellation, kill -9 recovery and two tenants.
  No paid model call. A full Terminal-Bench run spends model money and is larry#95.
