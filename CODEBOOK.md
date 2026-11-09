---
type: Codebook
title: Larry codebook
description: Module map, owners and invariants for the Larry workspace.
resource: /CODEBOOK.md
tags: [larry, codebook, modules]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: read of each crate root and module declaration, 2026-11-07}
status: public-alpha
stale_after: 2026-12-06
sources: [crates/larry-contracts/src/lib.rs, crates/larry-runtime/src/lib.rs, src/lib.rs, INVARIANTS.md]
---

# Codebook

Rules live in `INVARIANTS.md` with their tests; decisions in `docs/adr/`. Each module has one owner.

## crates/larry-contracts (pure algebra; no state, clock or I/O)

| Module | Owns |
|---|---|
| `ids` | Typed identifiers and BLAKE3 content digests |
| `state` | The run-state algebra and its one transition function |
| `event` | Recorded events and per-run sequencing |
| `command` | Command envelopes and duplicate suppression |
| `effect` | Intents, reconciliation strategies, terminal receipts |
| `authority` | Grants, attenuation, boundary re-validation |
| `binding` | Resolution of an intent to its external target |
| `replay` | The pure authorization reducer and pinned replay; expiry on `lgwks_bot::skew::lease_holds`, coverage on `lgwks_bot::gate::GrantSet` (larry#77) |
| `error` | Every typed contract failure |
| `workspace` | Bounded workspace projections and the Continue boundary |
| `fixture` (test only) | Pinned instants shared by the unit tests |

## crates/larry-runtime (the deterministic tick)

| Module | Owns |
|---|---|
| `journal` | The bounded append-only authoritative record |
| `payload` | Typed bodies of the closed event vocabulary |
| `projection` | The fold, effect phases and the replay report |
| `protocol` | E47 version and capability negotiation: the offer, its typed refusals |
| `gate` | Admission and the `cap::Auth` proof every mutating reach presents (larry#77, ADR 0018) |
| `fence` (private) | Dispatch-time freshness: epoch, revocation, offline bound |
| `owner` | The effect-owner trait; test doubles under `fixture` only |
| `consumer` | Bounded, tenant-scoped journal catch-up with explicit resync (E35) |
| `error` | Every refused runtime operation |
| `runtime` | Settings, `TickReport`, stage 2 (admit, reserve, dispatch), `restore` |
| `runtime::instant` | The tick's one pinned instant: `lgwks_bot::clock` read as time since the Unix epoch (larry#74) |
| `runtime::display` | E65 display and admission on the real command path |
| `runtime::ingress` | `submit_protocol`: negotiate, tenant check and bounded backoff before the queue (E47) |
| `runtime::live` | Run and effect witnesses the journal does not carry |
| `runtime::reconcile` | Stage 3 reconciliation and stage 4 cancellation |
| `runtime::reach` | The one owner reach: the fence, then the `cap::Auth` proof, then the call (larry#77) |
| `runtime::retry` | The effect key an intent derives and the retry decision on `lgwks_bot::retry` (larry#78, ADR 0017) |
| `runtime::writers` | Every journal writer and the run-state `advance` |

## Root crate (the `larry` binary)

| Module | Owns |
|---|---|
| `lib` | The effect-free DEMO session state, `replace_file` (the whole-or-nothing write) and `diagnose` (one stderr line) |
| `executor` | `larry exec`: settings, the run of one task, its JSON report (ADR 0004) |
| `executor::gate` | Every task as a displayed proposal on `larry_runtime`; `LARRY_POLICY`; the sealed refusal (ADR 0019) |
| `executor::task` | The bounded task document and its typed refusals |
| `executor::author` | The brief (goal, files) and the model step that writes each file before acceptance (ADR 0008) |
| `executor::run` | One command in its own process group: capture, timeout, group kill |
| `executor::ledger` | The SQLite record of live groups, the reap after a kill -9, the receipt key |
| `executor::receipt` | The ed25519 seal over a report, its verification and derived verdict (ADR 0006) |
| `executor::workspace` | The throwaway `git clone --local` workspace, its diff and its removal |
| `eval` | `larry eval`: one arm over a matched set, origins, outcomes, the scorecard (ADR 0009) |
| `eval::set` | The bounded task set and its typed refusals, checked before any task runs |
| `eval::score` | Whole-number prices and costs, nearest-rank percentiles, the card |
| `model` | The blocking chat-completions client to 9router (ADR 0003) |
| `model::client` | One exchange per attempt, retried within the `RetryPolicy` |
| `model::config` | Gateway, key (never shown), pinned model, temperature, timeout, retry |
| `model::wire` | The request body and the reply parser |
| `native` | The native AppCUI view: session state, commands and saturating geometry |
| `native::render` | Painting the forge chrome: transcript, panels, dashboard, input, status hub |
| `native::input` | Key routing and the paint entry point of the session view |
| `native::workspace` | The board-first workspace: cards, the `ink!` and `shell_card!` painters |
| `native::workspace::views` | The board, truth and work scenes |
| `native::workspace::input` | Workspace keys, mouse and paint |
| `theme` | View colours |
| `mcp` | The `larry mcp` server; `larry_submit_task` runs the executor (ADR 0008) |
| `serve` | `larry serve`: the address, tenant keys, the routes on `lgwks_bot::app`, the stop signals (ADR 0021, 0022) |
| `governance` | `larry governance`: the structural checker over the governed documents and its report (ADR 0010) |
| `governance::documents` | The bounded load of the top-level documents and the `docs/` tree; split documents read whole (ADR 0015) |
| `governance::markdown` | Frontmatter, fences, local links and GitHub heading ids |
| `governance::inventory` | The R, D, E, C and axis inventories, traceability, cells, log drift, crash cases |
| `governance::error` | Every refusal that stops a check, printed in the Python checker's wording |
| `governance::receipts` | The reranker receipts' shape, SHA-256 (retained) or BLAKE3 (ADR 0013) |
| `mascot` | `larry mascot`: the one pixel grid, its SVG, preview and pet package (ADR 0014) |
| `mascot::webp` | The lossless VP8L encoder of the pet spritesheet (RFC 9649) |
| `mascot::sim` | Seeded encoder round trips through an RFC reader, the SVG check, the command line |
| `rerank` | `larry-rerank`: the request, its bounds, the ranking and the create-exclusive receipt (ADR 0013) |
| `rerank::qwen` | The pinned Qwen3 reranker on Candle and its tokenizer, feature `rerank` only |
| `rerank::sim` | Seeded ports of the Python reranker controls, bounds, ranking and receipts |
| `governance::tracker` | `larry tracker`: the tracker page rendered from `tools/tracker/tracker.json` |
| `governance::pyjson` | JSON printed byte for byte as CPython's `json.dumps`, for the tracker page |
| `governance::patterns` | Every checker regex, compiled once, searched as Python's `\w`, `\s` and `\b` read |
| `governance::pystr` | CPython `str` semantics the messages need: `splitlines`, `split`, `repr`, decode errors |
| `governance::sim_*` | Seeded ports of the Python controls, parser references, bounds and two-root checks |
| `src/main.rs` | The demo binary entry |
| `src/bin/larry.rs` | The `larry` launcher: native view, `mcp`, `model`, `exec`, `verify`, `key`, `eval`, `governance`, `tracker`, `mascot`, `serve` |
| `src/bin/larry-local-ci.rs` | `cargo local-ci`: every gate in AGENTS.md order, then the local CI receipt |
| `src/bin/larry-rerank.rs` | `larry-rerank` (feature `rerank`): one ranked request and its printed receipt |
| `eval/matched-v1.json` | The shipped matched task set, 12 POSIX sh tasks |

## Invariants the module map carries

- Only `runtime::reconcile` completes a cancellation, and only with no owned effect open (ADR 0001).
- Only `runtime::writers` appends to the journal; every append folds the projection after it.
- `runtime::live` witnesses are private to `runtime`; `restore` rebuilds them from the journal.
- `model` retries only 429 and 5xx and never shortens a `Retry-After` (ADR 0003).
- `executor::ledger` records a group's leader before the wait and removes it only once the group is gone, and opens and migrates `executor.db` under the `executor.db.lock` file lock, one executor at a time.
- Only `executor::receipt` derives `accepted`; sealing and verifying both call it (ADR 0006).
- Shared test fixtures live in `tests/support`, `tests/sim_kit` and `tests/fence_kit`, each with a test
  that exercises every helper: `support/origin.rs` the origin repository the executor and MCP journeys
  share, `support/mcp_task.rs` the operator session and receipt check behind `larry_submit_task`,
  `support/larry_cli.rs` the `larry` child and its report readers, `support/serve_cli.rs` `larry serve`.
