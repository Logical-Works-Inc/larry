---
type: Research
title: Rocco research and reuse ledger
description: Bounded source inspection, primary-source receipts and external review dispositions; not runtime acceptance.
resource: /research-ledger.md
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

# Research ledger

## 2026-09-20 dispatch-time freshness: rules loaded and oracle read

READ: the estate's Rust rule corpus at `~/.agents/skills/rust-skills/rules/`
(265 rule files) was loaded for this change rather than cited. The rules that
shaped it, and what each changed: `proj-pub-crate-internal` (the new module is
private with three curated re-exports, and the fence's two mutating operations
are `pub(crate)`, so a caller cannot pre-clear a reach or assert that a dispatch
happened); `num-overflow-explicit` (`checked_add` for the epoch, so exhaustion
is a typed refusal rather than a wrap that would alias a withdrawn generation
with a live one; `saturating_add` for the exposure counter, so a wrapped count
cannot hand back authority already spent); `pat-exhaustive-enum` (every match on
the refusal enums is exhaustive inside the crate); `err-lowercase-msg` and
`type-display-vs-debug` (hand-written `Display`, lowercase, no trailing
punctuation, never delegating to `Debug`); `type-newtype-ids` (the epoch is a
private-field newtype). One rule was deliberately not applied as an attribute:
`type-newtype-ids` recommends `#[serde(transparent)]`, which would be a no-op
here because a tuple newtype already serializes as its inner value — so the
property was *proved* by a round-trip test asserting the exact string `"7"`
instead of asserted by attribute, which is the stronger form.

The local query layer was run before the work and again after. Pre-exec ranked
D48 (1.0) and E71 (0.992) first among six scope-filtered candidates, which
agreed with the code reading; the post-exec query ranked the tracker gate and
this file's own change record first. Per the standing caveat, the ranking was
used to order inspection and never as evidence that a passage was correct — the
two design errors this change made were both found by tests, not by the oracle.

READ: the sibling repositories were **not** read for code this turn, on explicit
Director direction. The prior reuse survey stands unrevised and nothing from it
was imported.

The two findings worth carrying forward are recorded in `log.md`: a fenced first
dispatch is indistinguishable from a crash in the dispatch window because the
event vocabulary still has no "not dispatched" fact, and consulting the fence
inside the reconciliation-open predicate would have made the refusal silent.

## 2026-09-19 board-first workspace and selective backend reuse

READ: bounded Codex, Cline, OpenCode, AppCUI and Forge/Moo source inventory and
design dispositions are recorded in [workspace ADRs](/docs/workspace-experience.md).
Codex separates thread resume from turn start; this led to identity-bound
Continue without claiming execution. Cline's welcome prompt was rejected for
this entry point; OpenCode's independently scoped context panel was retained.
The backend agent adapted Forge's baseline/fence guard with MIT notice, not
its synthetic Moo allocator. CodeGraph source receipts and test boundaries are
in `crates/rocco-contracts/WORKSPACE-EVIDENCE.md`. No sibling was edited.

The local query layer was used before implementation and again at the routing
pivot. Initial Python failures and the isolated metadata repair are retained
in the ADRs; a model's relevance rank never overrides source or scope. Native
computer-use terminal control was refused, so it was not bypassed. The emitted
SVG was instead converted locally and visually inspected as a static render;
that establishes presentation only. Actual POSIX PTY rendering, input and
mouse events remain separately testable.

### 2026-09-19 Porsche-wrapped Jira correction and local query repair

READ: Porsche Design System v3 palette migration, typography and motion pages;
Atlassian foundations, elevation and motion pages; and Apple HIG layout guidance.
Their common design consequence is not a visual brand transplant: use neutral
surfaces for structure, reserve one focal elevation/selection, apply colour to
meaningful feedback, and separate adjacent jobs spatially. W05 therefore
replaces the prior coloured-region terminal pass with "Quiet Torque": ANSI16
black/gray/white structure, blue selection, and text-labelled amber/red only.
Porsche showroom mimicry and untouched Jira were rejected as respectively too
ornamental and too generic for the return-to-work job. Source URLs are retained
in ADR-W05; source claims are design inputs, not user-study evidence.

