---
type: Design
title: specification.md, part 2 of 18
description: Part 2 of /specification.md, in order; the index lists every part.
resource: /docs/specification/02-harness-review-requirements.md
tags: [rocco, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
### Harness-review requirements

These derived requirements strengthen the original inventory rather than replace
it. Source anchors H01–H04 are in the research ledger. Every row has an exact
pass/fail journey below; no source inspection is counted as a passing journey.

| ID | Origin | Required behavior | Decision | Acceptance |
|---|---|---|---|---|
| R44 | OpenCode/Codex | Durable input admission, safe steering, queue ordering and retry identity | D25 | E28 |
| R45 | Codex/OpenCode | Explainable configuration/instruction precedence, trust and provenance | D26 | E29 |
| R46 | Codex/OpenCode/Letta | Durable human questions and approvals, expiry, cancellation and exact scope | D25 | E30 |
| R47 | OpenCode/Letta | Failure-atomic compaction and explicit context epochs preserving pending exchanges | D27 | E31 |
| R48 | OpenCode | Version-bound tool registration, isolated extension lifecycle and schema checks | D28 | E32 |
| R49 | OpenCode/Codex | Provider-neutral transcript lowering, authentication and rate-limit recovery | D29 | E33 |
| R50 | OpenCode | Distinguish capture loss, preview truncation and complete retained artifacts | D29 | E34 |
| R51 | OpenCode/Codex | Durable cursor catch-up, bounded live notifications and slow-consumer recovery | D25 | E35 |
| R52 | OpenCode/Codex | Explicit service lifetimes and equivalent embedded/remote behavior | D25 | E36 |
| R53 | OpenClaw | Scoped external sender identities and reversible explicit account linking | D30 | E37 |
| R54 | OpenClaw | Reply owner and destination binding across concurrent channels and threads | D30 | E38 |
| R55 | Director/OpenClaw | Bounded scheduled/event-driven work, DST, missed runs, overlap and expiry | D31 | E39 |
| R56 | Letta | Separate accepted, scheduled and durably applied background-memory cursors | D27 | E40 |
| R57 | Letta/OpenCode | Acknowledged shared corrections have an explicit visibility fence | D27 | E41 |
| R58 | OpenClaw | Resumable setup, readiness distinctions and non-interactive recovery | D32 | E42 |
| R59 | OpenClaw/OpenCode | Desired versus effective configuration with atomic activation and rollback | D26 | E43 |
| R60 | OpenClaw/Letta | Diagnostics expose observation age, execution phase and safe recovery actions | D32 | E44 |
| R61 | Codex | Explicit fork, archive, export/import and redaction semantics; no replayed effects | D35 | E45 |
| R62 | Director | Adjustable decay and user-facing scales; memory implementation waits for Logical-DB | D33 | E46 |
| R63 | Codex | Versioned headless API, typed exit/error results and capability negotiation | D25 | E47 |
| R64 | Codex/OpenCode | Remote environment identity, path semantics and uniform effect admission | D28 | E48 |
| R65 | Director | Independent evaluation sources; MatrAIx is one source, not the entire oracle | D34 | E49 |
| R66 | Harness synthesis | Verified distribution, compatibility-aware updates and interrupted-upgrade recovery | D35 | E50 |
| R67 | Harness synthesis | Accessible UI and safe rendering of untrusted terminal/media content | D32 | E51 |
| R68 | Moo/Codex | Conflict-aware change previews and undo that preserves unrelated human edits | D08 | E52 |
| R69 | Research review | Answer citations retain source version, retrieval time and claim/evidence distinction | D13 | E53 |
| R70 | Harness synthesis | Bounded no-progress loops with causal diagnostics, not endless agent chatter | D09 | E54 |

### SOTA-gap requirements (2026-09-16 research)

Derived from the 2026-09-16 harness gap research: the Gemini/ChatGPT/Claude
triad review, the eleven-system harness study, the MCP/A2A/Agent Skills
receipts and the adversarial-memory and honesty literature. Source anchors,
evidence levels and rejected claims are in the research ledger. Every row
binds a decision and falsifier; no research finding is accepted as runtime
evidence.

| ID | Origin | Required behavior | Decision | Acceptance |
|---|---|---|---|---|
| R71 | Claude triad review 2026-09-16 | Authorization purity: reducer decisions replay bit-identically from the recorded effect log under pinned policy/capability/budget snapshots | D36 | E55 |
| R72 | Gemini/ChatGPT/Claude reviews 2026-09-16 | Unified effect metadata carries an explicit reconciliation strategy; permanent OutcomeUnknown is a declared disposition, never a blind retry | D36 | E56 |
| R73 | Harness study 2609.00006; MCP/A2A receipts | Ecosystem adapters: Agent Skills package format, pinned MCP schema revision with elicitation/sampling decisions, A2A agent-card compatibility boundary | D37 | E57 |
| R74 | Claude Code changelog receipts 2026-09 | Operations telemetry: cost/token attribution by agent, skill, MCP server and repository; gateway cost parity; LLM request-shedding queue; per-command egress allowlists | D38 | E58 |
| R75 | Triad reviews 2026-09-16 | Budget reservations are leases with expiry, reclaimed through the ordinary reconciliation machinery, bounded by parent budgets | D39 | E59 |
| R76 | Claude triad review 2026-09-16 | Memory edge semantics: decay freeze on supersession, pin survival against machine-derived supersession, declared compaction reversibility and tombstone provenance | D40 | E60 |
| R77 | arXiv 2609.11987/2609.12345 | Harness-effect isolation on a contamination-controlled private suite and process-level recovery scoring in evaluation | D41 | E61 |
| R78 | arXiv 2609.08258/2609.14758 | Adversarial honesty and memory hygiene: revoked facts are not acted on, tool-failure reporting stays truthful, interacting-fault journeys are planned and the inventory carries a derivation note | D41 | E62 |

### Hostile-model security requirements (2026-09-17 review)

These requirements assume that a weak, confused, misaligned or deliberately
hostile model can emit any bytes. They preserve Nova as the sole sandbox owner:
Rocco admits effects, Secure-Authority authenticates and attenuates authority,
Nova contains execution, Sentinel mediates external connections, Moo owns
workspace integration and Logical-DB owns durable security state. No prose or
model behavior is part of the trusted computing base.

| ID | Origin | Required behavior | Decision | Acceptance |
|---|---|---|---|---|
| R79 | Director/Gemini security debate | Treat every model, prompt, memory summary, content-parser output, plugin and tool payload as hostile input; model-supplied identity, provenance, policy, labels, grants, budgets and lifecycle state are never authoritative | D42 | E63 |
| R80 | Complete-mediation review; Nova correction | Every supported effect crosses the responsible owner's boundary; unavailable containment is `Unsupported`, never a host or `NativeTrusted` fallback for hostile code | D43 | E64 |
| R81 | Confused-deputy/TOCTOU review | Bind admission to authenticated principal and tenant, intent revision, operation, resolved resource identity/version, destination/account, payload or artifact digest, adapter/schema/policy epoch, budget reservation, expiry and execution fence | D43 | E65 |
| R82 | CaMeL/IFC review | Preserve protected origin, integrity, confidentiality, ownership and authority as separate facts through derivation; authenticated transport and provenance never confer instructional authority | D44 | E66 |
| R83 | Gemini laundering counterexample | Enforce explicit and implicit information flow at sinks; paraphrase, encoding, summarization, translation, generated code, images and agent relays cannot declassify data | D44 | E67 |
| R84 | Malicious-authorized-human counterexample | Apply composition-aware recipient, schedule, concurrency and cumulative impact bounds across runs, workers and identities governed by the same policy; disclose rather than pretend to govern identities outside that boundary | D45 | E68 |
| R85 | MCP/RFC 9700 confused-deputy review | Keep secret bytes outside model context and general process state; narrow secret-broker use and every actual connection, redirect and resolved destination to the admitted operation, audience, account and recipient | D46 | E69 |
| R86 | Colluding-agent counterexample | Collaboration, caches, embeddings, shared sessions, skills and evidence pools never union tenant data, authority, budgets or release entitlements | D47 | E70 |
| R87 | Revocation/replay review | Separate reproducible authorization from fresh authorization; enforce current revocation fences, security epochs, lease expiry, replay protection and explicit bounded offline authority at the executing owner | D48 | E71 |
| R88 | Crash/partial-failure review | Persist effect identity, reservation and recovery contract before dispatch; crash, cancellation, retry and restore cannot manufacture success, release ambiguous spend or resurrect authority; uncertain effects remain `OutcomeUnknown` | D48 | E72 |
| R89 | TUF/supply-chain review | Bind privileged activation to exact verified artifacts and compatibility; resist rollback, freeze and mix-and-match attacks with separated update roles, thresholds and protected security epochs | D49 | E73 |
| R90 | Operator-compromise review | Separate policy administration, update signing, execution and recovery authority; state the compromise threshold and require independent recovery for root compromise rather than claiming signatures solve it | D49 | E74 |
| R91 | Ordinary-user counterexample | Authorize a bounded workflow once and recheck mechanically; unchanged standing scope produces no repeated approvals, while an authentic UI action binds the exact changed recipient, release, payload or irreversible boundary | D50 | E75 |
| R92 | Director 8B survival rule | Security acceptance uses a hostile proposal generator as well as the named weak model; utility acceptance separately requires useful verified completion, so refusal or malformed output cannot bypass security or pass usefulness | D51 | E76 |
| R93 | Accessible-hardware correction | Pin weights, parameter count, quantization, tokenizer, template, adapters and runtime; prohibit stronger-model or cloud fallback in the 8B lane and preserve work under bounded OOM/kill/timeout on the 8 GiB and 4 GiB profiles | D51 | E77 |
| R94 | Assurance/oracle review | Each declared defense has an independent outcome oracle and a negative mutation that proves the suite detects its removal or bypass; passing model refusals are not security evidence | D52 | E78 |
| R95 | CaMeL residual-channel review | Declare and measure observable timing, termination and resource-contention channels per deployment; bound them where required and never claim zero leakage from explicit-flow tests alone | D52 | E79 |
| R96 | Governance-limit review | Report boundary security, abuse containment and content/moral judgment separately; authorization never certifies truth, consent, legality or harmless purpose, and optional content policy cannot expand authority | D53 | E80 |
