---
type: Specification
title: "Larry change and delivery contract"
description: "Architecture change packets, promotion gates and delivery sequencing to prevent repeated rediscovery."
resource: /architecture/larry-forge-spec/docs/05-change-and-delivery.md
tags: [larry, governance, delivery, architecture]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Derived from Larry governance and source review; implementation NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
---

# Change and delivery contract

## Machine change packet

Any change affecting architecture semantics must include a machine-readable packet with:

- change ID and owner;
- canonical requirements/decisions/falsifiers touched;
- semantic surface changed;
- old/new protocol or policy revisions;
- compatibility class: additive / behavior-preserving replacement / migration / breaking;
- migration and rollback boundary;
- resource and security deltas;
- affected entrypoints and proof of real-route reachability;
- required fault/conformance/performance tests;
- evidence produced, evidence still missing and explicit unknowns.

A green unit test suite cannot substitute for this packet when semantics changed.

## Promotion ladder

1. **Planned:** documented contract only.
2. **Present:** implementation exists in code.
3. **Exercised:** relevant path executed on exact commit.
4. **Fault-exercised:** applicable crash/race/negative controls run.
5. **Integrated:** canonical real entrypoints use the implementation.
6. **Independently evidenced:** external/independent verifier confirms required outcome.
7. **Scale evidenced:** physical/fleet acceptance where applicable.
8. **Safe to replace/remove predecessor:** migration and rollback conditions met.

Never collapse these states into “done”.

## C00–C12 sequence

Retain PR #1's execution-cell DAG. The machine-contract work tightens its promotion criteria:

- C00 freezes semantic invariants and explicit exceptions.
- C01 proves sibling owner contracts/revision compatibility.
- C02 establishes typed command/run/effect states and formal finite-state/race models.
- C03 remains blocked on Logical-DB readiness for real memory implementation.
- C04 proves one real effect end-to-end including ambiguous-outcome recovery and receipt evidence.
- C05 proves actual supported CLI/TUI route reaches the canonical domain path.
- C06 proves bounded delegation/schedules/speculation without budget multiplication.
- C07 proves workflow/tool/skill lifecycle and quarantine.
- C08 proves correction/learning promotion without authority/evaluator mutation.
- C09 independently evaluates task/persona/cost/security lanes.
- C10 proves cross-device/migration semantics.
- C11 proves physical baseline, multi-tenant isolation and hyperscale workload.
- C12 packages supported release surfaces and retained evidence.

## Anti-churn delivery rules

- Do not implement the same semantic state machine in multiple surfaces.
- Do not create a replacement architecture without removing/migrating the user route to it.
- Do not let compatibility adapters become accidental canonical logic.
- Generate schemas/types from one canonical contract where practical; copied protocol definitions require drift checks.
- Any workaround that violates an invariant must be explicit technical debt with an expiry/falsifier, not a hidden fallback.
- Deleted/deprecated semantic owners get grep/codegraph reachability gates preventing regrowth.
- Benchmark-specific code may live in eval tooling, never production control flow.

## Delivery endpoint

Repository governance remains authoritative. If target branch is not protected, delivery stops at a verified open PR. If protection/rules are active, satisfy the actual server-enforced checks before merge. Local validation is never relabeled hosted CI.

## Evidence retention

Retain failed attempts and counterexamples that changed the design. An architectural fix without the motivating falsifier invites regression and future rediscovery.
