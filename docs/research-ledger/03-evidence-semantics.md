---
type: Research
title: research-ledger.md, part 3 of 7
description: Part 3 of /research-ledger.md, in order; the index lists every part.
resource: /docs/research-ledger/03-evidence-semantics.md
tags: [rocco, research, codegraph, evidence, persona]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: CodeGraph current source and browser or GitHub API reads}
status: specification-v1.2-source-review-complete-runtime-unverified
stale_after: 2026-10-15
sources:
  - Director conversations 2026-09-15 and 2026-09-16
  - https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
  - https://www.anthropic.com/engineering/multi-agent-research-system
  - https://github.com/MatrAIx-ai/MatrAIx-Persona-8B
  - https://arxiv.org/abs/2609.00006
  - https://modelcontextprotocol.io/specification/latest
  - https://a2a-protocol.org/latest/
---
## Evidence semantics

READ = inspected source/output, not exercised runtime. DOC = source authors'
claim, not independently reproduced. CANDIDATE = design recommendation.
UNKNOWN = insufficient evidence. All product benchmarks here are NOT RUN.
Local worktrees can differ from HEAD: hashes below identify reference revisions,
not a claim that every inspected working byte is committed at that hash.

## Source inventory and reuse decisions

| Source / local HEAD observed | Inspection and finding | Design consequence |
|---|---|---|
| forge-harness `11e9bbc2f3de9fa97d38acf47736628e530cff20` | CodeGraph finds RunSpecV2, RunTerminalV2, RunReceiptV2 under crates/forge-runtime, coordinator, task scope, ledger, workspace transaction | Retract earlier absent-types claim; consider types/tests as reuse candidates, not missing code |
| forge-sdk `958f39ec138ab9b6e1960f7f6b8d0e9f27d8124f` | Earlier read shows stale ownership instructions relative to harness amendment | Historical source only; do not resurrect dual authority |
| Logical-DB `e4e1ead86042cc917e959c3f5fb2206ff61f9319` | CodeGraph Database: open, edit_if_revision, edit_many, changes_since, synchronization and restore; edit_many calls commit_local_batch | Embedded/offline storage is a real API candidate; revision CAS is not distributed fencing proof |
| Secure-Authority `ab417c9f8f664271cf5966e24c51bdc337aa4bed` | secure-daemon capability.rs: channel/audience/action-bound Biscuit, TTL ceiling 300s | Reuse issuer/verifier; tenant/resource/operation binding, attenuation, revocation and fencing need explicit integration proof |
| Nova `34fd34aa757c4d4fce941beb92cc297f075be80c` | native host has profiles, leases, generations and bounds; unsupported_profiles_never_fall_back_to_native test rejects confined/VM/Wasm on that native path | NativeTrusted is NOT hostile-code isolation; inspect separate VM profiles before reuse claims |
| Moo `a4e64252368226913f92a1453c8b435bd1efbede` | CodeGraph claims/release_claims and atomic/conflict fixtures; README has single-host six-agent scope, workspace/editor native VCS | Reuse workspace authority; do not infer fleet scalability; existing uncommitted documentation is preserved |
| Sentinel `a3d62f765352636a4f930d0ad3d1f222c13be9f7` | SearchProvider, MetaSearch, EgressClient authorize then pinned fetch | Reuse internet boundary; HTTP READ can leak/charge, so speculation still needs policy |
| Logicalworks-Crates `30ae69deca86eb25c95e113296eb2d08f50526a7` | lgwks_bot Bot::tick refreshes Auth, bounded poll waves; README foundation and dependency-storefront rules | Use estate foundation; BotSpec is validate-only, not an existing arbitrary workflow interpreter |
| claude-cookbooks `67ce644d33e5933f0bcc0b6eb4113df41bbf3a8f` | CodeGraph MemoryToolHandler and demo dispatch; context-engineering/compaction/memory notebooks inspected via bounded search | Reuse conceptual separation/experimental patterns; do not copy permanent-delete handler or treat a note as RL |

All local repo paths are `/Users/srinji/<name>`. No sibling source was patched.
Moo status inspection showed existing edits to CODEBOOK.md, README.md,
docs/PLATFORM_PLAN.md, docs/T2P-EDITOR-2026-08-15.md and
docs/T2P-WORKSPACE-2026-08-16.md; they remain unrelated and untouched.

### CodeGraph receipts

- `codegraph explore` used on Forge runtime, Secure-Authority, Logical-DB,
  Sentinel, Logicalworks-Crates, Nova, Moo and claude-cookbooks.
