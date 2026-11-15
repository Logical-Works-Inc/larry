---
type: Benchmark
title: "Larry cache-first prompts over 9router: OpenCode Go models on real errands"
description: Old and new prompt layout, same 12 errands, same 8 models, through 9router. Acceptance, prompt and cached tokens, cost per accepted task, task wall p50/p95/p99 and peak RSS.
resource: /docs/product/bench-cache.md
tags: [larry, benchmark, eval, cache, 9router]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: larry eval eval/errands-v1.json on Oracle, both arms, 2026-11-14}
status: measured
stale_after: 2027-02-14
sources: [larry#230, docs/adr/0043-one-prompt-prefix-per-brief.md, eval/errands-v1.json]
---

# Cache-first prompts over 9router

Director, 2026-11-14: "Use 9router endpoint plus models available from opencode as a test condition and
then benchmark it in a super real world use case. Compare with subtle needs and cheapness. Prioritize high
cache hits from day 0".

## Method

- Set: `eval/errands-v1.json`, 12 errands with hidden tests. On Oracle each stub fails and each reference passes.
  The errands are SemVer order, Retry-After (RFC 9110), CSV (RFC 4180), UTF-8 clip, safe path join, bill split,
  ring buffer, a three-file rename with a deprecated alias, two more errands on that accounts repo, port
  precedence with its doc, and log redaction.
- Arms: old = main `4e2de32`, one prompt per file. New = this change, one prefix per brief.
- Route: `LARRY_MODEL_URL` is 9router on the Mac, reached from Oracle through `ssh -R`. Models are `ocg/*`
  (OpenCode Go). Free `oc/*` models are not used: Zen says free chat models need OpenCode itself.
- Settings, both arms: temperature 0 (kimi 1, the model allows only 1), 3 attempts, no backoff, the 120 s
  default timeout. Debug build. The 8 models of an arm ran at once, and each arm ran once with a cold cache.
- Prices: OpenCode Go per 1M tokens, off-peak, including the cached-read and cache-write rates.
- Samples: 12 tasks per cell, so p99 is the slowest task. RSS is `/usr/bin/time -v` peak for the whole eval.

## Results (MEASURED)

| Model | Arm | Acc / rej / ref | Prompt tok | Cached ‰ | USD per accepted | Task p50 / p95 / p99 s | Peak RSS MiB |
|---|---|---|---|---|---|---|---|
| claude-haiku-5-5 | old | 12 / 0 / 0 | 0 (lost) | n/a | 0 (lost) | 7.30 / 41.45 / 41.45 | 146.1 |
| claude-haiku-5-5 | new | 12 / 0 / 0 | 58,809 | unreported | 0.002080 | 7.33 / 46.51 / 46.51 | 139.2 |
| deepseek-v4.1-flash | old | 9 / 0 / 3 | 6,401 | n/a | 0.001280 | 18.64 / 124.02 / 124.02 | 145.6 |
| deepseek-v4.1-flash | new | 9 / 1 / 2 | 11,792 | 0 | 0.001974 | 19.98 / 123.73 / 123.73 | 139.3 |
| glm-5.3-flash | old | 7 / 5 / 0 | 7,335 | n/a | 0.001605 | 8.49 / 66.86 / 66.86 | 145.9 |
| glm-5.3-flash | new | 6 / 6 / 0 | 12,697 | 0 | 0.001137 | 8.52 / 28.73 / 28.73 | 139.2 |
| kimi-k2.7-code | old | 9 / 0 / 3 | 5,965 | n/a | 0.007497 | 25.12 / 122.76 / 122.76 | 146.0 |
| kimi-k2.7-code | new | 9 / 0 / 3 | 10,771 | 570 | 0.009400 | 22.44 / 122.86 / 122.86 | 140.1 |
| mimo-v2.6-flash | old | 7 / 2 / 3 | 6,117 | n/a | 0.000595 | 36.13 / 122.76 / 122.76 | 146.0 |
| mimo-v2.6-flash | new | 7 / 1 / 4 | 10,907 | 475 | 0.000467 | 31.21 / 123.34 / 123.34 | 139.0 |
| qwen3.8-flash | old | 6 / 1 / 5 | 6,037 | n/a | 0.001582 | 96.70 / 123.41 / 123.41 | 141.2 |
| qwen3.8-flash | new | 5 / 1 / 6 | 10,089 | 0 | 0.001576 | 44.33 / 122.86 / 122.86 | 134.5 |
| minimax-m3 | old | 0 / 10 / 2 | 9,305 | n/a | none | 24.49 / 122.07 / 122.07 | 89.1 |
| minimax-m3 | new | 0 / 10 / 2 | 14,229 | 183 | none | 19.78 / 121.25 / 121.25 | 85.0 |
| gpt-6-luna | both | 0 / 0 / 12 | 0 | n/a | none | 0.32 / 0.50 / 0.50 | 16.0 |

Prompt tokens count only the tasks whose every completion reported usage, so a refused task adds none.

## What the numbers say

1. The old card lost every claude-haiku token: 9router streams it, and the old parser dropped the usage
   chunk. The new card counts it, so haiku costs 0.00208 USD per accepted task, not zero.
2. Cache reads show on three models from the first run: kimi 570 ‰, mimo 475 ‰, minimax 183 ‰.
   deepseek, glm and qwen report zero cached tokens on this set. Why is UNKNOWN; a longer shared prefix
   or a second run on the same prefixes was NOT RUN.
3. Through 9router, claude-haiku's stream carries no cache details, so its card says `cache_complete: false`.
4. Prompt tokens rose 1.5 to 1.9 times, because each call of a multi-file brief now carries every file.
   An agent harness on one leaf sends about 9 M ([bench-harness](/docs/product/bench-harness.md)).
5. Cost per accepted task fell for glm (29 % lower) and mimo (21 % lower), was flat for qwen, and rose for
   deepseek (54 % higher) and kimi (25 % higher).
6. Acceptance moved by at most one task per model. The 120 s timeout refused 16 old and 17 new tasks,
   5 of each on log-redact and 5 of each on retry-after: long single-file rewrites on slow models.
7. gpt-6-luna refuses any request with `temperature` (400). A route that sends no temperature is larry#205.
8. Peak RSS fell about 5 % in every cell (debug build).

## Cheapest and best on this set

- Most accepted: claude-haiku-5-5, 12 of 12, 0.00208 USD per accepted task.
- Cheapest per accepted task: mimo-v2.6-flash with the new layout, 0.000467 USD, 7 of 12 accepted.
