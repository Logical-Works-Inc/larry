---
type: Codebook
title: Larry codebook
description: Module map, owners and invariants for the Larry workspace.
resource: /CODEBOOK.md
tags: [larry, codebook, modules]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: read of each crate root and module declaration, 2026-11-06}
status: public-alpha
stale_after: 2026-12-06
sources: [crates/rocco-contracts/src/lib.rs, crates/rocco-runtime/src/lib.rs, src/lib.rs, INVARIANTS.md]
---

# Codebook

Rules live in `INVARIANTS.md` with their tests; decisions in `docs/adr/`. Each module has one owner.

## crates/rocco-contracts (pure algebra; no state, clock or I/O)

| Module | Owns |
|---|---|
| `ids` | Typed identifiers and BLAKE3 content digests |
| `state` | The run-state algebra and its one transition function |
| `event` | Recorded events and per-run sequencing |
| `command` | Command envelopes and duplicate suppression |
| `effect` | Intents, reconciliation strategies, terminal receipts |
| `authority` | Grants, attenuation, boundary re-validation |
| `binding` | Resolution of an intent to its external target |
| `replay` | The pure authorization reducer and pinned replay |
| `error` | Every typed contract failure |
| `workspace` | Bounded workspace projections and the Continue boundary |

## crates/rocco-runtime (the deterministic tick)

| Module | Owns |
|---|---|
| `clock` | The pinned time source, one instant per tick |
| `journal` | The bounded append-only authoritative record |
| `payload` | Typed bodies of the closed event vocabulary |
| `projection` | The fold, effect phases and the replay report |
| `gate` | The pure authorization reducer at admission |
| `fence` (private) | Dispatch-time freshness: epoch, revocation, offline bound |
| `owner` | The effect-owner trait; test doubles under `fixture` only |
| `consumer` | Bounded, tenant-scoped journal catch-up with explicit resync (E35) |
| `error` | Every refused runtime operation |
| `runtime` | Settings, `TickReport`, stage 2 (admit, reserve, dispatch), `restore` |
| `runtime::display` | E65 display and admission on the real command path |
| `runtime::live` | Run and effect witnesses the journal does not carry |
| `runtime::reconcile` | Stage 3 reconciliation and stage 4 cancellation |
| `runtime::writers` | Every journal writer and the run-state `advance` |

## Root crate (the `rocco` binary)

| Module | Owns |
|---|---|
| `lib` | The effect-free DEMO session state |
| `model` | The blocking chat-completions client to 9router (ADR 0003) |
| `model::client` | One exchange per attempt, retried within the `RetryPolicy` |
| `model::config` | Gateway, key (never shown), pinned model, temperature, timeout, retry |
| `model::wire` | The request body and the reply parser |
| `native` | The native AppCUI view |
| `theme` | View colours |
| `src/main.rs`, `src/mcp/` | The binary entry and the `rocco mcp` server |
| `src/bin/rocco.rs` | The `rocco` launcher: native view, `rocco mcp`, `rocco model` |

## Invariants the module map carries

- Only `runtime::reconcile` completes a cancellation, and only with no owned effect open (ADR 0001).
- Only `runtime::writers` appends to the journal; every append folds the projection after it.
- `runtime::live` witnesses are private to `runtime`; `restore` rebuilds them from the journal.
- `model` retries only 429 and 5xx and never shortens a `Retry-After` (ADR 0003).