- Initial Nova explore reported unavailable index although a `.codegraph/`
  directory existed. `codegraph index` then reported not initialized. Neither
  result was treated as an absent Nova capability.
- Director authorized sync/index. `codegraph sync` refreshed Logical-DB 33 files,
  Secure-Authority 34, Logicalworks-Crates 22, Forge harness 95 and Sentinel 14.
- `codegraph init --yes` indexed Nova 159 files/3900 nodes/13532 edges, Moo
  218/7291/24404, cookbook 180/1706/3048. Counts are tool output, not quality metrics.
- No hook or daemon installation was requested. Index artifacts are intentional
  task outputs with repository ownership, not production code changes.
- Focused `codegraph node Database`, `edit_if_revision`, `edit_many` and
  `crates/secure-daemon/src/capability.rs` followed search results.

### Important old-code counterexamples, not a bugfix queue

READ: Forge TaskScope separates capacity check, thread spawn and handle insertion;
review concurrency atomically before reuse. Coordinator execution uses stdout
`sealed` as a sentinel; the execution backend has test-like task sentinels and
printf behavior. These exact inspected paths cannot by themselves establish a
general verified effect runtime. Do not transplant the whole spine because its
types sound right. This is bounded reuse triage, not authorization to repair it.

## Primary research and what it changed

| ID | Source inspected 2026-09-15 | Evidence level and consequence |
|---|---|---|
| S01 | [Anthropic context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents), published 2025-09-29 | DOC finite attention/context; choose bounded relevant projection, not ever-growing transcript |
| S02 | [Anthropic multi-agent research](https://www.anthropic.com/engineering/multi-agent-research-system), published 2025-06-13 | DOC describes token-heavy multi-agent gains; their 15x-chat observation is not Forge measurement; require cost-aware delegation |
| S03 | [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching), live read | DOC full tools/system/messages prefix and TTL semantics; stable event storage alone cannot guarantee cache hits |
| S04 | [ExpeL v3](https://arxiv.org/abs/2308.10144v3), 2024-12-20 revision | abstract + arXiv API metadata read; nonparametric experience learning, not weight RL or universal error prevention |
| S05 | [Voyager v2](https://arxiv.org/abs/2305.16291v2), 2023-10-19 revision | abstract + API read; executable skill library in Minecraft; cannot extrapolate gains or safety to Forge |
| S06 | [SGLang v2](https://arxiv.org/abs/2312.07104v2), 2024-06-06 revision | arXiv API abstract read; RadixAttention reuse motivates serving adapter comparisons, not mandatory dependency |
| S07 | [Rhai operation limits](https://rhai.rs/book/safety/max-operations.html), [security guidance](https://rhai.rs/book/print.html), [native purity/volatility](https://rhai.rs/book/plugins/module.html) | Context7 CLI library then docs; fuel/limits required, native callbacks and optimizer behavior need separate audit |
| S08 | Local Anthropic cookbook `tool_use/context_engineering/context_engineering_tools.ipynb`, `automatic-context-compaction.ipynb`, `memory_cookbook.ipynb` | READ separates tool clearing, lossy compaction and durable notes; measure each rather than bundle every mechanism |
| S09 | Local cookbook `patterns/agents/prompts/research_lead_agent.md` | READ crisp task boundaries/useful parallelism; reject its universal subagent minimum as unsuitable for Forge efficiency |
| S10 | [Qwen/Qwen3-Reranker-0.6B](https://huggingface.co/Qwen/Qwen3-Reranker-0.6B), model card read 2026-09-17; pinned `e61197ed45024b0ed8a2d74b80b4d909f1255473` | READ: 0.6B text reranker, 32k context, local Transformers scoring with yes/no logits; model card's claimed capabilities are DOC, not Rocco retrieval acceptance. Use as on-demand relevance ranking after scope filtering, never as an authority or instruction source. |

arXiv API command returned an Atom feed with all three requested identities:
`curl --fail --silent --show-error --max-time 20 'https://export.arxiv.org/api/query?id_list=2308.10144,2305.16291,2312.07104&max_results=3'`.
Abstract retrieval is not a full methodology review. Current literature breadth,
matched baselines and learning-theory analysis remain open frontier obligations.

Existing research corpus queried before live sources:
`python3 /Users/srinji/rust-ai-stack/tools/research_indexer.py query 'agent memory reinforcement learning prompt caching' --chunks`.
Results were weakly relevant and included old material; did not treat the returned
`fresh` flag as proof of source currency (reported ages exceeded 1400 hours).
