---
type: Research
title: "Forge source review, part 2: fallback, models, context, verification and scale"
description: "Bounded source inspection of the current Forge user path, coordinator, execution and lifecycle seams that can cause architectural churn."
resource: /architecture/larry-forge-spec/docs/01-source-review-2.md
tags: [larry, forge, source-review, churn]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Pinned GitHub source inspection; runtime NOT RUN"}
status: proposed-source-evidence
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/forge-harness/tree/11e9bbc2f3de9fa97d38acf47736628e530cff20"
---

# Forge source review, part 2: fallback, models, context, verification and scale

Part 1 is [01-source-review.md](01-source-review.md).

## 7. Hidden fallback is architectural debt

The inspected builder has fallback behavior for current-directory resolution and run sandbox creation. Some fallbacks are useful UX; hidden semantic fallback is dangerous because it lets behavior vary based on incidental local state.

Larry's rule should be:

- preference fallback is allowed when semantics are unchanged and provenance is visible;
- security/authority/storage/isolation fallback is forbidden unless the owner contract explicitly defines an equivalent degraded mode;
- cloud/provider/model switching never occurs solely because a preferred backend failed;
- the effective configuration is materialized and inspectable before effects.

## 8. Model architecture must remain outside MD semantics

Forge's current `AgentState` and `LifecycleAgent` assume a textual model/tool loop. Larry should preserve the ability to use ordinary autoregressive models while avoiding a semantic dependency on textual chain-of-thought, a specific tokenizer, OpenAI-compatible tool calls or a particular model provider.

A model port should exchange typed observations/proposals/effects and accounting metadata. Future continuous-latent reasoning, multimodal state-space systems, symbolic/neural hybrids or locally learned policies should fit the same interface without changing authority/effect semantics.

## 9. Context and memory are separate from transcript mechanics

Forge carries in-run MD state plus separate mandatory memory. Larry PR #1 already correctly holds memory implementation until Logical-DB readiness. Preserve that. Context projection, summarization and provider prompt-cache choices should never become the authoritative memory representation.

The durable source is typed events/records. Context is a versioned projection with a manifest describing selected records, omissions, policy revision, model/tokenizer adapter and digest.

## 10. Verification must bind to obligations

A generic `success: bool`, process exit code or model statement is insufficient. Larry should freeze obligations at admission and amend them explicitly. Each obligation names its verifier/evidence class. Finalization can succeed only when required obligations are discharged or explicitly classified as unavailable/waived by authorized policy.

This allows new verification techniques—formal methods, learned judges, hardware attestation, external test systems—to improve evidence without rewriting completion semantics.

## 11. Fleet scale amplifies every local ambiguity

At one process, duplicate retries, loose cancellation, untracked children or implicit config fallback can look harmless. At millions of active runs they become retry storms, ghost work, cross-tenant leaks, stale authority and enormous cost amplification.

Larry therefore needs conservation laws at every fan-out boundary:

- child budget <= reserved parent budget;
- retries consume the same operation budget rather than resetting it;
- speculation consumes explicitly reserved compute/token/network capacity;
- queue bytes and descriptors both have bounds;
- notification loss cannot lose durable work;
- cache keys include tenant/security/policy/model/template domains;
- backpressure happens before unbounded buffering.

## 12. Evidence classification

The review records source observations and derived architecture requirements. It does not claim that Forge's current implementation is production-unsafe, that every observed path is reachable in all configurations, or that Larry's proposed contracts are proved. Those claims require runtime tests, fault injection, independent verification and real fleet evidence.
