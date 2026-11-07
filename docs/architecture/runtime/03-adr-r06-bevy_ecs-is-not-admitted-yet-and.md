---
type: ADR
title: docs/architecture/runtime.md, part 3 of 3
description: Part 3 of /docs/architecture/runtime.md, in order; the index lists every part.
resource: /docs/architecture/runtime/03-adr-r06-bevy_ecs-is-not-admitted-yet-and.md
tags: [rocco, runtime, determinism, replay, ecs, bevy]
generated: {by: claude-code, at: 2026-09-20}
verified: {by: claude-code, method: cargo build/test/clippy/fmt, lgwks-deps check, check_governance.py, unittest, all local; no hosted CI}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-20
sources: [/specification.md, /AGENTS.md, /contract/APPROVED.toml, /log.md, Director decisions 2026-09-20]
---
# ADR-R06 — `bevy_ecs` is not admitted yet, and this change adds no dependency edge

Status: **pending Director sign-off.** The runtime compiles against Rust `std`,
`lgwks_std` and `rocco-contracts` only. `lgwks-deps check .` reports one
semantic approval and every authored external edge owned, with no new row.

The dependency law in `/AGENTS.md` admits a new third-party crate only with an
owner, a reason and a human sign-off, and the default answer is no. `bevy_ecs`
has no row and was not given one by this change: an unsigned row carrying a
Director's name would be a fabricated approval, which is the exact failure this
register exists to prevent. The row text, ready to sign or to refuse, is in
`/contract/APPROVED.toml` as a comment.

`thiserror` is likewise not used here. Its register row names `rocco-contracts`
as the single `allowed_consumers` entry, so this crate hand-writes `Display` and
`std::error::Error` rather than widening a signed row as a local convenience.

# Evidence

Local gates, run from the repository root on 2026-09-20:

- `cargo build --bins --locked` — passed.
- `cargo test --workspace --all-targets --locked` — 131 passed, 0 failed
  (57 `rocco-contracts` unit, 12 `rocco-contracts` workspace, 30 `rocco-demo`
  unit, 6 `rocco-demo` integration, 2 `rocco-runtime` unit, 8 `rocco-runtime`
  determinism, 16 `rocco-runtime` lifecycle).
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `lgwks-deps check .` — `OK . — 1 semantic approvals, every authored external
  edge is owned`.
- `python3 tools/check_governance.py` — PASS, inventory 96/53/80/13/9.
- `python3 -m unittest discover -s tests` — 47 passed.
- `git diff --check` — clean.
- Reranker receipts: `tools/receipts/20260920-runtime-core-pre-exec.json` and
  `tools/receipts/20260920-runtime-core-post-exec.json`.

Hosted CI has not run for this change.

# What this does not claim

- **No canonical owner is integrated.** Effects pass through
  `owner::EffectOwner`; the only implementations in the crate are the
  deterministic `owner::fixture` doubles. Nothing contacts Moo, Sentinel, Nova,
  Logical-DB, Secure-Authority or a network.
- **No durability across process death.** The journal is bounded and in memory.
  Persistence is Logical-DB's and the memory hold is intact; this crate claims
  recovery *from a journal in hand*, not survival of a power loss.
- **No proof.** P1–P8 remain proof obligations. In particular P2 (atomic
  event+reservation and fenced effect ownership) and P5 (deterministic replay
  equivalence) are falsified by tests here, not discharged by proof; the
  contract module's own record of P5 awaiting property tests and model checking
  still stands.
- **No compensation path.** `TerminalDisposition::Compensated` is handled but
  this slice never produces it; compensation is a new authorized effect and
  belongs with the C04 work.
- **No policy activation.** The policy, capability and validity snapshots are
  pinned for the runtime's lifetime. D26's explainable, atomically activated
  effective policy is unimplemented.
- **No frontend.** The existing AppCUI surface is a mockup and is expected to
  change; this work package is headless and touches no renderer.

# Unresolved

- ADR-R05's authority question, for the Director.
- ADR-R06's dependency admission, for the Director's signature.
- Retry and reconciliation exhaustion currently leave an effect visibly
  unresolved with no escalation. That is fail-closed and honest, but it is not
  a policy, and it needs one.
- The `Cancelling` run state completes as soon as its owned effects stop; there
  is no compensation-before-cancel path, so `RolledBack` remains unreachable in
  this slice.
- A denial records the run's `Fail` transition but not the denial *reason*: the
  closed `EventKind` vocabulary has no place for it, and inventing one is a
  specification change. The reason is returned in the tick report instead.
