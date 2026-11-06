---
type: Specification
title: "Larry protocol, state and proof obligations"
description: "Typed identities, atomic boundaries and proof hierarchy for a durable assistant runtime."
resource: /architecture/larry-forge-spec/docs/03-protocol-and-proof.md
tags: [larry, protocol, state-machine, proof]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Derived from Larry PR #1 and Forge source review; runtime NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
---

# Protocol and proof obligations

## Canonical identities

At minimum the core protocol distinguishes `TenantId`, `PrincipalId`, `EnvironmentId`, `RequestId`, `RunId`, `TurnId`, `InputId`, `ObjectiveId`, `ObligationId`, `EffectId`, `QuestionId`, `ArtifactId`, `GrantId`, `PolicyRevision`, `ContextEpoch`, `ToolGeneration`, `ModelAdapterRevision` and `ReceiptId`.

Identifiers are opaque values. User-visible labels are never authorization identities.

## Run lifecycle

Candidate domain lifecycle:

`Draft -> Admitted -> Running -> Reconciling -> Verifying -> Finalizing -> Completed`

Side states: `AwaitingAuthority`, `AwaitingDependency`, `Conflict`, `Cancelling`, `Cancelled`, `Failed`, `OutcomeUnknown`.

A state transition is valid only when its preconditions and durable evidence are present. The reducer is deterministic over recorded events plus pinned reducer/policy versions.

## Atomic boundaries

The following operations require a named linearization point or equivalent owner guarantee:

1. request identity + digest + durable admission + resource reservation;
2. child capacity reservation + ownership registration;
3. effect reservation + append-before-dispatch record;
4. background-memory applied cursor + durable memory result;
5. configuration effective-revision activation;
6. receipt finalization + terminal lifecycle disposition;
7. schedule occurrence identity + admission reservation.

If an owner cannot provide the required atomicity, the feature remains blocked or must use a weaker explicitly specified protocol. Local process locks are not evidence of distributed fencing.

## Effect protocol

`validate -> authorize -> reserve -> persist intent -> dispatch -> observe/reconcile -> persist outcome -> verify -> integrate/finalize`

Database transactions cannot atomically include arbitrary remote effects. Receiver idempotency keys and queryable status are used when supported. Otherwise ambiguous interruption yields `OutcomeUnknown`.

Compensation is a new effect with its own authority and evidence. It does not rewrite the original effect as if it never happened.

## Replay modes

1. **Observation replay:** render recorded events/results; zero new effects.
2. **Control replay:** fold historical events with pinned reducer/policy/clock/random inputs; zero new effects.
3. **Fresh experiment:** rerun model/tools in isolated environment; may differ and is a new run/effect lineage.

Old grants demonstrate historical authority only. Replay never reactivates them.

## Proof hierarchy

Evidence strength is ordered by the claim being made, not by a single universal score:

- type/schema check;
- unit/property/model test;
- integration fixture;
- actual crash/fault injection;
- second real adapter conformance;
- physical-device test;
- multi-tenant/noisy-neighbor test;
- fleet load/recovery test;
- independent evaluator/human evidence where applicable.

A stronger-looking layer cannot substitute for a missing lower semantic proof. Example: million-user synthetic load cannot establish correct external-effect reconciliation if that protocol has never survived one real crash.

## Proof obligations inherited from PR #1

P1 grant attenuation/revocation; P2 atomic event+reservation and fenced effect ownership; P3 uncertain external outcome reconciliation; P4 bounded context compilation preserving commitments; P5 deterministic replay equivalence under pinned inputs; P6 termination/resource bounds including callbacks/children; P7 no cross-scope flow through caches/learning; P8 migration compatibility/rollback.

The machine contracts add an explicit **P9 real-route reachability obligation**: the candidate implementation must be shown reachable from every declared supported entrypoint before it can replace an older semantic owner.

## Protocol evolution

- additive optional fields may be introduced when older readers preserve/ignore them per schema;
- new required semantics require major-version negotiation;
- in-flight runs pin reducer/protocol semantics until a declared checkpoint;
- migrations are resumable and idempotent;
- stale replicas beyond compatibility/retention boundaries must rebootstrap rather than resurrect old meaning;
- unknown required states fail closed and remain inspectable.

No protocol evolution may reinterpret historical events silently.
