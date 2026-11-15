---
type: Benchmark
title: "Larry harness replay: Claude Code and OpenCode on one real leaf"
description: The larry#209 leaf brief replayed on Claude Code (Haiku) and OpenCode (DeepSeek V4.1 Flash). Tokens, cache, cost, wall time, and each output checked against the other arm's falsifier.
resource: /docs/product/bench-harness.md
tags: [larry, benchmark, harness, claude-code, opencode, cache]
generated: {by: claude-code, at: 2026-11-15}
verified: {by: claude-code, method: both arms run on Oracle 2026-11-15, token counts summed from each harness's own event log, cross-check run with cargo nextest}
status: measured
stale_after: 2027-02-15
sources: [larry#232, larry#209, larry#220, larry#72]
---

# Harness replay: Claude Code and OpenCode

Director, 2026-11-15: "Give me real numbers for a task and then compare real outputs for a task. Assume
Claude code as gold standard and baseline for now (only for now)". Then: "This is an ok estimate for now."

## Method

- Task: the larry#209 leaf brief at `ac62ecb`. Steps: falsifier red, fix, the Gates, docs. No plan, commit or PR.
- Arm A: `claude -p --model claude-haiku-5-5`. Arm B: `opencode run -m opencode-go/deepseek-v4.1-flash`.
- Both ran on Oracle at the same time, from shallow clones of one commit, with the same brief.
- Tokens are summed from each harness's own log. Claude Code cost is its list figure. OpenCode Go bills
  the plan, so arm B's cost uses DeepSeek off-peak rates per 1M tokens: 0.15 in, 0.60 out, 0.003 cached.
- Cross-check: each falsifier on the base commit, and each fix against the other arm's falsifier.
- Baseline row: the original Claude Code Haiku leaf that shipped as larry#220 (it also planned, committed
  and opened the PR).

## Tokens, cost and time (MEASURED, cost of arm B ESTIMATE)

| | Original leaf (#220) | A: Claude Code Haiku | B: OpenCode DeepSeek |
|---|---|---|---|
| Wall time | 52 min | 20.4 min | 23.5 min |
| Model calls | 131 | 62 | 56 |
| Prompt tokens | 36,913,107 | 8,750,295 | 9,029,912 |
| Cache read ‰ | 969 | 983 | 965 |
| Cache write | 1,126,573 | 148,631 | 0 |
| Uncached in | 271 | 174 | 314,648 |
| Output tokens | 139,435 | 32,717 | 46,743 (36,695 reasoning) |
| First call | 10,988 | 50,483 | 56,207 |
| Call p50 / p95 | 298,645 / 417,058 | 147,860 / 159,191 | 184,684 / 216,912 |
| Cost USD | not priced | 0.579 | about 0.101 |

## Outputs (MEASURED)

| Check | A: Claude Code Haiku | B: OpenCode DeepSeek |
|---|---|---|
| Own falsifier fails on base | yes, seed 0 | yes, seed 0 |
| Gates (`larry-oracle all`) | exit 0, 832 of 832 | exit 0, 832 of 832 |
| Retry through `ack` fixed | yes | yes |
| Retry through `Runtime::submit` fixed | no | yes |
| Other arm's falsifier | fails at seed 1 (submit path) | passes |
| Diff | 4 files, +113 / -7 | 5 files, +220 / -24 |
| Final report | one BLOCKED line only | full report |

## What the numbers say

1. Both harnesses send about 9 M prompt tokens for this leaf, and 96 to 98 % of them are cache reads.
   Larry's eval sends about 1 k to 2 k per completion ([bench-cache](/docs/product/bench-cache.md)), but
   that is one completion, not an agent loop, so it does not compare with these rows.
2. Arm B costs about one sixth of arm A (ESTIMATE: the rates are not read from a live page).
3. Arm B found the second entry point (`Runtime::submit`). Arm A did not, and arm B's falsifier proves it.
4. Arm B ignores the brief's style rules: its comments carry em dashes.
5. The merged gold (#220) uses another rule: a refused sync latches the journal until a restore. It covers
   both entry points. Neither arm chose it.
6. One task, one run per arm, on a shared host. Wall time is not a fair comparison. larry#233 replays an
   Opus lead session, three runs per arm, at live prices.
