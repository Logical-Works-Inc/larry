#!/usr/bin/env python3
"""Rerank already-scoped JSONL candidates with a pinned local Qwen model."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import sys
from pathlib import Path

MODEL_ID = "Qwen/Qwen3-Reranker-0.6B"
MODEL_REVISION = "e61197ed45024b0ed8a2d74b80b4d909f1255473"
MAX_CANDIDATES = 256
MAX_LENGTH = 8192
MAX_QUERY_BYTES = 16 * 1024
MAX_CANDIDATE_BYTES = 256 * 1024
MAX_INPUT_BYTES = 2 * 1024 * 1024
DEFAULT_MAX_SECONDS = 900
MAX_MAX_SECONDS = 3600
INSTRUCTION = "Given a software-engineering query, retrieve relevant code and documentation passages."


def candidates(path: Path | None) -> list[dict[str, str]]:
    source = path.open(encoding="utf-8") if path else sys.stdin
    try:
        rows, consumed = [], 0
        for line in source:
            consumed += len(line.encode("utf-8"))
            if consumed > MAX_INPUT_BYTES:
                raise ValueError(f"candidate input exceeds {MAX_INPUT_BYTES} bytes")
            if line.strip():
                rows.append(json.loads(line))
                if len(rows) > MAX_CANDIDATES:
                    raise ValueError(f"candidate count must be between 1 and {MAX_CANDIDATES}")
    finally:
        if path:
            source.close()
    if not rows:
        raise ValueError(f"candidate count must be between 1 and {MAX_CANDIDATES}")
    normalized = []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("each candidate must be a JSON object")
        identifier, text = row.get("id"), row.get("text")
        if not isinstance(identifier, str) or not identifier or not isinstance(text, str) or not text:
            raise ValueError("each candidate needs non-empty string id and text")
        if len(text.encode("utf-8")) > MAX_CANDIDATE_BYTES:
            raise ValueError(f"candidate text exceeds {MAX_CANDIDATE_BYTES} bytes")
        normalized.append({"id": identifier, "text": text})
    if len({row["id"] for row in normalized}) != len(normalized):
        raise ValueError("candidate ids must be unique")
    return normalized


def score(query: str, rows: list[dict[str, str]], batch_size: int, max_seconds: int) -> list[float]:
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer

    def timeout_handler(_, __):
        raise TimeoutError(f"reranker exceeded its {max_seconds}-second limit")

    previous = signal.signal(signal.SIGALRM, timeout_handler)
    signal.setitimer(signal.ITIMER_REAL, max_seconds)
    try:
        tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, revision=MODEL_REVISION,
                                                  local_files_only=True, padding_side="left")
        model = AutoModelForCausalLM.from_pretrained(MODEL_ID, revision=MODEL_REVISION,
                                                      local_files_only=True).eval()
        false_id = tokenizer.convert_tokens_to_ids("no")
        true_id = tokenizer.convert_tokens_to_ids("yes")
        prefix = ("<|im_start|>system\nJudge whether the Document meets the requirements based on the "
                  "Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\"."
                  "<|im_end|>\n<|im_start|>user\n")
        suffix = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
        prefix_ids = tokenizer.encode(prefix, add_special_tokens=False)
        suffix_ids = tokenizer.encode(suffix, add_special_tokens=False)
        prompts = [f"<Instruct>: {INSTRUCTION}\n<Query>: {query}\n<Document>: {row['text']}" for row in rows]
        scores = []
        for start in range(0, len(prompts), batch_size):
            inputs = tokenizer(prompts[start:start + batch_size], padding=False, truncation="longest_first",
                               return_attention_mask=False, max_length=MAX_LENGTH - len(prefix_ids) - len(suffix_ids))
            inputs["input_ids"] = [prefix_ids + item + suffix_ids for item in inputs["input_ids"]]
            inputs = tokenizer.pad(inputs, padding=True, return_tensors="pt")
            with torch.no_grad():
                logits = model(**inputs).logits[:, -1, :]
                pair_logits = torch.stack([logits[:, false_id], logits[:, true_id]], dim=1)
                scores.extend(torch.log_softmax(pair_logits, dim=1)[:, 1].exp().tolist())
        return scores
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("pre-exec", "post-exec"), required=True)
    parser.add_argument("--query", required=True)
    parser.add_argument("--candidates", type=Path)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--top-k", type=int)
    parser.add_argument("--batch-size", type=int, default=2)
    parser.add_argument("--max-seconds", type=int, default=DEFAULT_MAX_SECONDS)
    args = parser.parse_args()
    if not args.query or len(args.query.encode("utf-8")) > MAX_QUERY_BYTES:
        raise ValueError(f"query must be non-empty and at most {MAX_QUERY_BYTES} bytes")
    rows = candidates(args.candidates)
    top_k = args.top_k or min(16, len(rows))
    if (not 1 <= top_k <= len(rows) or not 1 <= args.batch_size <= MAX_CANDIDATES
            or not 1 <= args.max_seconds <= MAX_MAX_SECONDS):
        raise ValueError("top-k, batch-size and max-seconds must be bounded")
    receipt_root = (Path.cwd() / "tools" / "receipts").resolve()
    receipt_path = args.receipt.resolve()
    if receipt_path.parent != receipt_root:
        raise ValueError("receipt path must be directly under tools/receipts/")
    ranked = sorted(zip(rows, score(args.query, rows, args.batch_size, args.max_seconds)), key=lambda item: item[1], reverse=True)
    canonical = json.dumps(rows, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()
    result = {"phase": args.phase, "model": MODEL_ID, "revision": MODEL_REVISION,
              "query_sha256": hashlib.sha256(args.query.encode()).hexdigest(),
              "candidates_sha256": hashlib.sha256(canonical).hexdigest(),
              "candidate_count": len(rows), "top_k": top_k, "max_seconds": args.max_seconds,
              "results": [{"id": row["id"], "score": score} for row, score in ranked[:top_k]]}
    receipt_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(receipt_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as receipt:
        json.dump(result, receipt, separators=(",", ":"))
        receipt.write("\n")
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
