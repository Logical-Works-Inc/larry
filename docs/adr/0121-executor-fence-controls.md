---
type: ADR
title: "ADR 0121: Executor host fence controls and E35 shipment boundary"
description: Why revoke, epoch advance and re-authentication run from the executor host path and E35 stays unshipped.
resource: /docs/adr/0121-executor-fence-controls.md
tags: [larry, adr, runtime, fence, executor]
generated: {by: opencode, at: 2026-11-26}
verified: {by: opencode, method: cargo nextest run and cargo clippy on toronto, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#215, larry#90, larry#76, larry#197, ADR 0024]
---

# ADR 0121: Executor host fence controls and E35 shipment boundary

Status: accepted, 2026-11-26 (larry#215).

## Context

Runtime authority controls had no production caller: the executor never
withdrew, advanced or re-authenticated, so a restored fence's unconfirmed
bound never cleared in production. Every unmapped broker refusal collapsed
into one of two values, attempt zero was coerced to one, and an undecodable
accepted command was dropped at restore with no event. E35 catch-up has no
owner until `larry serve` owns a durable runtime journal route (larry#90).

## Decision

`larry exec` reads one typed `LARRY_FENCE_CONTROL` host action. It applies the
action after the command is durably admitted and before `Runtime::tick` can
reach the owner. The actions are `revoke`, `advance`, and
`authenticate:<epoch>`. They use the runtime's existing `RevocationFence`.

`submit_displayed` stays the queue door: it enqueues the display for
tick-time admission, preserving submission order and the held-submit
recheck (E65, larry#332). The production ingress (`submit_protocol`)
admits synchronously for crash-safe acknowledgement (E28) and validates
the same display at admission. Routing ingress through the queue door, or
the queue door through synchronous admission, breaks the held-submit
timing two E65 journeys pin, so the doors stay separate until a serve
route owns the ordering (larry#90).

Attempt zero is invalid. Restore propagates a codec refusal when an accepted
command cannot decode to its target. An unknown broker refusal keeps its cause
text in a typed Larry fence variant.

E35 remains unshipped. `larry serve` does not yet own a durable runtime journal
route. The consumer API stays tied to the runtime until the serve owner exists.
No in-memory catch-up route is introduced. The tenant catch-up index extends
on read: appends never hash, so no shipped caller is required to justify the
hot path. The first read after new facts hashes exactly those facts, once,
and subscribes validate against the extended index, so every E35 verdict
stands (peer review, larry#475).

## Consequences

Authority controls are reachable from the shipped binary and fail closed at
corrupt or unclassifiable boundaries. `submit_displayed` stays the queue
door for E65 hold timing; the production ingress keeps synchronous
admission. Journal appends pay no BLAKE3 for catch-up; the first read after
an append covers it. E35 stays unshipped pending the serve journal route.

## Proof

`tests/executor_fence_journey.rs` drives revoke, epoch advance and
re-authentication through `larry exec`. Runtime tests cover attempt zero and
malformed accepted history. `consumer::tests::appends_leave_the_tenant_index_cold_until_a_read`
pins the lazy index with the E35 suites proving unchanged reads. `cargo nextest`,
clippy and the binary journey are the required gates for this decision.
