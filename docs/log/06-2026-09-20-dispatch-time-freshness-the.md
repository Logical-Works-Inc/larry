---
type: Log
title: log.md, part 6 of 15
description: Part 6 of /log.md, in order; the index lists every part.
resource: /docs/log/06-2026-09-20-dispatch-time-freshness-the.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-20: dispatch-time freshness — the second half of authorization

Director direction: do not import code from the sibling repositories; work the
next step from the specification and the recorded issues; deliver it in one
pass at the repository's Rust standard.

Semantic change: D48 splits authorization in two and requires the split to stay
visible — "a pure reproducible reducer plus separately authenticated freshness
at dispatch". The reducer already existed and its replay was already proven.
The freshness half did not exist at all: `gate::revalidate_at_boundary` was
written but never called, and there was no revocation fence, no security epoch
and no offline bound anywhere in the runtime. That is E71, and it is now
implemented.

`SecurityEpoch` is a monotone newtype in `rocco-contracts::authority`. It has no
decrement and no subtraction: the only movement it offers is `successor` and
`later_of`, both upward, which is what makes a replayed old device, grant or
policy snapshot a non-event instead of an authorization. `EffectIntent` carries
the `admission_epoch` its reservation was recorded under, so the recovery
contract D48 asks to be persisted before dispatch is in the journal. The epoch
is an argument to `WorkProposal::intent`, never a field of the proposal: a
command cannot declare the security generation it is admitted under, so a
hostile generator cannot name one that outlives a revocation (D42, E63).

`RevocationFence` holds three facts — the epoch, the withdrawn operations and
the offline exposure bound — and reads no clock. It is host-owned live state,
never folded from the journal and never recorded as a decision, so replay stays
bit-identical while freshness stays current. It is consulted from exactly one
place, `Runtime::reach_owner`, which both lifecycle paths funnel through, and it
gates **mutating** reaches only. A status query is deliberately not fenced: an
effect already dispatched when revocation landed must still be resolvable, or
revocation would leave a real external effect permanently unlabelled. That is
the "already-dispatched treatment" D48 requires, and it is enforced by shape
rather than by convention. A refused reach calls no owner, records no outcome,
publishes no receipt and moves no run state — the effect stays visibly open,
which is the only truthful state for one that was reserved and never sent.

Two design consequences were found by tests rather than by reasoning, and both
are now pinned. First, a fenced first dispatch is *indistinguishable from a
crash in the dispatch window*, because `EventKind` still has no "not
dispatched" fact; the runtime therefore queries such an effect and never
re-dispatches one whose strategy is not retry-safe. Second, an earlier
formulation excluded fenced effects from reconciliation, which would have made
the refusal silent — the effect would sit open forever with nothing in the tick
report saying why. Leaving it in costs one pure check per open effect per tick
and keeps the refusal visible.

Affected IDs: none added. R01–R96, D01–D53, E01–E80, C00–C12 and A1–A9 are
unchanged; the inventory stays 96/53/80/13/9. This implements R87, D48 and E71
and redefines nothing.

Evidence: 29 new tests, 161 total (was 132). E71 is carried by 15 integration
falsifiers in `crates/rocco-runtime/tests/fence.rs`, 11 unit tests in
`crates/rocco-runtime/src/fence.rs` and 3 in `rocco-contracts::authority`,
covering revocation during queue, the ordering between the durable intent and
the owner call, partition, sleep, clock rollback, a replayed older snapshot,
epoch exhaustion, epoch exhaustion at the runtime boundary, the retry-versus-
query asymmetry, restore across a generation change, a zero bound, and the
reservation a fenced effect holds. `cargo build --bins --locked` passed;
`cargo test --workspace --all-targets --locked` passed 161;
`cargo clippy --workspace --all-targets --locked -- -D warnings` passed;
`cargo fmt --all -- --check` passed; `lgwks-deps check .` returned `OK . — 1
semantic approvals, every authored external edge is owned` with **no new
registered edge**; `python3 tools/check_governance.py` PASS on 96/53/80/13/9;
`python3 -m unittest discover -s tests` passed 55; `git diff --check` clean.
Reranker receipts `tools/receipts/20260920-dispatch-fence-pre-exec.json` and
`-post-exec.json`. Hosted CI has not run for this change.

Unresolved, and not claimed: no authenticated authority source supplies the
fence. Its facts are host-asserted in this slice, exactly as policy, capability
and validity are pinned, so the revocation *mechanism* is proven and the
*provenance* of revocation facts is not integrated. A fenced effect keeps its
reserved budget, because the reservation precedes the fence by design and
releasing it would need a journal fact the closed vocabulary does not have; the
direction fails closed and is pinned by a test. `gate::revalidate_at_boundary`
remains uncalled: it is the grant-level check, and no `Grant` is threaded
through the runtime yet — a different work package, not silently absorbed into
this one. The offline bound is a declared Cell 05-style number equal to one
tick's worth of admissions, not a measured exposure figure. E63 and E65 are
still NOT RUN as acceptance cases, so the C02 exit gate is not met.
