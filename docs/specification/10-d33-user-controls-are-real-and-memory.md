---
type: Design
title: specification.md, part 10 of 18
description: Part 10 of /specification.md, in order; the index lists every part.
resource: /docs/specification/10-d33-user-controls-are-real-and-memory.md
tags: [larry, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
### D33 — user controls are real, and memory waits for its owner

Director decision: defer memory implementation until Logical-DB is ready; retain
the complete memory requirements and blocking dependencies. No alternate store,
fake adapter, silent no-op or claim of implemented decay is permitted.

Every user-facing behavioral preference exposes a named scale or typed control
with unit, range, default, current effective value, scope, consequences, reset
and export/import behavior. Include verbosity, initiative, delegation, scheduling,
notification frequency, speculation, budgets, retrieval and decay. Settings take
effect at an explicit boundary and are testable; parsed-but-unenforced settings
fail acceptance. Linked controls disclose coupled effects rather than silently
clamping or changing other settings. Resource/authority ceilings stay explicit;
users may revise their own standing contracts, not another owner's permissions.

Proposed relevance decay for ordinary contextual messages is
`w(d, lambda) = 1 / (1 + lambda * ln(1 + d))`, with substantive-message distance
`d >= 0`, `lambda` in [0,4], default 1 and 0 disabling decay. This is a tunable
ranking policy, not a claim of optimal cognition or a retention policy. Status
and continuation messages do not advance d. Explicit corrections and unfinished
requirements remain pinned until the authorized user supersedes/cancels them.
Automatic archival/expiry is a separate user-visible owner policy, preserves
obligations and follows reversible-removal rules; it is not implicit in w.
Exact storage/retention implementation depends on Logical-DB's accepted contract.
E46; owner user settings over Logical-DB and authority boundaries.

### D34 — triangulate evaluation without rewriting upstream harnesses

MatrAIx is one external evaluator, not a runtime dependency or a universal score.
Keep upstream code unchanged; adapters and run configuration live in the eventual
evaluation tooling only when implementation is authorized. Use SWE-bench for
coding, tau-bench for tool-user-policy interaction, LongMemEval for memory,
AgentDojo for injection, independently authored research/citation tasks, lifecycle
fault journeys and consenting humans. Source scope and versions are in the ledger.

Freeze task/evaluator revisions, public-benchmark contamination limitations,
development/hidden splits, failed/missing trial treatment and real cost before
comparison. A disabled judge or refused task remains in the declared denominator
with its disposition. Do not average away a security violation with persona gains.
Inspect evaluator changes before claiming regression improvement; prove each
custom gate detects a relevant negative control. Upstream dataset licensing is
separate from source-code licensing. E49; owner independent evaluation.

### D35 — history, upgrades and distribution are reversible where claimed

Fork creates a new identity with lineage and a declared committed history boundary;
it does not copy execution leases, pending effects or grants as usable authority.
Read/export/archive never resumes work. Imports validate schema/provenance and
treat foreign instructions as data until admitted. Export is scoped, portable and
redacted by policy; roundtrip must preserve declared semantics, not secret keys.

Verify release provenance/digest and compatibility before activation; avoid
ambient install scripts. Pin in-flight semantics. Interrupted upgrades preserve a
recoverable old state; incompatible durable migrations have an explicit backup
and owner-approved recovery contract, never an invented universal downgrade.
After retention expiry, stale replicas must rebootstrap rather than resurrect
removed data. History redaction makes affected replay explicitly incomplete.
E45/E50; owner release and storage integration.

### D36 — authorization is pure and reconciliation is explicit effect metadata

Choose authorization as a pure function of the declared effect, a policy
snapshot, capability state and the reserved budget snapshot: no wall-clock
time, live external balance or network state is read inside the reducer.
Replaying the recorded effect log under pinned inputs yields bit-identical
authorization decisions; any divergence is a new reducer version entering at
an explicit checkpoint, never silent reinterpretation. One effect-metadata
schema unifies capability declaration and recovery contract:
`reconciliation_strategy` is queryable, idempotent-retry-safe,
requires-human or always-compensate. OutcomeUnknown can be permanent for
adapters without query or idempotency support; such effects hold visibly
without blocking unrelated work. Cancellation does not release money an
unresolved effect may already have spent; compensation carries its own
authority and budget, and compensation chains are depth-bounded. E55/E56;
owner Larry control plane with adapter contracts.

### D37 — ecosystem protocol adapters: Agent Skills, MCP, A2A

Choose the Agent Skills package format as a primary admitted input for
discovered skills; frontmatter and scripts stay untrusted until quarantine
and D06 promotion. Pin one target MCP schema revision per adapter; map MCP
elicitation onto durable scoped questions and route MCP sampling requests
through the MD authority gate like any effect; unsupported protocol
features are declared refusals. Make an explicit A2A adapter decision for
federation: which agent-card fields map to Larry capability manifests and
which identity claims are refused. These are compatibility surfaces, not
authority transfers. E57; owner tool integration.

### D38 — operations telemetry and provider-admission pressure

Choose OpenTelemetry export with repository/ref, agent, skill and MCP-server
attribution on cost and token metrics, redacted by policy; gateway-injected
pricing keeps reported cost equal to the spend meter. Per-command egress
allowlists are declared through Sentinel and enforced at the effect
boundary. Add an admitted LLM request-shedding/priority queue so the
control plane degrades by explicit policy under provider rate limits
instead of starving. Telemetry is evidence, never an authorization input.
E58; owner observability with Sentinel/Nova integration.
