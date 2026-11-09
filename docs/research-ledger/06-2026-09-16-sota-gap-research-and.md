---
type: Research
title: research-ledger.md, part 6 of 7
description: Part 6 of /research-ledger.md, in order; the index lists every part.
resource: /docs/research-ledger/06-2026-09-16-sota-gap-research-and.md
tags: [larry, research, codegraph, evidence, persona]
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

Director scope: defend supported Larry paths against malicious/misaligned weak
models, malicious authorized compositions and untrusted internet content without
turning routine work into approval fatigue. Evidence labels: READ means inspected
source bytes/output; ADVISORY means a reviewer argument, not runtime proof. No
Larry runtime, 8B model, physical Ryzen device or human study ran in this pass.

| Source | Evidence and limit | Design consequence |
|---|---|---|
| [Saltzer and Schroeder](https://web.mit.edu/Saltzer/www/publications/protection/Basic.html), READ | complete mediation, least privilege, fail-safe defaults and psychological acceptability; no claim of complete prevention | D43/D50 require owner-boundary mediation with standing low-friction authority |
| [CaMeL, arXiv:2503.18813v2](https://arxiv.org/abs/2503.18813), READ via arXiv API | capability/IFC architecture reports 77% secure-task utility versus 84% undefended on its benchmark; trusted-user-query and uncompromised-memory assumptions do not cover Larry's stronger threat model | D42/D44 use hostile-model and compartment IFC principles without importing the paper's assumptions or claiming its result transfers |
| [NIST AI 100-2e2025](https://doi.org/10.6028/NIST.AI.100-2e2025), READ | taxonomy includes direct/indirect prompt injection, misaligned outputs, connected-resource compromise, misuse and supply-chain risk; it does not choose Larry's risk tolerance | E63–E80 span those attack surfaces with Larry-specific pass predicates |
| [MCP security best practices](https://modelcontextprotocol.io/specification/draft/basic/security_best_practices), READ | confused-deputy, token-passthrough, local-server and network risks make authenticated transport insufficient | D46 binds secret-broker use and actual egress destinations |
| [OAuth 2.0 Security BCP, RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html), READ | audience restriction, sender-constrained tokens and replay defenses are established mechanisms, not full effect semantics | D43/D46/D48 add exact effect, recipient and freshness binding |
| [TUF specification](https://theupdateframework.github.io/specification/latest/), READ | threshold roles and rollback/freeze/mix-and-match resistance; root-threshold compromise requires independent recovery | D49 separates update/operator/recovery roles and states the root limit |
| [Biscuit authorization overview](https://www.biscuitsec.org/), READ | offline attenuation supports narrow capabilities; authentication and revocation are external responsibilities | Secure-Authority must integrate authenticated principal and durable freshness/fencing contracts; Biscuit alone does not satisfy R87 |
| Larry dependency manifest at `b297a3d`, Secure-Authority at `ab417c9f`, Nova local working copy `efd006b2` over parent `a66ef8de`, READ | Larry assigns sandboxing to Nova; Secure-Authority describes capability issuance; Nova's inspected safety docs do not yet claim hostile-code isolation | D43 preserves canonical owners and returns `Unsupported` until the owner contract is verified; no readiness claim |
| Director-relayed Gemini Socratic debate, ADVISORY | challenged malicious-authorized composition, content laundering, 8B unreliability and approval fatigue; BrowserOS submissions were interrupted before a durable page readback | accepted counterexamples became R79–R96/E63–E80; schema adherence was rejected as a security predicate and blanket permanent process taint was replaced by compartment-scoped IFC |

Research failures retained: the first Biscuit security-document URL returned 404
and was replaced by the current official overview; `wwfd fable ask` could not
resolve its configured service; BrowserOS Gemini submission did not yield a
retained agent-owned readback, so only the Director-relayed text is cited as
advisory. A Context7 Wasmtime lookup established possible engine mechanisms but
was rejected as an ownership decision after the Director's correction: Nova,
not Larry, owns the sandbox contract.

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
