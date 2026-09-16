---
type: Specification
title: "Rocco evolution envelope — build semantic foundations once"
description: "Contract for adopting future models, execution substrates, storage mechanisms and breakthrough techniques without semantic rewrites."
resource: /architecture/rocco-forge-spec/docs/06-evolution-envelope.md
tags: [rocco, architecture, evolution, compatibility, hyperscale]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Derived from Rocco PR #1 invariants and Forge source review; runtime NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Rocco/pull/1"
---

# Evolution envelope: build the semantics once

The architecture MUST optimize for **replacement of mechanisms without replacement of meaning**. “Build once” does not mean freezing algorithms, models, deployment topology, storage engines or hardware. It means stabilizing the semantic waist that every future implementation must satisfy.

## Semantic invariants

Unless explicitly superseded by human architecture authority:

1. Requests, runs, effects, obligations, artifacts, grants, revisions and receipts retain stable typed identities and causal lineage.
2. Model output is never authority; effects are admitted against current scoped capability state owned by Secure-Authority.
3. Proposed/reserved/dispatched/observed/outcome-unknown/verified/finalized remain distinguishable.
4. Acknowledged commitments survive process loss; durable state remains owned by Logical-DB and workers remain reconstructible.
5. Sentinel owns internet boundary, Nova containment, Moo workspace mutation, Logical-DB persistence/memory, Secure-Authority authenticated authority and Logicalworks-Crates foundation primitives.
6. Completion is obligation discharge backed by independently inspectable evidence, not model prose/exit code/provider finish reason.
7. Tenant/principal/environment scope follows caches, context, tools, effects, learning and observability.
8. Child work consumes bounded parent resources; retries/speculation/delegation/background work cannot manufacture budget.
9. Durable objects declare schema/protocol/policy revisions; unknown required semantics fail closed and migrations are resumable.
10. Major authority, irreversible, training, recipient and ownership changes remain explicit human-policy decisions.

These invariants form Rocco's **semantic waist**. Implementations above and below it are replaceable.

## Replaceable mechanism classes

Keep these outside core semantic assumptions:

- frontier/local model family, tokenizer, reasoning architecture, multimodal codec, inference transport and serving stack;
- context ranking/compaction/retrieval/embedding/cache implementation;
- planner/search/abductive reasoner/bandit/RL/learned verifier/skill synthesis mechanism;
- workflow authoring syntax/compiler frontend;
- Nova-backed VM/Wasm/native/remote execution profile;
- Logical-DB storage layout/index/replication mechanism;
- fleet scheduler, sharding, queue implementation, transport and process topology;
- UI renderer/device presentation;
- evaluation models and benchmark suites while evidence provenance/frozen-oracle semantics remain intact.

A breakthrough that cannot fit an extension seam triggers architecture review because it changes semantics—not because it is novel.

## Capability negotiation

Every replaceable component advertises a machine-readable manifest with protocol revision, required/optional capabilities, effect classes, cancellation/streaming semantics, limits, locality/confidentiality/egress properties, accounting, migration/import/export compatibility and evidence capabilities.

Negotiate before mutation. Missing required capability => typed refusal. Optional absence => only explicitly specified degraded behavior. Core semantics must not branch on stringly provider/model names.

## Extension safety classes

- **P Pure:** deterministic/pure transforms; speculative within resource budget.
- **O Observational:** reads with confidentiality/freshness/cost/external-observability properties.
- **E Effectful:** mutation/external visibility; requires authority, stable effect identity and reconciliation.
- **A Authority-changing:** only canonical authority owner may commit.
- **L Learning:** changes future behavior; requires train/eval separation, provenance and rollback; cannot enlarge authority.
- **M Migration:** transforms durable semantics; requires compatibility plan and resumable checkpoints.

Novel AI techniques do not bypass classification.

## Evolution gates

Promote a future model/runtime/storage/reasoning breakthrough only after applicable:

1. conformance to the same semantic contract and negative cases;
2. differential old/candidate traces with every semantic difference classified;
3. crash/network/power interruption around commit boundaries;
4. supported version-skew testing;
5. tenant-isolation and cache/learning leakage tests;
6. resource-amplification checks across retry/speculation/delegation/background work;
7. full-path performance/resource accounting;
8. migration/rollback/stale-replica validation;
9. independent evidence for claimed improvement;
10. proof candidate is reachable from real supported entrypoints.

Dead replacement architecture does not count.

## Hyperscale invariants

Scaling to >1,000,000 active concurrent users does not change meaning:

- no global total-order requirement;
- tenant/resource/run partitioning with causal lineage;
- one fenced execution authority per exclusive effect scope;
- backpressure before unbounded queues;
- admission quotas at every fan-out;
- retry budgets conserved across layers;
- noisy-neighbor isolation;
- notifications are hints, durable history authoritative;
- caches disposable and security-domain keyed;
- scheduled/background work occurrence-identified and deduplicable;
- overload may reject but never silently lose acknowledged work or weaken authority/evidence semantics.

## Anti-churn rules

Architecture failure even when tests pass:

- two code paths independently implement run/effect semantics;
- copied schemas instead of canonical generated/imported contracts;
- UI/CLI/server independently invent lifecycle state;
- sentinel strings/stdout encode control state;
- configuration precedence duplicated in consumers;
- retry counters reset independently across layers;
- deprecated path remains the supported user route after replacement is declared authoritative;
- compatibility adapter silently becomes canonical business logic;
- benchmark-specific branches enter production;
- migration reinterprets old events instead of pinning historical reducer semantics.

## Breakthrough examples

**Continuous-latent/non-token reasoning:** admissible behind the model capability contract if inputs, proposals, effects, accounting, cancellation and evidence remain typed. Core must not require textual chain-of-thought.

**Learned tool routing:** may propose routes; deterministic control still owns authority/resource/effect admission.

**Persistent neural memory:** may become a Logical-DB-backed representation after owner approval; never an untracked second memory authority.

**Secure hardware/enclaves:** Nova/Secure-Authority adapters may exploit attestation; hardware evidence strengthens but does not redefine effect/grant identity.

**Novel distributed databases/CRDTs:** may implement Logical-DB internals; convergence still does not create exclusive effect fencing.

**Autonomous self-improvement:** generated policies/skills/models are candidate artifacts; promotion is separately versioned and independently evaluated and cannot modify its own authorization/evaluator to pass.

## Evidence status

This specification does not prove future-proofness, hyperscale performance or runtime correctness. It establishes the intended design discipline: mechanisms may evolve aggressively; semantic changes must be explicit architecture decisions rather than accidental churn.
