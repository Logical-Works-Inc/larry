---
type: Research
title: "Forge source review for Rocco architecture"
description: "Bounded source inspection of the current Forge user path, coordinator, execution and lifecycle seams that can cause architectural churn."
resource: /architecture/rocco-forge-spec/docs/01-source-review.md
tags: [rocco, forge, source-review, churn]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Pinned GitHub source inspection; runtime NOT RUN"}
status: proposed-source-evidence
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/forge-harness/tree/11e9bbc2f3de9fa97d38acf47736628e530cff20"
---

# Forge source review — what can cause Rocco churn

Reviewed Forge revision: `11e9bbc2f3de9fa97d38acf47736628e530cff20`.

This review is intentionally about architectural seams, not a comprehensive bug list. The purpose is to identify where apparently local implementation choices can create repeated rewrites if Rocco does not freeze semantic ownership first.

## 1. Current user path and replacement risk

The supported CLI and TUI route through `forge-cli`. The TUI delegates to the same `execute_with_config_forwarded` run pipeline. That pipeline calls `setup_execution`, constructs a `HarnessRuntime`, loads memory, snapshots the working tree, races the run against interruption, applies post-run gates and persists a final audit result.

The important problem is not that this path is necessarily wrong. It is that its runtime construction still goes through `forge-harness/src/builder.rs`, whose first line marks it deprecated and says new code should land in `crates/forge-runtime`. A replacement architecture can therefore exist while the actual product route still executes the legacy semantic owner. Rocco must make **real-route reachability** an architecture acceptance gate.

### Churn prevention

- one canonical run-domain implementation;
- all surfaces call that domain implementation;
- replacements are not considered landed until entrypoint reachability is proven;
- no duplicate lifecycle semantics in CLI, TUI, server or worker layers;
- deprecated implementations must have an explicit removal gate and no new features.

## 2. Coordinator and effect semantics

Forge's newer `RunCoordinator` separates admission, task scope, execution, ledger and receipt types. That direction is structurally useful. However, the inspected execution backend demonstrates why types alone are not enough:

- ordinary task text is currently mapped to `/usr/bin/printf`;
- special task strings select timeout/large-output sentinel paths;
- execution appends the run idempotency key/spec to the ledger before process spawn;
- if the ledger already contains the key, the backend emits a synthetic success receipt whose stdout is the string `sealed`;
- coordinator finalization interprets stdout `sealed` as duplicate-sealed behavior;
- the success projection emits the literal verification description `execution verified`.

The architecture lesson is severe: **payload bytes must never impersonate protocol state**. A future tool, provider, model or process output could naturally contain the same sentinel. More importantly, a receipt label is not evidence that a domain verifier ran.

### Churn prevention

- protocol state uses enums/tagged records, not magic strings;
- task output is untrusted data and cannot control lifecycle state;
- effect observation, verification and finalization are separate typed transitions;
- every receipt points to evidence produced by the responsible verifier;
- duplicate admission returns the existing durable operation/result identity, not a fabricated normal success payload.

## 3. Append-before-effect is necessary but incomplete

The execution backend's append-before-spawn ordering is the correct direction for crash reasoning, but a durable intent record does not imply exactly-once external effects. A crash may occur after the receiver performed the effect and before local observation/finalization.

Rocco therefore needs an explicit `OutcomeUnknown` state and reconciliation contract. Idempotency keys are used when the receiver supports them; otherwise retries of ambiguous non-idempotent effects are forbidden until reconciliation resolves them.

### Churn prevention

Do not build business logic around “retry on timeout”. Build around effect identity, receiver capabilities and observed outcome state. This makes future providers and transports replaceable without changing safety semantics.

## 4. TaskScope concurrency has semantic races to close

The inspected `TaskScope` checks `closed`, checks capacity, spawns a thread and then pushes its handle into the tracked vector. Those operations are separate. A close/capacity race can therefore exist between the checks and handle registration.

This matters beyond the current implementation. If lifecycle closure is defined as “all descendants settled,” child admission and registration must be one semantic operation even if the mechanism changes from threads to Tokio tasks, remote workers or fleet leases.

### Required contract

