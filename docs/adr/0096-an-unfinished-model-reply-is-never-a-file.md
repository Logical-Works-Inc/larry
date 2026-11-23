---
type: ADR
title: "ADR 0096: an unfinished model reply is never a file"
description: Why Larry refuses a reply whose finish_reason names a cut or a filter, a torn stream chunk and an empty file reply, on a closed list of finished reasons.
resource: /docs/adr/0096-an-unfinished-model-reply-is-never-a-file.md
tags: [larry, adr, model, wire, author]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: a_rejected_or_refused_task_is_a_failed_tool_result_never_success red on Oracle then green; the workspace suite on Oracle, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#396, larry#128, larry#236, OpenAI chat completion object finish_reason, Anthropic Messages stop_reason, Gemini FinishReason]
---

# 0096: an unfinished model reply is never a file

Status: accepted, 2026-11-23 (larry#396).

## Context

The wire reader took `choices[0].message.content` and ignored `finish_reason`. A reply
cut at the token limit (`length`) or stopped by a filter (`content_filter`) was written
as the whole file, its acceptance ran, and the report was sealed. An empty reply
truncated the file. A streamed `data:` chunk that was not JSON was skipped, so the
rest of the stream was written as the whole reply.

## Decision

- The list of finished reasons is closed: a missing or `null` reason, `stop`,
  `end_turn` and `stop_sequence`, ASCII case ignored (OpenAI, vLLM, Anthropic through a
  gateway, Gemini's `STOP`). Every other reason is `ModelError::Unfinished` naming it,
  at most 64 characters with controls blanked. A reason that is not text is malformed.
- A stream keeps its last non-null reason. A `data:` chunk that is not JSON is
  `ModelError::StreamChunk` with its line and column.
- None of these is retried: the reply was paid for, and the same prompt is cut again.
- A brief file whose reply is empty after the fence is removed is
  `AuthorFault::EmptyReply`. `larry eval` scores it, and `Unfinished`, as the arm's
  refusal.

Rejected: refusing only `length` and `content_filter` by name. Anthropic's
`max_tokens`, Gemini's `MAX_TOKENS` and `SAFETY`, and any reason no gateway documents
would still write half a file. Rejected: retrying a cut reply with a larger limit. The
route owns the limit, and a silent second paid call is spend nobody asked for.

## Consequences

- A gateway that sends a reason outside the list for a finished reply is refused.
  Reopen the list if a route's sealed refusals name such a reason for a reply that
  was whole.
- Falsifiers: `model::wire::sim` (`sim_every_finish_reason_in_every_framing_gets_the_reference_verdict`,
  `a_torn_stream_chunk_refuses_the_reply_by_its_line`),
  `executor::author::sim::sim_an_empty_reply_is_never_written_as_the_file`, and
  `tests/mcp_stdio_journey.rs` (`a_rejected_or_refused_task_is_a_failed_tool_result_never_success`).
