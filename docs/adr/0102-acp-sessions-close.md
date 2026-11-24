---
type: ADR
title: "ADR 0102: Larry's ACP sessions close and free their slot"
description: Why larry acp serves session/close, so the 16-session bound counts open sessions only, and why an abandoned session keeps its slot.
resource: /docs/adr/0102-acp-sessions-close.md
tags: [larry, adr, acp, sessions]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: sim_acp_closed_sessions_free_their_slots_past_the_bound red then green, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#411, larry#321, larry#338, larry#93, "https://agentclientprotocol.com/protocol/v1/session-setup", "https://agentclientprotocol.com/announcements/session-close-stabilized"]
---

# 0102: ACP sessions close and free their slot

Status: accepted, 2026-11-24 (larry#411). Extends ADR 0035.

## Context

`larry acp` held at most 16 sessions, and nothing took one out of the book. An editor or
Harbor that keeps one process and opens 17 chats got the bound error until a restart. ACP v1
stabilized `session/close` on 2026-04-23: the agent stops the session's work as
`session/cancel` would, frees it, and answers an empty result, also for an unknown or closed id.

## Decision

- Advertise `agentCapabilities.sessionCapabilities.close` as `{}`.
- The reader thread marks a `session/close` as it reads the frame, so the running prompt and
  every queued prompt of that session stop as `cancelled`, and logs one acknowledgement line.
- The agent answers the close by removing the session from the book, with `{}`. A close with
  no `sessionId` string is -32602.
- An abandoned session keeps its slot. The reader abandons every session at end of input while
  frames sent before it still wait for the agent, so a freed slot would answer those frames by
  thread timing, not stream order: the sim caught a 17th session admitted that way. A gone
  client sends nothing more, and the book goes when `serve` returns.
- The bound stays 16 open sessions.

Rejected: evict the oldest idle session when a 17th opens. It ends a chat the client still
holds without a word to it. Rejected: raise the bound. It only delays the same lock. Reverse it
if a client is seen to rely on a closed session id coming back with `session/load`.

## Consequences

- Session ids still count up and are never reused, so a stale close never hits a new session.
- Falsifiers: `sim_acp_closed_sessions_free_their_slots_past_the_bound`,
  `sim_acp_a_close_stops_the_running_prompt_and_every_queued_one`,
  `sim_acp_an_abandoned_session_keeps_its_slot_in_stream_order`.
