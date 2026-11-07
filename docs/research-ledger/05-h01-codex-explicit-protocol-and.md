---
type: Research
title: research-ledger.md, part 5 of 7
description: Part 5 of /research-ledger.md, in order; the index lists every part.
resource: /docs/research-ledger/05-h01-codex-explicit-protocol-and.md
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