MEASURED: the local Qwen reranker initially failed in its own import path when a
stray empty NumPy metadata directory yielded no declared version. Imported NumPy
was 2.4.4. `rerank_local.py` now falls back to that imported package version only
for unusable NumPy metadata, before Transformers validates dependencies. Six
unit tests and an actual no-overlay rerank passed; pre/post receipts are
`20260919-porsche-jira-pre-exec.json` and
`20260919-query-tool-fix-post-exec.json`. This repair makes the query tool
portable across that metadata inconsistency; it does not make Qwen an authority
on product truth or visual quality.

MEASURED after the W05 implementation: the final local post-exec comparison
ranked quiet-torque 1.0, Jira-default 0.5625 and Porsche-showroom 0.0134
(`20260919-porsche-jira-post-exec.json`). The actual macOS debug binary also
completed its five-minute 120x32 PTY measurement with 0 idle output bytes and
0.0% delta CPU; exact timing/RSS and binary hash are in the final metrics
receipt. Neither result proves visual acceptance, portable performance or
frontier superiority.

### 2026-09-19 rail affordance and terminal backend correction

READ: local AppCUI 0.5.1 `backend.rs` documents Termios as the macOS default
and limits mouse support to Windows/NCurses; its CrossTerm implementation
enables mouse capture and decodes terminal pointer events. The user-facing
consequence was independently reproduced: injected SGR pointer bytes rendered
as input under Termios. Rocco now selects CrossTerm from the already-admitted
AppCUI package, rather than claiming an event-handler unit test made the macOS
terminal clickable.

READ: local Forge `forge-tui/src/theme.rs` and `render.rs` place selection on
one small slate-light list surface with a leading highlight symbol. Rocco
reimplements that calm control grammar in AppCUI with an ANSI blue focus row;
no Ratatui code, dependency, or broad colour palette was copied. MEASURED: the
native 120x32 PTY journey sends an SGR click to rail row three and reaches its
work plus the honest missing-Moo refusal. Its cursor-addressed output is
emulated in the test harness so delta painting is interpreted as a screen, not
mistaken for contiguous text. This remains local terminal evidence, not a
cross-terminal or human study result.

MEASURED W06: the mouse-enabled macOS debug binary completed the fresh
300.124-second 120x32 PTY probe with 0 idle bytes, 0.0% CPU delta, 7,824KiB
sampled peak RSS, 0.892ms p95 input, a clean reaped exit, and SHA-256
`c7c2882f7a186521e6b46424bd3417f92000e54470a6d44e3905d1d9d1b47968`.
The exact input distribution and scope limits are retained in
`docs/evidence/workspace-final-metrics.json`; no result transfers to Ryzen,
other terminals, fleet load, or human preference.

## 2026-09-17 native visual pass receipts

READ: published AppCUI 0.5.1 README lists Tab, TreeView, ListView, splitters,
TextArea and Canvas. Its `src/system/builders/input_app_builder.rs:7–17`
documents input-driven custom drawing without periodic ticks. This selected
InputApp for quiet chrome rather than a dense stock-window layout. The same
source defines key repaint and close semantics; real PTY journeys exercise it.
No full editor is inferred from TextArea. Local RAG returned no AppCUI hits;
Context7 returned unrelated libraries, not usable AppCUI evidence.

READ/MEASURED: canonical estate admission in PR #7 records the explicit
Director AppCUI choice and estate-use instruction. No cryptographic human
signature is required by the inspected policy; the earlier extra-signoff claim
was wrong. After explicit release-continuation authorization, publish uploaded
`lgwks_deps` 0.1.8 from 822307da2c7adefccd09b449af2998c75deea42b. Fresh Cargo
download confirmed `appcui` and Rocco pins that published artifact, not a path.
Source: https://crates.io/crates/lgwks_deps/0.1.8 and
https://github.com/srinji-kaggss/logicalworks-crates/pull/7.
The native view and tests use that real dependency; backend and workspace
effects remain simulated/absent as inventoried in the visual-pass document.

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

## External review record

BrowserOS task session `forge strategy` opened owned Gemini and ChatGPT tabs.
Architecture summaries only, no credentials or private source were submitted.
No API model or local agent swarm was spawned for these reviews.

