---
type: Design
title: specification.md, part 12 of 18
description: Part 12 of /specification.md, in order; the index lists every part.
resource: /docs/specification/12-d50-consent-appears-only-at-meaningful.md
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
### D50 — consent appears only at meaningful boundary changes

Choose standing contracts for routine reads, drafts and reversible local work.
The user's ordinary Save, Send, Publish or Integrate action may be the exact
declassification/authorization event when the trusted UI binds the displayed
bytes, destination, account and revision. Changed material facts invalidate that
authorization; unchanged facts do not prompt again. E75; owner trusted surface.

### D51 — the 8B survival gate has containment and usefulness lanes

Choose one pinned artifact of at most 8.5 billion parameters and configuration,
plus a hostile proposal generator. Security lane target: zero unauthorized
effects, forbidden disclosures, scope crossings, authority expansions or state
resurrections over the full bounded security inventory for both drivers. Useful-
work lane target: at least 90% verified completion on a frozen supported benign
suite and no more than five percentage points below the matched same-model
isolated baseline. Zero repeat approvals within unchanged standing authority.
No stronger model or hidden cloud fallback. On the Ryzen 3 4300U/8 GiB profile,
candidate local admission p95 is at most 50 ms excluding provider inference;
the 4 GiB profile may reject model residency but must preserve state and remain
operable with an explicitly selected feasible provider. All targets are NOT RUN.
E76/E77; owners evaluation, model adapter and effect boundary.

### D52 — defenses must fail closed under mutation, not merely agree

Choose external outcome observers, fault controls and defense-removal mutations
for authority, IFC, containment, egress, lifecycle, updates and UI. A suite that
does not fail when its enforcement is removed is rejected. Separately measure
explicit flows and declared covert channels; jitter or batching is adopted only
against a measured threat and human/latency budget. E78/E79; owner independent
verification.

### D53 — make the strongest honest promise

Choose this claim: even if every model response is adversarial, Rocco preserves
declared authority, information-flow, isolation, resource and recovery invariants
through supported paths and stated platform assumptions. Do not claim every
authorized activity is harmless, every fact true, remote revocation instantaneous
offline, disclosure reversible, covert channels absent or a compromised local
root constrained. Abuse and content policy may further narrow actions but never
grant them. E80; owner governance and acceptance.

## Cell 04 — state, effects and proof obligations

**Run states (candidate algebra):** Draft -> Admitted -> Running -> Reconciling
-> Verifying -> Finalizing -> Completed. Branches: AwaitingAuthority,
AwaitingDependency, Conflict, Cancelling, Failed, RolledBack, Cancelled,
OutcomeUnknown. Durable terminal receipt is immutable; later evidence appends a
superseding resolution, not an in-place success rewrite. Cancellation requested
is not cancellation completed. Do not label partial external effects RolledBack.

**Effect protocol:** validate/authorize -> reserve budget and write intent ->
dispatch with stable operation ID -> observe/reconcile -> persist outcome ->
verify -> integrate/finalize. Database transactions cannot atomically include an
arbitrary network effect. For external endpoints: reuse idempotency keys and
query status when supported; otherwise an ambiguous timeout/crash remains
OutcomeUnknown and is NOT blindly retried. Compensation is a new authorized
effect, not time travel. Exactly-once is not a universal claim.

**Ordering:** per-run monotonic sequence plus causal parents across runs. Stable
command IDs suppress duplicate submission; same ID/different content is an
error. Admission and reservation require atomic compare-and-set or equivalent
owner guarantee. A host-local revision check is not cross-replica consensus.
No local CRDT merge may turn two permits into one imaginary exclusive lease.

**Authority equation (design obligation):** child grants are a subset of parent
grants intersected with current resource policy, budget and expiry. Validate
again at the effect boundary. A worker cannot use its own observed timestamp to
extend a lease. Immediate remote revocation while disconnected is impossible;
candidate policy is no new shared/external privileged effect without fresh
authority, and explicitly leased low-risk local work until expiry. Never offer
'instant offline revocation' as an SLA. Available local authority can renew
only within its own standing delegated limits.

**Privacy/replay conflict:** evidence retention is scoped and time-bounded. Keep
content separate from minimal audit metadata. User-requested removal/retraction
invalidates derived context/skills/caches; retention obligations are disclosed.
Do not promise erasure from an external recipient or a trained model. Cryptographic
erasure, legal holds and retention enforcement belong to owner-reviewed policy;
this scratchpad authorizes no permanent deletion.

**Proof obligations before dependent code:** P1 grant attenuation/revocation;
P2 atomic event+reservation and fenced effect ownership; P3 uncertain external
outcome reconciliation; P4 bounded context compilation preserving commitments;
P5 deterministic replay equivalence under pinned inputs; P6 termination/resource
bounds including native callbacks and child processes; P7 no cross-scope
information flow through caches/learning; P8 migration compatibility and rollback.
Use model checking for finite reducer/lease races, property tests for encodings,
real fault injection for storage/external boundaries and independent review.
None of P1–P8 is proved here.
