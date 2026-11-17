---
type: ADR
title: "ADR 0044: a terminal session compacts its own conversation, with no model call and no operator"
description: Why a session clears old output and folds old steps before each call, learns the model's window from its own refusal numbers, and runs 256 steps instead of 48.
resource: /docs/adr/0044-session-auto-compaction.md
tags: [larry, adr, executor, session, acp, compaction, cache]
generated: {by: claude-code, at: 2026-11-15}
verified: {by: claude-code, method: src/executor/sim_compact.rs, src/executor/sim_refusal.rs and src/model/sim_wire.rs run on Oracle, 2026-11-16}
status: accepted
stale_after: 2027-02-15
sources: [larry#235, larry#236, larry#93, docs/adr/0035-larry-as-a-harbor-agent-over-acp.md, docs/adr/0043-one-prompt-prefix-per-brief.md]
---

# 0044: a terminal session compacts its own conversation

Status: accepted, 2026-11-15 (larry#235)

## Context

The Director, 2026-11-15: "Auto compact being truly auto is a goal of mine." A terminal session kept
every reply and every observation. The only bound was 48 steps, chosen so the longest conversation fit
the route's 1 MiB request bound. Two failures followed. A long Terminal-Bench task ended
`max_turn_requests`. A model whose window is smaller than its route (128k tokens is about 512 KB)
answered 400 or 413, and the session ended as a refusal.

Claude Code compacts by asking the model for a summary. That is one more paid call, it can fail, it is
not replayable from a seed, and it runs the model over hostile command output a second time. Microsoft
Agent Framework offers `ToolResultCompactionStrategy` (no model) and a summarizing strategy (a model).

## Decision

- Before each model call the session measures the encoded request with the client's own encoder.
  Past 3/4 of the window it compacts toward 1/2. No model call is made.
- Stage 1: clear the output of every observation but the newest 4. The status line stays, and the
  turn says to run the command again to see it.
- Stage 2: fold the oldest steps into one record turn, one line per step (`step N: <command> ->
  <status>`, 160 bytes at most). The newest step is never folded. Past 120 unfolded steps it folds
  down to 96, so a request stays under the client's 256 messages and the fold runs once in 24 steps.
- The request is encoded once per step: the session measures those bytes and sends the same bytes.
- Stage 3: if still over, clear the last 4 too, then drop the oldest record lines. The record keeps 64
  lines; a dropped line is counted in the record, never lost silently.
- The window starts at the route's request bound. A gateway 413, or a 400 whose `error.code` or
  reason names the context window, shrinks it (larry#236). When the refusal names the model's limit,
  the window becomes nine tenths of the bytes that limit leaves: the refused request's own bytes and
  token count give the bytes a token, so `len * limit / counted` is the model's capacity for this
  conversation, and the tenth is the room a denser next encoding keeps. A refusal that names only the
  limit sizes it at a byte a token. A refusal that names no number shrinks it to 3/4 of the refused
  request, as before. The session compacts and sends again, at most 8 times. Then, or when the head
  alone is over the window, the session ends as a typed refusal.
- The system turn and the task turn never change, so the provider's prefix cache holds. The record
  turns change only when a compaction runs.
- The step bound is 256. It bounds the report, not the conversation.
- The sealed report carries `session.compaction`: `runs`, `cleared`, `folded`, `shrinks` and
  `window_bytes`.

## Consequences

- A model no longer sees old output whole. It sees the command and its status, and can run it again.
- The stop reason set is unchanged. A session stops on the window only when even the head is too big.
- A refusal that names the limit reaches the model's window in one send; one that names none still
  takes the three quarters rule (larry#236).
- Every decision is a byte length, so one seed replays one conversation (`sim_same_seed_same_render_hash`).

## Rejected

- A model-written summary: a paid call that can fail, so the session is not truly auto.
- The 48-step cap alone: it ends long tasks and does not answer a small-window model.
- A token estimate per model: the route has no tokenizer, and the gateway's refusal is the real bound.
- A local tokenizer counting tokens before each send (larry#236): a dependency and a model table that
  drifts, where the refusal already states the provider's own count for this request.
