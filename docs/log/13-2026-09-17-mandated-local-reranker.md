---
type: Log
title: log.md, part 13 of 15
description: Part 13 of /log.md, in order; the index lists every part.
resource: /docs/log/13-2026-09-17-mandated-local-reranker.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-17: mandated local reranker evidence for repository execution

Director direction: Rocco repository work now uses the local,
on-demand `tools/rerank_local.py` before and after non-read-only execution.
The tool pins `Qwen/Qwen3-Reranker-0.6B` at
`e61197ed45024b0ed8a2d74b80b4d909f1255473`, accepts already access-filtered
JSONL candidates, caps them at 256, and returns ranked JSON evidence. Pre-exec
receipts inform planning; post-exec receipts rank verification evidence. Neither
is an authority decision, source of instructions, automatic model download,
daemon, host hook or product-runtime acceptance claim.

READ: the Hugging Face model card specifies a 0.6B reranker with 32k context and
the documented Transformers yes/no-logit method. MEASURED on this macOS host:
the exact pinned local model ranked the relevant synthetic candidate above the
irrelevant candidate (0.9921875 vs 0.0000166893). This is a two-candidate tool
smoke test, not retrieval quality, low-resource hardware, portability,
multi-tenant, fleet or human-outcome evidence. The local package import failed
for `sentence_transformers` because its installed TorchCodec dependency lacks a
compatible FFmpeg library; the tool uses the model card's direct Transformers
method instead. No Rust file changed.

Retained execution receipts: `pre-exec` ranked `local-reranker` 1.0 over
`unrelated` 0.00014019012451171875; `post-exec` ranked `verification` 0.9921875
over `unrelated` 0.0004177093505859375. Both used two synthetic, scope-safe
candidates and the pinned local artifact. Their query/candidate digests are
retained under `tools/receipts/`; each evidence command uses a new receipt
path so a prior receipt cannot be overwritten.
These receipts prove the CLI path and ordering for this bounded sample only.

## 2026-09-17: AI query workflow made executable in AGENTS.md

`AGENTS.md` now gives the agent-facing local query sequence: scope and filter
untrusted candidates, produce a unique pre-exec receipt, use rank only as
relevance evidence, perform authorized work, retain a distinct post-exec
receipt over final evidence, then run the actual repository checks. It names
the local failure modes and explicitly preserves source provenance, authority,
scope, test and human-experience boundaries.

## 2026-09-17: local reranker lifecycle bounded

The local reranker remains one-shot: it loads only during a query, defaults to
a 900-second wall-clock bound (maximum 3600), writes an evidence receipt, and
exits so operating-system process teardown reclaims model memory. A timeout is
a loud failed query, never a silently resident model service. This is lifecycle
semantics for the repository tool, not a measured RAM budget or hardware claim.

## 2026-09-17: hostile-model defense and the 8B survival gate

The Director required strict defense in depth against malicious or unreliable
models, malicious authorized compositions and the untrusted internet, with Nova
retained as sandbox owner and Secure-Authority as the authenticated capability
owner. R79–R96, D42–D53 and E63–E80 make model obedience unnecessary for
security, bind resolved effects at their domain owners, add compartment-scoped
information flow, cumulative composition limits, secret/egress binding,
collaboration isolation, revocation/recovery/update fences, exact low-friction
consent and independent mutation oracles.

The named-artifact 8B gate separates zero-violation containment from useful work:
target at least 90% verified benign completion, no more than five percentage
points below the matched same-model baseline, zero repeated approvals within
unchanged authority and no stronger-model/cloud fallback. The Ryzen 3 4300U
8 GiB and 4 GiB profiles, local-admission p95 <=50 ms and human estimate of 0–1
routine interventions are targets, NOT RUN measurements.

No Rust file changed in Rocco, Secure-Authority or Nova. No runtime, model,
hardware, coverage, fleet or human result is claimed. The prior Wasmtime owner
assumption was corrected before specification delivery: an engine can only be a
Nova implementation mechanism. Failed research/browser receipts and source limits
are retained in `research-ledger.md` rather than converted into evidence.

Local document verification on the final working-tree bytes: `python3
tools/check_governance.py` passed with 96 requirements, 53 decisions, 80
falsifiers, 13 execution cells and 9 axes; `python3 -m unittest discover -s tests
-v` passed 38/38 governance tests; `git diff --check` passed. The checker states
that Mermaid rendering and runtime/model/hardware/human trials are not checked.

## 2026-09-15: settled Director directions

| Direction | Disposition |
|---|---|
| Codex means the OSS harness | Inspect actual source structure; do not confuse it with a model review |
| Rocco is the new placeholder name; create private repository now | Private `srinji-kaggss/Rocco`, documentation/governance only |
| Core + TUI first | D19 delivery order settled; phone/app/web preserved |
| Yes to bounded unattended work | D31 capability required, no background job installed now |
| Adapters, not clones | D28 compatibility commitment settled |
| Defer memory until Logical-DB is done; auto decay, customizable on a scale | D33 records the hold and real typed controls; memory remains required |
| Strict and self-updating governance in the repo being worked on | AGENTS/GOVERNANCE maintenance, deterministic report and CI; not a runtime subsystem |
| Add control flow and flowchart/DAG | Specification includes component map, runtime flow, effect states and implementation DAG |
| MatrAIx is one evaluation source; do not modify it | D34 portfolio with independent benchmarks, fault tests and human trials |

Finalizing the specification is not blanket approval of every proposed
architecture decision, implementation work or billable benchmark execution.
