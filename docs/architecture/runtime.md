---
type: ADR
title: Larry runtime — the lowest invariant, made executable
description: The journal-authoritative deterministic runtime, its ECS seam, the deferred authority question and the pending Bevy dependency admission.
resource: /docs/architecture/runtime.md
tags: [larry, runtime, determinism, replay, ecs, bevy]
generated: {by: claude-code, at: 2026-09-20}
verified: {by: claude-code, method: cargo build/test/clippy/fmt, lgwks-deps check, check_governance.py, unittest, all local; no hosted CI}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-20
sources: [/specification.md, /AGENTS.md, /contract/APPROVED.toml, /log.md, Director decisions 2026-09-20]
---

# Outcome

`crates/larry-runtime` is the first code in this repository that *drives* the
C02 contract algebra rather than describing it. Before this change the finite
run-state model, the pure authorization reducer, the effect intents and the
terminal receipts all existed in `larry-contracts` as pure functions and values
with nothing to sequence them; the specification's runtime control flow was a
diagram, not a program.

The runtime is the lowest invariant rather than a feature: every higher claim in
the specification — that a commitment survives a crash, that an effect is never
dispatched without authority, that an outcome is never invented — is only as
true as this layer's ability to refuse the alternative.

## Parts

1. [ADR-R01 — The journal is authoritative; everything else is a fold](/docs/architecture/runtime/01-adr-r01-the-journal-is-authoritative.md)
2. [ADR-R05 — The ECS seam exists; the authority question is deliberately open](/docs/architecture/runtime/02-adr-r05-the-ecs-seam-exists-the.md)
3. [ADR-R06 — `bevy_ecs` is not admitted yet, and this change adds no dependency edge](/docs/architecture/runtime/03-adr-r06-bevy_ecs-is-not-admitted-yet-and.md)
