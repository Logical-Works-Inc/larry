---
type: Design
title: Cargo sweep, what Larry builds on and where Loco fills the estate's gaps
description: Every Larry capability mapped to its estate owner, a forge port or Loco.rs, with cargo measurements, the order to production and the benchmark against other harnesses.
resource: /docs/dependencies/cargo-sweep.md
tags: [larry, dependencies, loco, cargo, benchmarks, nine-axes]
generated: {by: claude-code, at: 2026-11-03}
verified: {by: claude-code, method: cargo generate-lockfile, cargo tree and timed cold builds on scratch manifests; crates.io API; design-gate research}
status: active
stale_after: 2026-12-03
sources:
  - https://loco.rs/docs/how-to/multi-tenancy
  - https://github.com/modelcontextprotocol/rust-sdk
  - https://arxiv.org/abs/2605.27922
  - https://arxiv.org/html/2601.11868v1
  - https://www.tbench.ai/leaderboard/terminal-bench/2.0
---

# Cargo sweep

Read 2026-11-26. Rule (Director 2026-10-24, 2026-11-03): estate first (`std`, `lgwks_std`, `lgwks_bot`,
`lgwks_ast`, `lgwks_deps`, all at their newest release: 5.0.0, 10.0.3, 6.1.1, 8.0.0 on 2026-11-24); Loco.rs where the estate
owns nothing; any other crate only with a `gap` issue on its manifest line. Labels: MEASURED, READ, DOC.

## Measured with cargo (MEASURED, stable toolchain, load average 4 to 6)

