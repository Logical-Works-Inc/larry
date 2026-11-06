---
type: Specification
title: "Evolution envelope, part 2: anti-churn rules and breakthroughs"
description: "Contract for adopting future models, execution substrates, storage mechanisms and breakthrough techniques without semantic rewrites."
resource: /architecture/larry-forge-spec/docs/06-evolution-envelope-2.md
tags: [larry, architecture, evolution, compatibility, hyperscale]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Derived from Larry PR #1 invariants and Forge source review; runtime NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
---

# Evolution envelope, part 2: anti-churn rules and breakthroughs

Part 1 is [06-evolution-envelope.md](06-evolution-envelope.md).

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
