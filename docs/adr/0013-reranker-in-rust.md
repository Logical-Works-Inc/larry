---
type: ADR
title: "ADR 0013: the local reranker is larry-rerank on Candle"
description: Why the Python Qwen3 reranker became a feature-gated Rust binary and what its receipts carry.
resource: /docs/adr/0013-reranker-in-rust.md
tags: [larry, adr, reranker, python-ban]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: live larry-rerank run on the pinned weights; cargo nextest run --workspace --locked}
status: accepted
stale_after: 2027-02-07
sources: [larry#53, larry#150, Qwen/Qwen3-Reranker-0.6B model card, candle-transformers 0.11 qwen3, ADR 0010]
---

# 0013: the local reranker is larry-rerank on Candle

Status: accepted, 2026-11-07 (larry#53 step 3, larry#150)

## Context

`tools/rerank_local.py` ran the pinned Qwen3-Reranker-0.6B through PyTorch and Transformers, and
`tests/test_rerank_local.py` was the last Python control in CI. Python is banned in product
repositories (Director 2026-10-13).

## Decision

- `larry-rerank`, a binary of the root package behind the `rerank` cargo feature, takes the same flags
  and bounds and prints the same messages. The engine is Candle's `qwen3::ModelForCausalLM` and the
  Hugging Face tokenizer, both through lgwks_deps (`ml-candle`, `ml-tokenizers`). Rejected: an always-on
  dependency, which puts Candle in every build and test lane; a new crate, which needs the Director's
  name; a hand-written transformer, a second implementation of what the estate storefront owns.
- It reads only the pinned snapshot in the Hugging Face cache (`HF_HUB_CACHE`, `HF_HOME/hub` or
  `~/.cache/huggingface/hub`), never the network. The weights load as f32 on the CPU, as the Python
  tool loaded them.
- The score is the probability of "yes" against "no" at the last position, as the Python
  `log_softmax(...)[:, 1].exp()` computed it. Candidates run one sequence at a time; Python's left-padded
  batch under an attention mask gives each row its unpadded logits, so `--batch-size` stays a bound only.
- The time limit is checked after loading and before each candidate; Python's `SIGALRM` could also stop a
  forward pass partway.
- Receipts keep every key and add BLAKE3 digests (`query_blake3`, `candidates_blake3`), since lgwks_std
  has no SHA-256 (as ADR 0010). `larry governance` admits a receipt with exactly one pair. The receipt is
  written create-exclusive, owner-only, directly under `tools/receipts/`. `--top-k 0` is refused, where
  Python treated it as absent.
- The four candidate controls are seeded simulations in `rerank::sim`, which also cover the bounds, the
  ranking against a reference and the receipt. The two NumPy metadata controls retire with NumPy.

## Consequences

- CI's rust job and `cargo local-ci` lint the gated binary; the controls run in the workspace tests, and
  no CI job or local lane runs Python for the reranker.
- Measured on the pinned weights: 6 candidates ranked in 3.01 s wall with 3.0 GiB peak RSS, release build.
  A cold clippy of the feature took 49.4 s. No Python baseline was run (banned), so the speed comparison
  with PyTorch is NOT RUN.