- atomically reserve child capacity before execution starts;
- child ownership is durable/observable before the child can create effects;
- close transitions the scope to no-new-children before draining;
- no child can start in the gap between admission and tracking;
- timeout/panic leaves an explicit unresolved/failed descendant rather than silently shrinking the tracker.

## 5. Cancellation currently has multiple routes

CLI interruption races the harness run with `ctrl_c`, closes memory task state and persists an interrupted audit result. TUI `RunHandle` cancellation aborts the spawned outer Tokio task. These are not obviously the same semantic cleanup path.

Rocco should not let presentation surfaces own cancellation semantics. Cancellation must be a domain command that:

1. prevents new descendant/effect dispatch;
2. propagates to owned workers;
3. reconciles already-dispatched effects;
4. releases leases/reservations;
5. closes or marks unresolved descendants;
6. emits a durable terminal disposition only after those obligations settle.

## 6. Builder/config growth is a predictable churn source

The deprecated Forge builder accumulated many setters and resolution helpers because runtime assembly, policy, adapters, defaults and feature wiring converged into one construction surface. That creates broad blast radius and makes additions look like builder changes rather than contract changes.

Rocco should use typed configuration objects grouped by semantic owner, generated/validated against versioned schemas. Assembly should consume already-validated manifests. New model/provider/tool features should extend adapter registries, not the central run constructor.

## 7. Hidden fallback is architectural debt

The inspected builder has fallback behavior for current-directory resolution and run sandbox creation. Some fallbacks are useful UX; hidden semantic fallback is dangerous because it lets behavior vary based on incidental local state.

Rocco's rule should be:

- preference fallback is allowed when semantics are unchanged and provenance is visible;
- security/authority/storage/isolation fallback is forbidden unless the owner contract explicitly defines an equivalent degraded mode;
- cloud/provider/model switching never occurs solely because a preferred backend failed;
- the effective configuration is materialized and inspectable before effects.

## 8. Model architecture must remain outside MD semantics

Forge's current `AgentState` and `LifecycleAgent` assume a textual model/tool loop. Rocco should preserve the ability to use ordinary autoregressive models while avoiding a semantic dependency on textual chain-of-thought, a specific tokenizer, OpenAI-compatible tool calls or a particular model provider.

A model port should exchange typed observations/proposals/effects and accounting metadata. Future continuous-latent reasoning, multimodal state-space systems, symbolic/neural hybrids or locally learned policies should fit the same interface without changing authority/effect semantics.

## 9. Context and memory are separate from transcript mechanics

Forge carries in-run MD state plus separate mandatory memory. Rocco PR #1 already correctly holds memory implementation until Logical-DB readiness. Preserve that. Context projection, summarization and provider prompt-cache choices should never become the authoritative memory representation.

The durable source is typed events/records. Context is a versioned projection with a manifest describing selected records, omissions, policy revision, model/tokenizer adapter and digest.

## 10. Verification must bind to obligations

A generic `success: bool`, process exit code or model statement is insufficient. Rocco should freeze obligations at admission and amend them explicitly. Each obligation names its verifier/evidence class. Finalization can succeed only when required obligations are discharged or explicitly classified as unavailable/waived by authorized policy.

This allows new verification techniques—formal methods, learned judges, hardware attestation, external test systems—to improve evidence without rewriting completion semantics.

## 11. Fleet scale amplifies every local ambiguity

At one process, duplicate retries, loose cancellation, untracked children or implicit config fallback can look harmless. At millions of active runs they become retry storms, ghost work, cross-tenant leaks, stale authority and enormous cost amplification.

Rocco therefore needs conservation laws at every fan-out boundary:

- child budget <= reserved parent budget;
- retries consume the same operation budget rather than resetting it;
- speculation consumes explicitly reserved compute/token/network capacity;
- queue bytes and descriptors both have bounds;
- notification loss cannot lose durable work;
- cache keys include tenant/security/policy/model/template domains;
- backpressure happens before unbounded buffering.

## 12. Evidence classification

This document records source observations and derived architecture requirements. It does not claim that Forge's current implementation is production-unsafe, that every observed path is reachable in all configurations, or that Rocco's proposed contracts are proved. Those claims require runtime tests, fault injection, independent verification and real fleet evidence.
