---
type: Design
title: specification.md, part 3 of 18
description: Part 3 of /specification.md, in order; the index lists every part.
resource: /docs/specification/03-cell-02-candidate-architecture-graph.md
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
## Cell 02 — candidate architecture graph

```mermaid
flowchart TD
  H[Humans: private and shared scopes] --> UI[Message surfaces: TUI phone app web]
  UI --> C[Versioned command and event contract]
  C --> MD[MD: admission reducer scheduler budgets evidence]
  MD --> SA[Secure-Authority: identity grants revocation secrets]
  MD --> DB[Logical-DB: scoped durable records and sync]
  DB --> CX[Context compiler: graph projection and manifest]
  CX --> MI[MI: principal assistant and bounded specialist workers]
  MI --> P[Typed proposals: plans hypotheses effects memory skills]
  P --> MD
  MD --> N[Nova: admitted compute and isolation profiles]
  MD --> M[Moo: workspace claims editor integration]
  MD --> S[Sentinel: internet egress browser research]
  N --> EV[Observed results and evidence references]
  M --> EV
  S --> EV
  EV --> MD
  MD --> UI
  EV --> L[Offline learning and held-out evaluation]
  L --> PR[Versioned policy and skill candidates]
  PR --> MD
  F[Logicalworks-Crates: std bot deps ast] -.foundation.-> MD
```
Edges denote contracts, not mandatory processes or distributed RPC for every
call. Begin with a modular Rust core and out-of-process untrusted workers. A
deployment can embed trusted components; fleet deployments place the same
contracts across workers. Do not build an independent service for every box.

**Ownership rule:** Larry owns assistant-domain intent, commitments, context
selection and run orchestration. Logical-DB owns persistence/replication; Moo
owns workspace transactions; Nova owns process/compute containment;
Secure-Authority owns authenticated authority; Sentinel owns external access.
References to pre-existing kernel/Braid identities and receipt protocols require
an explicit adapter decision, not a Larry replacement. No cross-repository
charter amendment is fabricated by this candidate.

### Runtime control flow: one request through verified completion

```mermaid
flowchart TD
  In["Human input or admitted scheduled event"] --> Accept["Authenticate scope; persist request ID and digest"]
  Accept --> Ack["Acknowledge durable admission, not success"]
  Ack --> Order["Queue or steer at a safe turn boundary"]
  Order --> Context["Build bounded context manifest and pinned obligations"]
  Context --> Plan["MI proposes plan, hypotheses and typed effects"]
  Plan --> Gate{"MD validates authority, versions, budget and contracts"}
  Gate -->|"Missing human decision"| Ask["Durable scoped question; expiry is not consent"]
  Ask -->|"Valid answer"| Gate
  Gate -->|"Invalid or unsupported"| Fail["Typed failure; preserve work and evidence"]
  Gate -->|"Admitted"| Reserve["Persist intent; reserve budget and fenced ownership"]
  Reserve --> Dispatch["Canonical owner executes; no ambient host bypass"]
  Dispatch --> Observe{"Outcome known?"}
  Observe -->|"No: timeout or crash"| Unknown["OutcomeUnknown; reconcile without blind retry"]
  Unknown -->|"Receiver evidence obtained"| Verify
  Unknown -->|"Still unknown"| Hold["Visible unresolved effect; no false completion"]
  Observe -->|"Yes"| Verify["Independent domain checks and evidence capture"]
  Verify --> Good{"Original obligations satisfied?"}
  Good -->|"No; retry admissible"| Plan
  Good -->|"No; exhausted or blocked"| Fail
  Good -->|"Yes"| Close["Settle descendants, resources and delivery obligation"]
  Close --> Receipt["Durable terminal receipt; publish truthful scoped result"]
  Stop["Cancellation or revocation"] --> Cancel["Stop new dispatch; drain or cancel owned work"]
  Cancel --> Observe
  Receipt --> Learn["Budgeted evaluation; quarantined learning proposals"]
  Learn --> Promote{"Independent promotion checks and standing scope"}
  Promote -->|"Accepted"| Future["Versioned strategy or skill for later runs"]
  Promote -->|"Rejected"| Archive["Retain failure evidence; no privilege increase"]
```
Cancellation enters reconciliation: it does not turn an accepted remote effect
into a non-event. A resumed request returns to its durable position, not always
to Plan. The terminal result distinguishes completed, failed, cancelled and
unresolved dispositions. The learning edge is not mandatory idle inference and
cannot rewrite the current run's grader.
