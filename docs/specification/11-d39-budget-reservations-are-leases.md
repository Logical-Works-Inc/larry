---
type: Design
title: specification.md, part 11 of 18
description: Part 11 of /specification.md, in order; the index lists every part.
resource: /docs/specification/11-d39-budget-reservations-are-leases.md
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
### D39 — budget reservations are leases

Choose leases with explicit TTL and renewal-within-standing-limits over
indefinite reservations. Expiry and reclaim flow through the ordinary
reconciliation machinery: a reclaim is an admitted effect, the terminated
worker cannot double-spend, and the fleet aggregate stays inside the parent
budget. No separate janitor authority is created. E59; owner scheduler and
budget ledger.

### D40 — memory edge semantics are explicit

Choose: superseded records freeze their decay clock and become provenance,
not live context; pinned commitments close only through an authorized
transition, so machine-derived supersession never silently unpins;
compaction's reversibility claim is declared exactly — reconstructable from
epoch events for audit, or atomic-only, and the choice is tested; compacted
nodes leave cryptographic tombstones so provenance chains survive payload
removal. E60; owner memory semantics over Logical-DB.

### D41 — evaluation hardening and auditable inventory

Choose a contamination-controlled private task suite for the same-model
lane; public-suite scores are reported separately with declared
contamination limits. Add process-level recovery scoring (error
localization and repair, not only final outcome), a revocation-enforcement
falsifier for memory, a post-failure honesty falsifier for tool reporting,
and interacting-fault combined journeys in addition to single-fault cases.
The falsifier inventory carries a derivation note mapping fault modes to
effect classes so its size is auditable rather than asserted. E61/E62;
owner independent evaluation.

### D42 — model obedience is unnecessary for containment

Choose a reference-monitor contract in which the model can be replaced byte for
byte by a hostile proposal generator. Security facts come only from protected
runtime state and authenticated owner observations. Models, prompts, memory,
plugins and tool payloads stay outside the trusted computing base; their failure
may prevent useful work but cannot widen authority. E63; owner Larry admission.

### D43 — bind resolved effects and mediate them at their owners

Choose canonical effect parsing and exact binding over approval of persuasive
descriptions. Larry admits; Secure-Authority supplies authenticated attenuated
authority; Nova contains processes, inherited handles, filesystem exposure, IPC
and resources; Sentinel mediates actual egress; Moo resolves and integrates
workspace objects; Logical-DB durably fences state. Wasmtime or another engine
may be a Nova mechanism, never a second Larry sandbox owner. Missing Nova hostile-
code containment is `Unsupported`, including when `NativeTrusted` exists. E64/E65.

### D44 — provenance and compartment-scoped information flow are distinct

Choose protected labels for origin, integrity, confidentiality, ownership and
authority, propagated conservatively over everything an opaque model invocation
received, including retained state and control choices. A model cannot assert or
remove those labels. Enforce at sinks; release is a narrow authorized transform
or exact human declassification. Keep invocations and memory compartments small
enough to avoid global taint explosion; reset or rebuild a compartment without
discarding durable obligations. E66/E67; owner Larry IFC with domain sinks.

### D45 — authorized compositions consume shared impact envelopes

Choose persistent conservation across run, principal, governed credential,
tenant, destination and rolling time window for recipients, schedules,
concurrency, bytes, compute, messages and spend. Retries, aliases, workers and
restarts cannot reset the relevant counter. Expansion needs independent bounded
authority. This contains supported campaigns without claiming a local executable
can morally govern an owner who replaces it or acts elsewhere. E68; owner policy
and durable budget ledger.

### D46 — secrets are narrow operations and egress is the actual connection

Choose typed broker operations rather than credential disclosure. Bind broker
use to operation, audience, account, recipient and fence. Sentinel revalidates
resolution, redirects, subresources, reconnects and protocol; renderers cannot
initiate hidden requests, clipboard writes or approval-like controls. E69;
owners Secure-Authority secret broker and Sentinel egress.

### D47 — collaboration never merges security domains

Choose access filtering before retrieval and ranking, with scope on existence
metadata, embeddings, summaries, KV caches, learning records, artifacts and
notifications. Messages carry information-flow restrictions, not grants.
Consensus and copying cannot manufacture release authority. E70; owner Larry
collaboration over Secure-Authority and Logical-DB.

### D48 — freshness and recovery are executable-owner fences

Choose a pure reproducible reducer plus separately authenticated freshness at
dispatch. Define revocation's linearization point, offline exposure bound and
already-dispatched treatment. Persist intent, reservation and recovery contract
before dispatch; owner-supported idempotency/reconciliation determines outcome.
Whole-state rollback detection requires protected state or an independent
witness, not a self-contained local hash chain. E71/E72; owner effect lifecycle.

### D49 — updates require role separation and anti-rollback state

Choose exact artifact identity, threshold roles, security-version floors,
compatibility checks, staged activation and protected recovery state. A plugin
cannot update its own monitor. Recovery may restore older application behavior
without lowering the security epoch. State honestly that compromise of the
trusted recovery threshold needs an independently trustworthy recovery path.
E73/E74; owner release and platform security.
