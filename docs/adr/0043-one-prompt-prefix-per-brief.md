---
type: ADR
title: "ADR 0043: one byte-stable prompt prefix per brief, and cached tokens priced at their own rate"
description: Why every completion of a brief begins with the same bytes, why a tenant partition opens the prefix, and how eval prices cached tokens.
resource: /docs/adr/0043-one-prompt-prefix-per-brief.md
tags: [larry, adr, executor, model, eval, cache]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: src/executor/sim_author.rs, src/model/sim_wire.rs, src/eval/sim_eval.rs and tests/sim_model_client.rs run on Oracle, 2026-11-14}
status: accepted
stale_after: 2027-02-14
sources: [larry#230, larry#205, larry#87, docs/adr/0009-matched-task-set-eval.md, docs/adr/0039-routes-are-tenant-data.md]
---

# 0043: one byte-stable prompt prefix per brief

Status: accepted, 2026-11-14 (larry#230)

## Context

The Director, 2026-11-14: "Prioritize high cache hits from day 0". A provider caches a prompt by its
leading bytes. OpenAI, DeepSeek, Anthropic and OpenCode Go all bill a cache read at 2 to 20 % of the
input price. Before this change each file of a brief got its own prompt, and the one shared part was the
system line, so the cache saw only a few hundred tokens.

Measured on 2026-11-14: a 10k-token prefix sent three times through OpenCode Go. Every model read 96 to
99.9 % of it from the cache on the third send, for example deepseek-v4.1-flash 10,880 of 11,002 and
claude-haiku-5-5 15,899 of 15,903. Through 9router the cache hit with no session header, because 9router
adds one. Direct Go needs `x-opencode-session` (larry#205).

## Decision

- One prefix per brief. The system turn is the instruction, then `Cache partition: <tenant digest>`. The
  user turn is every file of the brief as it was before any write, sorted by path bytes, then the goal.
  Only the last line names the file to write now.
- Snapshot first. Every file is read before any is written, so all prompts of a brief share the bytes.
- Tenant partition. The partition is the BLAKE3 digest of the state directory's tenant. It opens the
  prefix, so two tenants never share a cached prefix.
- Bounded. Past 512 KiB of raw context, or when any request that carries it would pass the route's
  request bound (`ModelClient::fits`, the client's own pre-send check), each prompt carries only its own
  file. JSON escaping can grow a control byte sixfold, so only the encoded request decides.
- The client names itself `User-Agent: larry/<version>`, as OpenCode Go asks of other agents.
- Usage reads cached tokens from `prompt_tokens_details.cached_tokens`, `prompt_cache_hit_tokens` or
  `cache_read_input_tokens`, and written tokens from `prompt_tokens_details.cache_write_tokens` or
  `cache_creation_input_tokens`. Counts that exceed the prompt are dropped. A stream keeps its last usage.
- Eval prices cached reads at `LARRY_EVAL_PRICE_CACHED` and cache writes at
  `LARRY_EVAL_PRICE_CACHE_WRITE`. An undeclared rate is the input price. The card prints
  `cached_tokens`, `cache_write_tokens`, `cache_hit_permille` and `cache_complete`.

## Consequences

- A later file's prompt shows the earlier files as they were, not as the model wrote them. The old
  prompts showed no other file at all.
- Each record says `prefix: shared` or `own`, so a receipt shows which path a brief took.
- A replay of one seed across two state directories now sends different bodies, because the partition
  differs. Tests that compare bodies share one state directory.
- The benchmark over 9router is `docs/product/bench-cache.md`.

## Rejected

- A per-file prompt with the context after the file name: the bytes differ from the first file on.
- The partition after the context: a provider could serve one tenant's cached context to another.
- A session header on every route: 9router adds its own, and a direct Go route is larry#205.
