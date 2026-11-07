---
type: ADR
title: docs/architecture/runtime.md, part 1 of 3
description: Part 1 of /docs/architecture/runtime.md, in order; the index lists every part.
resource: /docs/architecture/runtime/01-adr-r01-the-journal-is-authoritative.md
tags: [rocco, runtime, determinism, replay, ecs, bevy]
generated: {by: claude-code, at: 2026-09-20}
verified: {by: claude-code, method: cargo build/test/clippy/fmt, lgwks-deps check, check_governance.py, unittest, all local; no hosted CI}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-20
sources: [/specification.md, /AGENTS.md, /contract/APPROVED.toml, /log.md, Director decisions 2026-09-20]
---
# ADR-R01 — The journal is authoritative; everything else is a fold

Decision: an append-only, bounded journal is the only source of truth. The
run-state view, the effect phases and the receipt list are a fold over it.
`runtime::replay` rebuilds a projection from a journal alone and consults no
live state, so recovery is a re-fold and a projection is discardable at any
moment.

This is D04 taken literally — "event history is authoritative; graph and context
are projections" — and it is the property that makes every determinism argument
below cheap. A test discards the live projection, re-folds it from the journal,
and asserts the two agree.

Rejected: mutable world state with the journal as an audit trail. That inverts
D04 and makes replay correctness depend on the world's iteration order rather
than on the log.

# ADR-R02 — The tick is a fixed, bounded stage sequence

Decision: one tick samples the observed instant exactly once and then runs four
stages in a fixed order — admit, reconcile, cancel, sample. Every stage is
bounded by an explicit constant; every collection is a `BTreeMap` or an
insertion-ordered `Vec`; no stage consults the wall clock except the first.

The single pinned instant is E30/D36: expiry is a pinned input, so an expired
approval is a typed denial and never consent by timeout. One instant per tick
also means every authorization made in a tick is comparable, because they were
all decided against the same observed time.

Rejected: reading `SystemTime::now()` at each decision site. That is
unreplayable by construction — the same journal would re-derive different
decisions on a different day.

# ADR-R03 — No dispatch without a durable intent carrying its own replay record

Decision: the intent is written to the journal *before* the owner is called, and
it carries the pinned authorization inputs and the decision they produced. A
crash inside that window therefore leaves the effect reading as `Reserved` —
"may have happened" — never as a non-event.

`contracts::event::EventKind` is a closed five-variant vocabulary owned by the
specification, and it has no "dispatched" variant. That absence is load-bearing
and this work package did **not** extend the vocabulary to get a more convenient
picture. An effect that crashed mid-dispatch is genuinely ambiguous, and the
journal's silence about dispatch is what encodes that.

Rejected: adding an `EffectDispatched` event kind. It would let a resumed run
distinguish "dispatched" from "not yet dispatched", which is exactly the
distinction the runtime cannot make across a crash.

# ADR-R04 — Reconciliation is not retry, and neither is unbounded

Decision: an effect's *dispatch* budget and its *reconciliation* budget are
tracked separately. A strategy that declares retry safe (`IdempotentRetrySafe`)
re-dispatches up to the bounded ceiling, reusing the stable operation ID.
`Queryable` asks what happened. `RequiresHuman` and `AlwaysCompensate` can
neither be queried nor retried, so the only truthful record is `Unknown`, which
resolves to a permanent disposition — not an invented success and not an
invented failure.

Restoration spends the dispatch budget of any effect the journal leaves
`Reserved`, because the journal proves reservation and cannot prove the effect
never ran. Reconciliation budget is untouched, because asking what happened is
not retrying. Once a budget is spent the effect is left *visibly unresolved*:
no receipt, no status rewriting. The escalation policy above that is a later
work package's, and this runtime deliberately does not paper over its absence.

Rejected: treating an unknown outcome under a non-queryable strategy as a
failure. That is the single most dangerous simplification available here — it
turns "we do not know whether this happened" into a false claim that it did not.
