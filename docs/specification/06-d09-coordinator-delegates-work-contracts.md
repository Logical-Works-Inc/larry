---
type: Design
title: specification.md, part 6 of 18
description: Part 6 of /specification.md, in order; the index lists every part.
resource: /docs/specification/06-d09-coordinator-delegates-work-contracts.md
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
### D09 — coordinator delegates work contracts, not cloned conversations

Choose a task DAG with exclusive goals, explicit dependencies, permitted effects,
input context manifests, output schema, deadline and total budget. Children get
shared immutable context references plus a bounded task delta, not all private
history. Result envelopes retain provenance, unknowns and failed attempts;
summaries are optional projections. Replanning cancels obsolete descendants.
Moo owns conflicting workspace claims. No shared writable prompt or infinite
agent-to-agent debate. Principal assistant remains responsible for integration;
human sees one status with drill-down, not every worker's narration. E08; owner
Larry scheduler, consuming Moo coordination.

### D10 — speculation is a budgeted effect class

Choose parallel deterministic computation and isolated branch experiments when
benefit exceeds expected waste. A network GET can disclose secrets, charge money,
mark content read or mutate a service: READ is not synonymous with safe. Each
adapter declares confidentiality, external observability, cost, reversibility,
freshness and idempotency. Default speculation is local pure work; external
speculation requires an existing capability and explicit policy. Recheck
authorization and input versions before integrating any speculative result.
Shared writes, sending messages and purchases never occur because a branch was
speculative. E10; owner scheduler/effect boundary.

### D11 — dedicated compute is an owned resource envelope

Choose Nova-managed workspaces/leases, explicit CPU/RAM/storage/egress/time/spend
limits and scoped secret injection. The agent may experiment freely within that
contract; it cannot increase its own budget, persist a daemon or escape into the
host. Local model support is a capability-negotiated interface covering chat
templates, tokenizer, tools, structured output, cancellation, context limits and
usage. Malformed tool output gets bounded repair or explicit failure.

Do not hardcode a model family or assume OpenAI-compatible transport means equal
semantics. Support CPU-only configurations and user-selected larger models;
benchmark quantization and model residency separately AND in whole-system cost.
No automatic weight download, cloud fallback or paid retry. A resident local
model's RAM counts against the physical machine; the control-plane budget cannot
hide it. Model-serving locality and sandbox profile are independently admitted.
E11; owner model/compute adapters.

### D12 — abduction is competing hypotheses plus discriminating observations

Choose bounded hypothesis records: explanation, evidence for/against, assumptions,
predictions, confidence and the next informative test. Prefer low-cost probes
that distinguish hypotheses over additional rhetorical reflection. Track missing
data and alternative causes; abstain when evidence is insufficient. Rank via
calibrated probabilistic or symbolic methods where appropriate; MI proposes the
space, MD budgets experiments. Evaluate on hidden root causes and interventions,
not eloquence or agreement with the user. 'Outthink the human' is a useful
aspiration; measurable claim is finding valid omitted causes/actions on held-out
tasks without false certainty. E12; owner reasoning/evaluation.

### D13 — completion is a verified domain outcome plus lifecycle closure

Choose intent-specific obligations fixed at admission and amended explicitly.
Coding, research, messaging and administrative tasks have different graders.
Separate objective validity, subjective acceptance and unavailable evidence.
No model self-report or exit zero alone produces success. A receipt binds request,
policy, artifacts, tests/observations, versions, effect disposition, cleanup and
remaining uncertainty. Receipts attest evidence, not universal correctness.
Domain owners supply observations; Larry does not fabricate signatures or a new
global receipt authority. E13; owner completion reducer and domain verifiers.

### D14 — messaging UI plus a distinct system voice

Choose a typed message envelope: concise answer, visible status, optional choices,
attachments, evidence and expandable detail. Core events drive task cards,
budget notices, delegation status and recovery prompts without another model
call. Clearly distinguish assistant narrative, human messages and system facts.
Attachments have scope, digest, size/type bounds and accessible descriptions;
they cannot smuggle instructions into an authoritative channel. Streaming tokens
are provisional until a complete validated message exists. E15; owner surfaces.

### D15 — matched weak-model and persona evaluation, not a single score

Choose three lanes: A same-model harness comparison; B best-product comparison
against Claude Code with all differences reported; C human/persona experience.
Never compare a local weak model against an unavailable Claude Code configuration
and call it controlled. If Claude Code cannot run that exact model/tool setup,
use a transparent neutral baseline for A and retain Claude Code for B. Freeze
versions, tasks, tools, budgets, providers, context, caching, network and judges.
Persona adaptation and user-simulator fidelity are different experiments. Reuse
MatrAIx tooling subject to license and verifier audit; detailed plan in Cell 10.
E16/E27; owner independent evaluation.

### D16 — security and ethics at every trust transition

Choose threat modeling for malicious documents/tools/skills, compromised workers,
credentials, prompt injection, memory poisoning, replay, confused deputies,
resource abuse and inter-user disclosure. Separate instructions from retrieved
data; provenance tagging alone is not injection prevention. Enforce output/effect
constraints outside the model. Use per-scope encryption/key policy, least
privilege, egress controls, supply-chain provenance and independent red teams.
No model sees raw root credentials. Privacy/export/retention and revocation cover
embeddings, caches, summaries, skill artifacts and training copies too.

Ethics requires scoped consent, contestability, accessible controls and honest
uncertainty; do not equate legality with absence of harm. Reversible user choices
may be unwise without justifying paternalistic obstruction. Do not optimize
dependency, emotional vulnerability or covert persuasion. Cross-tenant learning
is off unless explicitly authorized and privacy-reviewed. E17; owner security
and human acceptance, enforced through Secure-Authority and domain boundaries.