| Scratch set | Unique crates | Cold build | Peak RSS | Result |
|---|---|---|---|---|
| Larry's estate set + `loco-rs =1.3.1`, defaults off, 2026-11-24: none, `auth`, `mailer`, `worker`+`db-postgres`, `with-db`+`db-sqlite`, `with-db`+`db-postgres` | 209 to 344, 360, 359, 388, 416, 429 | not built | not built | resolves; `db-sqlite` and `lgwks_deps` `sqlite` share `libsqlite3-sys` 0.40, so crates#849's clash is gone (ADR 0104) |
| Larry's estate set less one edge, 2026-11-24: lgwks_bot `edge`, lgwks_ast `lang-rust`, `crossterm` | 209 to 141, 188, 197 | not built | not built | `edge` pulls the bevy ECS world (crates#1138); `lang-rust` pulls ast-grep's matcher and walker (crates#1139) |
| `rmcp =3.8.0` (`server`, `transport-io`), deps at `opt-level 3` | 62 | 69.5 s | 545 MB | resolves |
| Larry's estate set + `lgwks_bot =7.1.1` (`edge`, `script`), 2026-11-08; Larry ships on 8.2.0 | 220 (111 without) | 35.8 s (22.7 s) | 1.08 GB | resolves; `edge` alone compiles from 8.0.1 |
| Historical larry#88 branch on `lgwks_bot =8.0.1` (`edge`, `signal`), `cargo build --bins`, load 10.4, 2026-11-09 | 225 | 45.97 s | 998 MB | historical, not the shipped pin set |
| Larry's branch for larry#293 on `lgwks_ast =6.0.3` (`lang-rust`), `env -u CARGO_BUILD_BUILD_DIR cargo build --bins --locked --target-dir <fresh>`, 2026-11-17; the tree without the edge measured 230.8 s on 214 crates the same day | 228 (214 without) | 406.7 s, load 5.75 to 10.55 on 4 cores (73% CPU) | 598 MB | resolves; `larry governance`'s temp-path rule p50 279.5 ms, p95 282.8 ms, p99 356.1 ms per check over the root's `src` and `tests` (ADR 0061) |
| Larry shipped pins 5.0.0 / 10.0.3 / 6.1.1 / 8.0.0, fresh target on Toronto, 2026-11-26 | 217 | 124.94 s | 666.4 MiB (682,440 KiB) | resolves; `lgwks-box` cache enabled |

Method: `lgwks-box cargo tree --workspace --locked -e normal --prefix none | sort -u | wc -l` printed 217.
`lgwks-box sh -lc 'fresh=$(mktemp -d /tmp/larry-217-after.XXXXXX); /usr/bin/time -f "elapsed_seconds=%e peak_rss_kib=%M" cargo build --bins --locked --target-dir "$fresh"'`
printed `elapsed_seconds=124.94 peak_rss_kib=682440` on Toronto with the box cache enabled. Older rows are historical.

## The boundary, one row per capability

| Larry needs | Owner | Rejected alternative, and why |
|---|---|---|
| Async runtime, net, supervision | `lgwks_bot::rt` | `tokio::main`: one process driver, `lgwks_bot::block_on` |
| Tick clock, authority expiry (larry#74) | `lgwks_bot::clock`, `skew`, defaults off: 22 to 27 crates, cold 4.2 s to 6.4 s | the deleted `larry-runtime` clock |
| Journal, crash recovery, replay (larry#75) | `lgwks_bot::kernel::journal::RecordJournal`, group-committed (ADR 0023) | Loco jobs or SeaORM rows: a second journal |
| Dispatch fence, offline exposure (larry#76, #99) | `lgwks_bot::broker` (ADR 0024, 0026) | the `larry-runtime` revocation fence |
| Tick, projection | `lgwks_bot` ECS world | Loco scheduler: not deterministic, not replayable |
| Authority, grants (larry#77) | `lgwks_bot::gate` `GrantSet`, `cap::Auth`; policy from secure-authority | the `BTreeSet` coverage loops in `larry-contracts`; Loco `auth`: it authenticates, it never authorizes |
| Effect keys, retry, dispatch certainty (larry#78) | `lgwks_bot::effect`, `retry`, defaults off: no new crate | the inline attempt counter in `larry-runtime` (larry#68) |
| Receipts signed (larry#86, ADR 0006) | `lgwks_std::sign` (ed25519), seed in `lgwks_deps` `sqlite` | forge's SipHash audit chain: a MAC, no offline check |
| Sealed records at rest | `lgwks_bot::sealed` | none needed |
| Model client on the tenant route table (ADR 0039) | `lgwks_std::http` + `json` + `retry`, ported from forge-providers | `async-openai`: a second client, more crates |
| Code reading, diffs | `lgwks_ast`, whose checked parse now judges the temp-path rule (ADR 0058) | `syn`: the estate has one reader |
| Prompt-injection screen (larry#37) | `lgwks_deps` `llm-guard` | a classifier of our own first |
| Rerank port (larry#53) | `lgwks_deps` `ml-candle`, `ml-tokenizers` | Python reranker: banned |
| Terminal UI (larry#239, ADR 0045) | Larry's own cell frame on `crossterm` =0.29.0 and `unicode-width` =0.2.3, gap crates#1067 | AppCUI: bold lost its colour, 82 crates; cursive, iocraft, ratatui: see ADR 0045 |
| HTTP routes, body and tenant bounds (larry#88) | `lgwks_bot::app` (axum through `lgwks_deps`), ADR 0021 | Loco: +135 crates with every feature off (1.3.1), a second edge |
| Human sign-in, password hashing, magic links | **planned (#89)**: Loco `auth` | estate owns none yet |
| Approval mail, out-of-band checks (larry#41) | **planned (#91)**: Loco mailers (lettre) | estate owns none yet |
| Side jobs (mail fan-out, digests) | **planned (#91)**: Loco workers | effects: never, they stay on the journal |
| Operational tables, migrations | **planned (#90)**: Loco SeaORM, rebuilt from the journal | authority in SQL: refused |
| Config, env, CLI tasks | **planned (#70)**: Loco | estate owns none yet |
| Row-level tenancy | **planned (#89)**: Loco `multi-tenancy`, Postgres RLS backstop | developer discipline alone |
| MCP stdio (Face 3) | forge-mcp server ported onto `lgwks_std::json` | `rmcp`: 62 crates for JSON-RPC we own |
| MCP remote, Streamable HTTP and OAuth (Face 2) | `rmcp` | stdio port: carries no transport or OAuth |
| Sandboxed execution | Nova (crates#846) | forge stations: interim, Apple `container` only |
| Terminal commands, interim kernel confinement (larry#243, ADR 0046) | host `sandbox-exec` (macOS), `setpriv --landlock` (Linux), gap crates#1075 | host-wide `model.command`: refused |
| Executor process groups, orphan reaping (larry#85) | `lgwks_std::process` | processkit: kill-on-drop dies with its owner under kill -9 |
| Workspace directory mode reset before removal (larry#334, #366) | `lgwks_std::fs::capability::Dir` holds the walk; `rustix` =1.1.6 `openat`/`chmodat` until the estate adds a mode-reset call, gap crates#1113 | `std::fs::set_permissions` by path: follows a link swapped in after the check (CWE-367, CWE-59) |
| Executor ledger of live groups (larry#85) | `lgwks_deps` `sqlite` | a pid file: no start instant, so a reused pid reads as live |
| Typed errors | the estate's `lgwks_macros::Error` derive (`Display`, `Error`, `From`; a `#[from]` or `#[source]` field is the source, lgwks_ast 6.1.1 `error_derive/parse.rs`), on every error type since larry#435; `RouteError`, `RerankError` (an `Option` source) and `mutants::Fault` (a source inside a non-`Error` field) keep hand-written impls | `thiserror` (removed larry#339: its gap #842 closed) |
| Memory, durable product state | Logical-DB | any substitute store: refused |

Gap issues (logicalworks-crates): #848 web layer; #849 SQLite lane; #850 remote MCP; #846 Nova; #1113 mode
reset from a held `Dir`; #1141 durable file replace; #1143 process-group exit wait. Each manifest line cites one.

## Where Loco is used, and where it is refused

- `lgwks_bot::app` serves (ADR 0021). Loco, where it lands, mails and keeps operational tables; it never
  serves routes, dispatches an effect, authorizes or holds authority.
- Every Loco table is a projection the journal rebuilds; dropping the database loses nothing but caches.
- One SQLite engine per binary: the executor ledger (larry#85, ADR 0004) takes `lgwks_deps/sqlite`, and Loco's
  `db-sqlite` links the same `libsqlite3-sys` 0.40 (measured 2026-11-24), so both share one engine (ADR 0104).
- Historical ADR 0021 measured `larry serve` at 45.97 s on the old 8.0.1 tree. The current shipped-pin measurement
  is recorded in the table above. Loco, when #91 adds it, sits behind a feature with the `opt-level 0` override:
  `mailer` alone is +150 crates on the default build (ADR 0104).
- The production config boots in the live test (empty templated YAML becomes null and 500s a route).

## Forge: code once

`srinji-kaggss/forge-harness` already has a provider client, an MCP JSON-RPC server, worktree containment,
approvals and an audit chain. They move onto the estate (larry#71) rather than being rewritten. Parts that
`forge-mcp`'s NOTICE marks as adapted from OpenAI Codex are not ported; they are rewritten from the spec.

## Speed to production

1. larry#68 rows 1 to 5 (shipped): clock, the journal on `RecordJournal` (kill -9 loses nothing), fence, gate, retry.
2. Track 3 executor (larry#71): Director's own code daily, no customer data, no web layer, so no Loco cost.
3. Track 2 gateway (larry#70): `larry serve` on the app edge over the same runtime, two tenants, receipts.
4. Benchmarks (larry#72, larry#65) run on what shipped.

## Benchmarks against other harnesses (DOC until run)

Harness-Bench (106 tasks, harness crossed with model; compares Claude Code, Codex, OpenCode, OpenClaw,
Hermes, Goose, Nanobot) and Terminal-Bench 2.0 through Harbor (runs Claude Code, Codex CLI, OpenHands).
Model held constant, harness the only variable. Reported: pass rate, cost per accepted task,
p50/p95/p99, peak RSS, approvals seen, and receipt completeness, which only Larry can score. Built against a
seeded mock model endpoint first; scored runs need model spend (larry#72).

## Nine axes

Frontier: each row names the alternative it beat. Hyperscale: Loco payload and timeout limits over the
runtime's bounds; 10,000 req/s target with p50/p95/p99. Idiomatic: typed errors, no lint lowered.
Generalized: one engine per binary, Postgres through the same sqlx path. Decoupled: one owner per row.
Ephemeral: Loco state is rebuildable; the journal is the record. Portable: pure-Rust estate edges; bundled
SQLite. Multi-tenant: tenant from authenticated data, run-table tenant check, RLS backstop. Fastest:
measured above; every new edge re-measured in its own PR.
