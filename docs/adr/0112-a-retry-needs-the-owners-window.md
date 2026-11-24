---
type: ADR
title: "ADR 0112: A retry-safe effect is resent only under its owner's deduplication window"
description: Why the retry rule's inputs come from the key a reach sent and the owner's own window, and why a mutating reach with no live witness is refused.
resource: /docs/adr/0112-a-retry-needs-the-owners-window.md
tags: [larry, adr, runtime, retry, authority]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#197, larry#182, larry#461, ADR 0017]
---

# 0112: A retry-safe effect is resent only under its owner's deduplication window

Status: accepted, 2026-11-24 (larry#197, #182 defects 4 and 6). Amends ADR 0017.

## Context

- `retry::decide` passed one effect key as both `dispatched` and `proposed`, so lgwks_bot
  RQ-009's `IntentChanged` conjunct could never fire.
- It minted a `DeduplicationContract` for every `IdempotentRetrySafe` effect. The strategy is
  the submitter's claim. A proposal that declared it for an endpoint that does not deduplicate
  got three sends.
- `reach_owner` checked the authority proof only when a live entry existed. A reach with no
  live entry passed unchecked.
- A broker `spend` refusal after the owner ran went into `TickReport::fenced`, whose contract
  says nothing was sent.

## Decision

- `LiveEffect::dispatched` keeps the key the last mutating reach warranted. `decide` passes it
  as `dispatched`, or after a restore the key the intent names at its recorded attempt, and
  passes the digest of the intent a resend would carry as `proposed`.
- `EffectOwner::deduplication_window` (default `None`) is the only source of the contract. An
  effect that may have reached its owner is resent only inside that window. One that no reach
  sent carries `EffectEvidence::NotApplied`.
- A mutating reach with no live entry is denied `AuthorityExpired`. The retry rule's authority
  input is the fence's answer; the reach checks the admission proof itself.
- A `spend` refusal after the owner ran is the tick refusal `RuntimeError::Unspent`.

Rejected: keep minting the contract from the strategy and add an owner opt-out. A new owner
that forgets the opt-out fails open. A measurement that would reverse this: an owner whose
endpoint deduplicates and that cannot state a window. None is known.

## Consequences

- `ScriptedOwner` and the e65 `AtomicOwner` declare an unbounded window, so a test exercises
  the other conjuncts. `tests/sim_retry_window.rs` wraps the double in an owner that keeps the
  default and proves one send without a window and three with one, for 64 seeds.
- The owner still receives no proof at the call: larry#461.
