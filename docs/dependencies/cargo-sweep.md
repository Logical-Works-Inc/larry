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

Read 2026-11-03. Rule (Director 2026-10-24, 2026-11-03): estate first (`std`, `lgwks_std`, `lgwks_bot`,
`lgwks_ast`, `lgwks_deps`, all at their newest release: 4.1.1, 8.2.0, 6.0.2, 6.3.0); Loco.rs where the estate
owns nothing; any other crate only with a `gap` issue on its manifest line. Labels: MEASURED, READ, DOC.

## Measured with cargo (MEASURED, stable toolchain, load average 4 to 6)

| Scratch set | Unique crates | Cold build | Peak RSS | Result |
|---|---|---|---|---|
| `loco-rs =1.2.0` (`cli`, `with-db`, `db-sqlite`, `auth`) + `lgwks_bot =6.1.0` (`rt`, `net`), deps at `opt-level 0` | 544 | 87.5 s | 393 MB | resolves; over the 60 s budget under this load |
| `loco-rs` (`db-sqlite`) + `lgwks_deps =5.1.2` (`sqlite`) | none | none | none | refused: `libsqlite3-sys` 0.37 (sqlx) and 0.39 (rusqlite) both link `sqlite3` |
| `rmcp =3.8.0` (`server`, `transport-io`), deps at `opt-level 3` | 62 | 69.5 s | 545 MB | resolves |
| Larry's estate set + `lgwks_bot =7.1.1` (`edge`, `script`), 2026-11-08; Larry ships on 8.2.0 | 220 (111 without) | 35.8 s (22.7 s) | 1.08 GB | resolves; `edge` alone compiles from 8.0.1 |
| Larry's branch for larry#88 on `lgwks_bot =8.0.1` (`edge`, `signal`), `cargo build --bins`, load 10.4, 2026-11-09 | 225 | 45.97 s | 998 MB | resolves |

Method: `cargo generate-lockfile`, `cargo tree -e normal --prefix none | sort -u | wc -l`,
`/usr/bin/time -l cargo build --locked` into a fresh target directory after one warm-up fetch, sccache on
(40 % Rust hit rate). logical-forms measured 42.2 s for the same Loco stack with the override on a quiet machine.

## The boundary, one row per capability

| Larry needs | Owner | Rejected alternative, and why |
|---|---|---|
| Async runtime, net, supervision | `lgwks_bot::rt` | `tokio::main`: one process driver, `lgwks_bot::block_on` |
| Tick clock, authority expiry (larry#74) | `lgwks_bot::clock`, `skew`, defaults off: 22 to 27 crates, cold 4.2 s to 6.4 s | the deleted `larry-runtime` clock |
| Journal, crash recovery, replay (larry#75) | `lgwks_bot::journal::RecordJournal`, group-committed (ADR 0023) | Loco jobs or SeaORM rows: a second journal |
| Dispatch fence, offline exposure (larry#76, #99) | `lgwks_bot::broker` (ADR 0024, 0026) | the `larry-runtime` revocation fence |
| Tick, projection | `lgwks_bot` ECS world | Loco scheduler: not deterministic, not replayable |
| Authority, grants (larry#77) | `lgwks_bot::gate` `GrantSet`, `cap::Auth`; policy from secure-authority | the `BTreeSet` coverage loops in `larry-contracts`; Loco `auth`: it authenticates, it never authorizes |
| Effect keys, retry, dispatch certainty (larry#78) | `lgwks_bot::effect`, `retry`, defaults off: no new crate | the inline attempt counter in `larry-runtime` (larry#68) |
| Receipts signed (larry#86, ADR 0006) | `lgwks_std::sign` (ed25519), seed in `lgwks_deps` `sqlite` | forge's SipHash audit chain: a MAC, no offline check |
| Sealed records at rest | `lgwks_bot::sealed` | none needed |
| Model client to 9router | `lgwks_std::http` + `json` + `retry`, ported from forge-providers | `async-openai`: a second client, more crates |
| Code reading, diffs | `lgwks_ast` | `syn`: the estate has one reader |
| Prompt-injection screen (larry#37) | `lgwks_deps` `llm-guard` | a classifier of our own first |
| Rerank port (larry#53) | `lgwks_deps` `ml-candle`, `ml-tokenizers` | Python reranker: banned |
| Terminal UI | `lgwks_deps` `appcui` (already) | none |
| HTTP routes, body and tenant bounds (larry#88) | `lgwks_bot::app` (axum through `lgwks_deps`), ADR 0021 | Loco: 544 crates, a second edge |
| Human sign-in, password hashing, magic links | **Loco** `auth` | estate owns none |
| Approval mail, out-of-band checks (larry#41) | **Loco** mailers (lettre) | estate owns none |
| Side jobs (mail fan-out, digests) | **Loco** workers | effects: never, they stay on the journal |
| Operational tables, migrations | **Loco** SeaORM, rebuilt from the journal | authority in SQL: refused |
| Config, env, CLI tasks | **Loco** | estate owns none |
| Row-level tenancy | **Loco** `multi-tenancy`, Postgres RLS backstop | developer discipline alone |
| MCP stdio (Face 3) | forge-mcp server ported onto `lgwks_std::json` | `rmcp`: 62 crates for JSON-RPC we own |
| MCP remote, Streamable HTTP and OAuth (Face 2) | `rmcp` | stdio port: carries no transport or OAuth |
| Sandboxed execution | Nova (crates#846) | forge stations: interim, Apple `container` only |
| Executor process groups, orphan reaping (larry#85) | `lgwks_std::process` | processkit: kill-on-drop dies with its owner under kill -9 |
| Executor ledger of live groups (larry#85) | `lgwks_deps` `sqlite` | a pid file: no start instant, so a reused pid reads as live |
| Typed errors | `thiserror` | estate has no derive |
| Memory, durable product state | Logical-DB | any substitute store: refused |

Gap issues (Logical-Works-Inc/logicalworks-crates): #848 web layer (`loco-rs`, `axum`, `sea-orm`,
`async-trait`); #849 the SQLite links clash; #850 remote MCP (`rmcp`); #842 `thiserror`; #846 Nova's
edges. Each new manifest line cites its issue.

## Where Loco is used, and where it is refused

- `lgwks_bot::app` serves (ADR 0021). Loco, where it lands, mails and keeps operational tables; it never
  serves routes, dispatches an effect, authorizes or holds authority.
- Every Loco table is a projection the journal rebuilds; dropping the database loses nothing but caches.
- One SQLite engine per binary: the executor ledger (larry#85, ADR 0004) takes `lgwks_deps/sqlite`, so Loco
  takes Postgres through sqlx, or `db-sqlite` once #849 closes.
- `larry serve` is in the default build: the edge takes Larry's tree from 118 to 225 crates and the cold build
  is 45.97 s, under 60 s (ADR 0021). Loco, if #91 adds it, sits behind a feature with the `opt-level 0` override.
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
