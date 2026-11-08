---
type: ADR
title: "ADR 0018: the authorization gate runs on lgwks_bot grants and proofs"
description: Why the reducer's policy and capability coverage is lgwks_bot::gate::GrantSet::admit, and why every mutating reach presents a cap::Auth proof leased for the pinned validity.
resource: /docs/adr/0018-gate-on-lgwks-bot-grants.md
tags: [larry, adr, runtime, gate, authority, capability]
generated: {by: claude-code, at: 2026-11-08}
verified: {by: claude-code, method: full nextest run before and after on the same replay seeds}
status: accepted
stale_after: 2027-02-08
sources: [larry#77, larry#68, logicalworks-crates#845, lgwks_bot 7.1.1 gate and cap, arxiv.org/abs/2606.28679]
---

# 0018: the authorization gate runs on lgwks_bot grants and proofs

Status: accepted, 2026-11-08 (larry#77, larry#68 row 4)

## Context

`replay::decide` checked policy and capability coverage with two `BTreeSet` find loops: a second coverage
rule beside `lgwks_bot::gate::GrantSet`, which larry#68 forbids. The runtime gate kept
`revalidate_at_boundary`, a public function nothing called, so the effect boundary had no authority check of
its own (SD-1, SD-6). Agent harnesses gate capabilities at admission but rarely re-check each call
(arXiv 2606.28679), and a grant must govern scope, expiry and revocation, not only scope.

## Decision

- `larry_contracts::replay::capability` names each `Scope` as a dotted `lgwks_bot::cap::Cap`
  (`larry.file.read`, `larry.file.write`, `larry.net.egress`, `larry.process.spawn`, `larry.model.invoke`).
- `decide` keeps its fixed order (validity, policy, capability, unit, budget). The policy and capability
  conjuncts are `GrantSet::admit` over the required scopes in sorted order, and the recorded `DenialReason`
  names the scope of the deficit's first shortage, so every recorded decision is bit-identical (C02 stays
  Larry's, as larry#68 allows).
- Admission mints a `cap::Auth` through `GrantSet::issue_at`, leased for `valid_until - observed_at` on the
  runtime's `lgwks_bot::clock::Clock`, and holds it on the live effect. Every mutating reach checks it after
  the fence and before the owner: a lapsed proof is `AuthorityExpired` in the tick's `denied`, and no owner is
  called. A retry's live authority is the fence and the proof together.
- `Auth` never serializes, so `restore` re-issues each recorded effect a proof for only the lease left at the
  restore instant; an intent with no recorded authorization holds none and is refused.
- `gate::revalidate_at_boundary` is deleted.

## Consequences

- While the lease holds, every decision and trace equals the old one; past it, nothing reaches an owner.
- No new crate: `gate` and `cap` compile with lgwks_bot's defaults off.
- When #845 publishes a supported surface for `gate` and `cap`, only `replay` and `gate` change.
- `replay::tests::larry_keeps_no_coverage_rule_of_its_own` fails if a find loop returns.