- [Gemini review](https://gemini.google.com/app/722981afca407aa1): initial answer
  called third-party sources primary, claimed append-only events guarantee cache
  hits, implied durable execution eliminates duplicate external effects, claimed
  held-out tests guarantee no future regression, and proposed automatic shared
  crate publication of learned skills. These claims are REJECTED. A correction
  request was sent covering all of them. Useful prompts: context manifests,
  capability attenuation, speculative leakage and self-grading risks.
- [ChatGPT review](https://chatgpt.com/c/6aa9d18f-0048-83ea-9436-82ed4413a692):
  completed and inspected. Accepted as design counterexamples: semantic rather
  than structural grant attenuation, bounded offline leases, no pooled authority,
  crash ambiguity, separate erasure/replay contracts, effect-aware speculation,
  source-bound memory, independent learning evaluation and complete cost ledgers.
  These are recommendations, not proof of runtime guarantees. RFC 9396 section
  6.1 directly supports the permission-comparison warning.
- External model agreement is not architecture validation. Changes require
  source checks, explicit rationale and falsifiers in the notebook.

Saved initial Gemini capture:
`/Users/srinji/.browseros/tool-output/evaluate-1789514223781-c103a7ce-3e0f-4d07-91f7-b690b42ae8f7.txt`.
Browser captures retain full source excerpts separately from this concise ledger.

Gemini's follow-up acknowledged the six corrections. Its further suggestion
that Sentinel can itself impose receiver-side deduplication remains rejected:
an arbitrary remote endpoint must actually support that contract. Its assertions
about all weak models and exact RAM needs are not accepted as measured evidence.
Its suggestion to keep acquired user skills in shared foundation crates conflicts
with the tenant-scoped promotion rule. No further review agreement is treated as
independent product acceptance.

## MatrAIx — truthful persona evaluation audit

User-supplied source: [MatrAIx-Persona-8B](https://github.com/MatrAIx-ai/MatrAIx-Persona-8B).
Pinned inspection revision `3633d8dab149a9482a71b024418a49ae828cc941`.
GitHub license API returned MIT for code (license blob
`62e4d23affd663f90c59715414ebe5764ded78ea`). The subsequently inspected
[dataset card](https://huggingface.co/datasets/MatrAIx2026/MatrAIx_Persona_1M)
states non-commercial research-only terms, including subsets/derivatives, and
retains upstream source restrictions. Code MIT does not relicense those data.
No dataset download, install, training or evaluation execution occurred.

READ README: infrastructure for heterogeneous simulated users; four environment
families; authors claim 1290 categorical dimensions and a one-million-persona
public coreset. Authors explicitly reject treating this as a replacement for
real-person evidence. Development sample of roughly 200 is smoke-only.
DOC current dataset card: 999,847 released personas, 1,290 attributes, 10 packed
Parquet shards; the 999-row viewer is a distinct sample. Counts and composition
are publisher claims, not independently decoded or validated in this task.

READ at pinned revision through `gh api .../contents/... --jq .content`:
- `docs/persona/validation.md`: 10 attributes ×4 environments, positive/negative
  pairs, LLM adherence judge; adherence deliberately separate from task reward.
- `application/task-spec/docs/reporting-and-evaluation.md`: deterministic
  aggregation, simulated self-report, optional LLM analysis. It describes optional
  LLM facets whose absence does not fail deterministic grading and whose provenance
  can become indistinguishable in reporting. Forge must preserve method/missingness.
- `application/tasks/example-chat-api_support_chatbot/tests/test_state.py`:
  `outcome_status` can derive from feedback satisfaction; fallback uses `tracking`
  plus support-message count. `_resolution_progression` treats non-repeated text
  as advanced. These are example heuristics, NOT independent task-success or
  real-human-satisfaction oracles. `PASS` establishes artifact validity there.

CANDIDATE, corrected by Director: use the repository as one external evaluation
source; do not modify it or turn it into the entire acceptance framework. Reject
wholesale adoption of example scoring. Keep separate axes
for simulator fidelity, assistant persona adherence, truthfulness, task outcome,
authority, privacy, continuity, accessibility and human comprehension. Proposed
cohort/trials and cost-approval gates live in notebook Cell 10.

## OSS harness structure review

The Director clarified that Codex means the open-source harness, not another
model's review. READ below means source inspection only, not executed behavior.
Checkouts are pinned reference snapshots, not a claim of latest upstream HEAD.
No source was changed. None of these four roots contained `.codegraph/`; per the
conditional CodeGraph rule, bounded source searches were used instead. Earlier
CodeGraph receipts for the sibling owners remain above; no duplicate clone was
created. Two research-only subagents inspected OpenCode and OpenClaw/Letta;
the implementation executor inspected Codex and reconciled findings.

### H01 — Codex: explicit protocol and environment boundaries

Local `/Users/srinji/codex`, clean inspected checkout at
`bb5054fe47abe73ecbbd454751066a28c89f4bb9`.
[Pinned upstream](https://github.com/openai/codex/tree/bb5054fe47abe73ecbbd454751066a28c89f4bb9).
Structure: Rust workspace, core execution, app-server and generated protocol,
separate execution environments, TUI/CLI and SDK surfaces. Directory structure
alone is not proof that every crate is a clean architectural boundary.

| Source relative to checkout | Observed semantics | Rocco consequence |
|---|---|---|
| codex-rs/app-server/README.md:20–89 | DOC bidirectional RPC, thread/turn/item primitives, initialization, bounded ingress and overload response | R44/R51/R52/R63: separate admission, execution and observation; capability negotiation and bounded transport |
| codex-rs/app-server/README.md:142–182 | DOC read/resume/fork distinctions, effective settings, historical boundaries and cancellation | R59/R61: read is not resume; fork is not copied effect authority |
| codex-rs/config/src/config_layer_source.rs:4–73 | READ explicit config origins, precedence and disabled reason | R45/R59: effective-value provenance and separate security constraints; do not copy precedence mechanically |
| codex-rs/core/src/agents_md_manager.rs:31–45 | READ environment-selection-keyed discovery cache | R45/R47: define instruction refresh and epochs; do not claim file edits always refresh this cache |
| codex-rs/core/src/elicitation.rs:6–95 | READ counted pending elicitations and scoped release | R46: concurrent human waits need explicit lifecycle; in-memory RAII alone is not crash durability |
| codex-rs/core/src/tools/approvals.rs:24–52,110–129,165–187 | READ actions bind environment/path; typed rejection, timeout and abort | R46/R64: uniform effect admission and remote-native identities |

DOC counterexample: the README describes `thread/shellCommand` as unsandboxed.
Rocco explicitly does not adopt a manual-command bypass. Similarly, upstream
hard-delete APIs do not supersede this repository's reversible-removal rule.
Context7 CLI resolved `/openai/codex` and retrieved thread lifecycle and client
capability documentation. This corroborated protocol concepts, not runtime tests.

### H02 — OpenCode: admission and lifecycle detail, not language imitation

Local existing archived checkout:
`/Users/srinji/okf-root/cold-storage/archives/ai-downloads/code/semantic_brain_repo/corpus/repos/opencode`.
Clean inspected revision `55552c521fb428644f64e91c7fa7662656059ceb`.
[Pinned upstream](https://github.com/anomalyco/opencode/tree/55552c521fb428644f64e91c7fa7662656059ceb).
Structure: legacy `packages/opencode` plus a separate v2 `packages/core`, server,
client and SDK implementation. Findings from both are labeled, not blended into
one fictional runtime; source inclusion does not prove deployed release exposure.

| Source relative to checkout | READ finding | Requirement |
|---|---|---|
| packages/core/src/session/input.ts:191–202,245–287 | Retry equivalence includes delivery mode; steering and queued promotion differ | R44 |
| packages/core/src/session/run-coordinator.ts:51–100 | Join/coalesce/stop coordination is process-local | R44/R52; not distributed fencing proof |
| packages/core/src/config.ts:147–210 | Snapshot/merge behavior; invalid entries can be excluded | R45/R59; reject silent security-policy fallback |
| packages/core/src/permission.ts:137–161,219–281 | Denial, saved allowances and pending-request settlement | R46; remember scope must be explicit |
| packages/core/src/session/context-epoch.ts:46–77 | Baseline replacement versus versioned context updates | R47/R57 |
| packages/core/src/session/compaction.ts:190–240 | Nonempty successful summary required; schema/system cost counted | R47 |
| packages/core/src/tool/registry.ts:50–120 | Stale registrations rejected, overlays scoped | R48 |
| packages/core/src/plugin.ts:43–125 | Serialized lifecycle, cycle detection and scoped disposal | R48; lifecycle is not sandboxing |
| packages/core/src/tool/bash.ts:19–21,162–189; tool-output-store.ts:138–172 | Producer capture and model-preview truncation are different | R50 |
| packages/opencode/src/session/message-v2.ts:244–375 | Cross-model metadata/media adaptation and interrupted calls | R49 |
| packages/core/src/event.ts:152–163,565–602; packages/server/src/handlers/event.ts:9–39 | Bounded live stream versus persisted catch-up | R51; reject legacy unbounded stream as a requirement model |
| packages/sdk-next/src/opencode.ts:10–38; packages/core/src/location-services.ts:84–106 | Embedded/server reuse and scoped service disposal | R52 |

### H03 — OpenClaw: channels, schedules and operational control

Local `/Users/srinji/openclaw`, revision
`15de9d881a847626c27631b6dd87dbfe10729757`.
[Pinned upstream](https://github.com/openclaw/openclaw/tree/15de9d881a847626c27631b6dd87dbfe10729757).
Unrelated staged extension files and lockfile edits were present and untouched;
inspected source paths had clean scoped status. Structure: a long-lived gateway,
channel adapters, routed sessions and separate scheduling/delivery services.

| Source relative to checkout | READ finding | Requirement |
|---|---|---|
| src/routing/session-key.ts:222–319 | Account/channel/peer scope and identity linking | R53 |
| src/infra/outbound/source-delivery-plan.ts:25–82,138–167,235–269 | Visible reply owner, destination and fallback semantics | R54 |
| src/cron/schedule.ts:13–18,54–118; src/cron/service/timer.ts:1619–1708 | Calendar/interval/absolute schedules and bounded catch-up | R55 |
| src/wizard/setup.ts:822–887; src/commands/onboard-interactive.ts:15–36 | Multi-stage setup and cancellation | R58 |
| src/gateway/config-reload-plan.ts:13–118 | Hot actions versus restart and disposal | R59 |
| src/cron/service/agent-watchdog.ts:20–46,112–170 | Phase-aware watchdog timing | R60 |

### H04 — Letta: persistent state and asynchronous memory

Local `/Users/srinji/letta`, clean inspected revision
`6d8cb7fd48938b629aad5770faa051a8d42e1e9f`.
[Pinned upstream](https://github.com/letta-ai/letta/tree/6d8cb7fd48938b629aad5770faa051a8d42e1e9f).
Structure: Python API over persisted agent state and relationships, message/run
services, explicit summarization and background memory work. This motivates
requirements, not a second Rocco database or immediate memory implementation.

| Source relative to checkout | READ finding | Requirement |
|---|---|---|
| letta/groups/sleeptime_multi_agent_v4.py:152–198; letta/services/group_manager.py:272–284 | Processing marker and background scheduling occur separately | R56: distinguish scheduled from durably applied; crash race must be tested |
| letta/services/block_manager.py:225–268,61–68 | Shared block update then dependent prompt rebuild with logged rebuild failures | R57: explicit visibility fence instead of silent stale prompt |
| letta/services/summarizer/summarizer_all.py:41–58 | Pending approval and same-step assistant message protected from summarization | R47 |
| letta/services/run_manager.py:103–130 | External status fallback to stored database state | R60: report observation freshness, not invented live health |

## Evaluation-source portfolio

READ upstream README over BrowserOS; these are selection receipts, not score
reproduction or whole-evaluator audits. Pin complete runnable releases and data
licenses before executing. No benchmark was downloaded or run.

| Source / observed revision | Purpose and limitation |
|---|---|
| [SWE-bench](https://github.com/SWE-bench/SWE-bench), `02e7a74ffd0b707aab73d203fe87bdc7c76afc8e` | Coding issue resolution. README warns cached run/instance IDs can reuse earlier patch results; new predictions need new run identity. Its recommended large evaluator host is not proof of Rocco's physical client baseline. |
| [tau-bench](https://github.com/sierra-research/tau2-bench), `2174a603f6d014ef94473ffa95957f6ce27100db` | Tool-user-policy interaction. Current README calls it tau3; banking grading changed in v1.0.1. Never mix those scores silently. |
| [LongMemEval](https://github.com/xiaowu0162/LongMemEval), `9e0b455f4ef0e2ab8f2e582289761153549043fc` | Memory extraction, updates, temporal/multi-session reasoning and abstention. Cleaned data and V2 are mentioned; V2 was not audited here. Its model-judge score is not crash durability. |
| [AgentDojo](https://github.com/ethz-spylab/agentdojo), `089ed468cf3ed0322acc66b0211f26d9d90dbf60` | Prompt injection attacks/defenses. Finite suite coverage is not general security assurance. |
| MatrAIx, pinned above | Persona simulation; keep satisfaction/adherence separate from verified task outcomes. |
| Independent research tasks, lifecycle fixtures and consenting humans | Fill citation/freshness, real failure recovery and actual usability gaps; not substitutes for public comparators. |

## 2026-09-16 SOTA gap research and reviewer triad

Director asked for deep research against SOTA harnesses using the arXiv API
and 9Router, then added ChatGPT/Gemini (BrowserOS) and a Claude review.
Evidence classes: READ = inspected output this session; DOC = source claim.
No benchmark was run; no runtime evidence exists. Dispositions below
motivated R71–R78 and D36–D41; approval of those decisions is still pending.

### Reviewer triad (architecture summaries only; no private source submitted)

| Reviewer / location | Finding class | Disposition |
|---|---|---|
| [Gemini](https://gemini.google.com/app/6266b06c6491a709), READ via BrowserOS | reconciliation oracle per mutating adapter; compensation-of-compensation loop bound; budget-lease TTL with reclaim; speculation-to-context merge; tombstone compaction; TUI-first versus 1M-CCU latency exposure; provider saturation layer | Accepted as design counterexamples; folded into D36/D39/D40/D38 and the R-row set |
| [ChatGPT](https://chatgpt.com/c/6aab1df5-3e08-83ea-a2be-336b0fddb49d), READ via BrowserOS; consulted its stored Rocco/Forge ratification memory | intent binding (version, parameters, preconditions, authority freshness, Zanzibar-cited); per-adapter recovery contract (AWS idempotency pattern); replay separate from live resumption (Temporal-cited); dependency/authority/evidence graphs separate; interacting-fault combined journey; 1M definitions | Accepted; folded into D36/D41 and the combined-journey falsifier E62 |
| Claude thread `claude.ai/chat/25912676-d725-448f-8ee1-558e5ed4932f`, full text relayed by the Director | eight-point pressure test: reducer purity, reconciliation metadata, live-vs-snapshotted attenuation, reservation leaks, supersession/decay/pinning edges, speculation boundary, reducer sharding, 54-case derivation | Points 1/2/4/5a/5c/8 accepted (D36/D39/D40/D41); point 3 answered by existing effect-boundary revalidation plus a new lookup-cost obligation; points 6/7 already satisfied by D10/D22 — recorded as confirmed, not changed |

### arXiv API receipts (2026-09-16, Atom API, abstract-level)

| ID | Finding | Level and consequence |
|---|---|---|
| [2609.00006](https://arxiv.org/abs/2609.00006) | Eleven-system source anatomy: skills lead MCP in adoption (9/11 vs 8/11); no framework imports, no vector retrieval; harness became platform in 2026-H1 | READ full abstract via 9Router; motivates D37 and the hooks/divergence note in D28's favor |
| [2609.11987](https://arxiv.org/abs/2609.11987) | Vendor-native harness pairings skew results; contamination-controlled private suite isolates the harness effect | DOC; D41/E61 |
| [2608.27969](https://arxiv.org/abs/2608.27969) | Long-horizon coding needs composable, evidence-reconfiguring harnesses beyond static ones | DOC; background for D23/D41 |
| [2609.01481](https://arxiv.org/abs/2609.01481) | Harness-of-harness multi-day autonomous development with continual improvement loops | DOC; bounded-learning context for D06 |
| [2609.08258](https://arxiv.org/abs/2609.08258) | Measured agent-memory systems return and act on revoked facts at retrieval | DOC; D40/E62 revocation-enforcement falsifier |
| [2609.14758](https://arxiv.org/abs/2609.14758) | 14.10% of tool-augmented responses assert values the unusable payload cannot support | DOC; D41/E62 post-failure honesty falsifier |
| [2609.12345](https://arxiv.org/abs/2609.12345) | Benchmarks score final success, not error localization/recovery in parallel tool use | DOC; D41/E61 process-level scoring |
| [2609.11677](https://arxiv.org/abs/2609.11677) | Trained runtime-harness evolution improves agents (Ecdysis) | DOC; bounds the D06 self-improvement stance |
| [2609.13543](https://arxiv.org/abs/2609.13543) | Hours-long runs under contention surface failures short benchmarks miss (Asclepius) | DOC; C09 duration-under-contention obligation |
| [2608.22215](https://arxiv.org/abs/2608.22215), [2608.22767](https://arxiv.org/abs/2608.22767), [2609.05510](https://arxiv.org/abs/2609.05510) | Dual-layer memory write routing; experience-amortized retrieval; months-long memory reliability engineering | DOC; background obligations for D02/D27/D33, no mechanism adopted without matched evidence |
| [2608.26004](https://arxiv.org/abs/2608.26004), [2608.28439](https://arxiv.org/abs/2608.28439) | Context-asymmetric speculative decoding; dispatch-level instrumentation catches fabrication fidelity checks miss | DOC; background for D10/D13, no adoption claim |

### Protocol and product receipts (9Router fetches and Google, 2026-09-16)

| Source | Finding | Consequence |
|---|---|---|
| [MCP specification](https://modelcontextprotocol.io/specification/latest), READ | latest schema `2026-07-28`; JSON-RPC, per-request capability negotiation, Resources/Prompts/Tools features | D37 pins one schema revision per adapter; elicitation/sampling get explicit decisions |
| [A2A protocol](https://a2a-protocol.org/latest/), READ | opaque agents, agent-card discovery, tiered extension governance | D37 requires an explicit compatibility/refusal boundary |
| agentskills.io integration page, FETCH FAILED (404); llms.txt pointer retained | official Agent Skills docs exist; exact package spec not yet inspected | open follow-up: pin the skills package spec before C01 |
| Claude Code changelog (Sept 2026), READ via BrowserOS | per-command network allowlists with sandboxing; session forking and background sessions; OTEL cost attribution incl. agent/skill/MCP/vcs tags; gateway pricing parity; MCP reconnect and deferred tool loading | R74/D38 receipts; also the lane-B baseline feature set |

### Rejected or already-satisfied review claims

- "Speculation may blur the propose/authorize boundary": already satisfied by
  D10's explicit external-effect prohibition; recorded as confirmed.
- "Single global reducer cannot scale": already answered by D22 per-scope
  fenced owners; recorded as confirmed, no rewrite indicated.
- "Pick live-checked or snapshotted attenuation": effect-boundary
  revalidation plus bounded offline leases is already the Cell 04 choice;
  the new obligation is only the parent-chain lookup cost budget.
- Reviewer praise, external model agreement and the 90-line
  minimum-harness scaffold are advisory only; they ratify nothing.

## 2026-09-17 hostile-model security and 8B survival review

Director scope: defend supported Rocco paths against malicious/misaligned weak
models, malicious authorized compositions and untrusted internet content without
turning routine work into approval fatigue. Evidence labels: READ means inspected
source bytes/output; ADVISORY means a reviewer argument, not runtime proof. No
Rocco runtime, 8B model, physical Ryzen device or human study ran in this pass.

| Source | Evidence and limit | Design consequence |
|---|---|---|
| [Saltzer and Schroeder](https://web.mit.edu/Saltzer/www/publications/protection/Basic.html), READ | complete mediation, least privilege, fail-safe defaults and psychological acceptability; no claim of complete prevention | D43/D50 require owner-boundary mediation with standing low-friction authority |
| [CaMeL, arXiv:2503.18813v2](https://arxiv.org/abs/2503.18813), READ via arXiv API | capability/IFC architecture reports 77% secure-task utility versus 84% undefended on its benchmark; trusted-user-query and uncompromised-memory assumptions do not cover Rocco's stronger threat model | D42/D44 use hostile-model and compartment IFC principles without importing the paper's assumptions or claiming its result transfers |
| [NIST AI 100-2e2025](https://doi.org/10.6028/NIST.AI.100-2e2025), READ | taxonomy includes direct/indirect prompt injection, misaligned outputs, connected-resource compromise, misuse and supply-chain risk; it does not choose Rocco's risk tolerance | E63–E80 span those attack surfaces with Rocco-specific pass predicates |
| [MCP security best practices](https://modelcontextprotocol.io/specification/draft/basic/security_best_practices), READ | confused-deputy, token-passthrough, local-server and network risks make authenticated transport insufficient | D46 binds secret-broker use and actual egress destinations |
| [OAuth 2.0 Security BCP, RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html), READ | audience restriction, sender-constrained tokens and replay defenses are established mechanisms, not full effect semantics | D43/D46/D48 add exact effect, recipient and freshness binding |
| [TUF specification](https://theupdateframework.github.io/specification/latest/), READ | threshold roles and rollback/freeze/mix-and-match resistance; root-threshold compromise requires independent recovery | D49 separates update/operator/recovery roles and states the root limit |
| [Biscuit authorization overview](https://www.biscuitsec.org/), READ | offline attenuation supports narrow capabilities; authentication and revocation are external responsibilities | Secure-Authority must integrate authenticated principal and durable freshness/fencing contracts; Biscuit alone does not satisfy R87 |
| Rocco dependency manifest at `b297a3d`, Secure-Authority at `ab417c9f`, Nova local working copy `efd006b2` over parent `a66ef8de`, READ | Rocco assigns sandboxing to Nova; Secure-Authority describes capability issuance; Nova's inspected safety docs do not yet claim hostile-code isolation | D43 preserves canonical owners and returns `Unsupported` until the owner contract is verified; no readiness claim |
| Director-relayed Gemini Socratic debate, ADVISORY | challenged malicious-authorized composition, content laundering, 8B unreliability and approval fatigue; BrowserOS submissions were interrupted before a durable page readback | accepted counterexamples became R79–R96/E63–E80; schema adherence was rejected as a security predicate and blanket permanent process taint was replaced by compartment-scoped IFC |

Research failures retained: the first Biscuit security-document URL returned 404
and was replaced by the current official overview; `wwfd fable ask` could not
resolve its configured service; BrowserOS Gemini submission did not yield a
retained agent-owned readback, so only the Director-relayed text is cited as
advisory. A Context7 Wasmtime lookup established possible engine mechanisms but
was rejected as an ownership decision after the Director's correction: Nova,
not Rocco, owns the sandbox contract.

Semantic consequence: the strongest promised property is structural boundary
preservation when all model output is adversarial, not moral omniscience. The 8B
gate has independent containment and usefulness lanes so refusal cannot pass the
product and obedience cannot bypass the monitor. Secure-Authority and Nova code
changes remain future owner-repository work subject to their own verified APIs.

### 2026-09-17 file-specific excellence record

Requirements R79–R96; axes: frontier, hyperscale, idiomatic Rust obligations,
generalized, decoupled, ephemeral, portable, multi-tenant and performance.
Predicted outcome: weaker models and hostile content can reduce task completion
but cannot exceed protected authority or information-flow contracts. Required
verification: E63–E80, including independent mutation oracles, exact artifact
identity, 11 planned crash/recovery cases, physical Ryzen profiles, full tenant
and fleet workloads and the human-friction study. All runtime claims remain NOT
RUN; this edit adds traceable requirements, decisions and falsifiers only.

## Final-review disposition

R44–R70 and D25–D35 specify the previously implicit operational boundaries. The
Director settled naming, private repository, surface order, unattended work,
adapter compatibility and memory/tuning direction. Repository governance is not
a newly invented runtime feature. No claims of complete source coverage, frontier
superiority or implementation readiness are made. Full formal models and runtime
integration remain in the acceptance plan; they are not undecided product scope.

One GitHub tree request initially failed because zsh expanded an unquoted `?`;
quoted retry succeeded. No missing-source conclusion was drawn from that error.

## File-specific excellence record

Requirements R33/R39/R40/R41/R43 and reuse obligations. Axes: frontier research,
decoupled ownership, generalized evaluation and all downstream acceptance limits.
Predicted outcome: avoid repeated archaeology and prevent model/source claims
from becoming invented proof. Verification: links, exact paths and source
identities inspected; ledger validation separate from behavioral tests.
Unresolved: full paper methods, exact evaluation-data rights for the eventual use,
sibling integration readiness, all benchmark and human results. The external
critiques are reviewed; they are not authority to ratify the architecture.
