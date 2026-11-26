---
type: ADR
title: "ADR 0124: Per-package dev profile for cold builds"
description: Why Larry optimizes only its measured hot-path packages in dev builds.
resource: /docs/adr/0124-cold-build-profile.md
tags: [larry, adr, build, performance]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: lgwks-box cargo build and cargo nextest on Toronto, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#414, ADR 0049, logicalworks-crates#1138, logicalworks-crates#1139, logicalworks-crates#1145]
---

# ADR 0124: Per-package dev profile for cold builds

## Context

The old profile optimized Larry at opt-level 1 and every dependency at
opt-level 2. The suite then measured 258.857 s on CI. Removing all overrides
cut the cold build, but the decision had no per-package hot-path rule.

## Decision

Keep `debug = 0` and `incremental = false`. Set opt-level 1 only for
`larry-demo`, `larry-runtime`, and `larry-contracts`. Keep every dependency at
Cargo's dev default 0. Set `[profile.dev.build-override] opt-level = 0` so
build scripts and proc macros stay unoptimized. Release settings do not move.

The measured suite hot path is Larry's executor, workspace snapshot and
isolation, eval, receipt, TUI, route, and serve code. The estate still owns
`lgwks_bot::app` and its `edge` feature. Larry does not fork that owner.

## Evidence

All candidate runs used the same 1,353-test workspace workload on Toronto.
Cold builds used an empty target, `RUSTC_CACHE=off`, and an empty
`RUSTC_WRAPPER`. The final runs sampled load with `uptime`; the initial opt-2
candidate used the same machine but did not emit a load sample.

| Profile | Cold build s | Suite s | Load | Machine |
| --- | ---: | ---: | --- | --- |
| All packages 0 | 82.15 | 53.128 | 7.31/11.52/10.82 to 19.02/13.87/11.74 | Toronto, 4 vCPU |
| Larry packages 1 | 107.02 | 50.957 | 12.96/13.61/11.94 to 20.27/15.59/12.79 | Toronto, 4 vCPU |
| Larry packages 2 | 97.65 | 50.823 | not emitted | Toronto, 4 vCPU |
| Old package-wide profile | 264.09 | 258.857 | CI review run | ubuntu-latest |

The final selected profile is 2.171 s faster than opt-0 on the full suite and
does not approach the 90 s suite budget. No candidate reached the 60 s cold
build target. The timing report and the issue's clean run identify the
remaining estate-owned holders: `lgwks_bot` and its `edge` path through
`bevy_ecs` (logicalworks-crates#1138), `lgwks_ast` and its Rust grammar
(`logicalworks-crates#1139`), and bundled `libsqlite3-sys` (`#1145`).

The box's nextest is 0.9.149 and rejects `--junit-path`. The supported
`libtest-json-plus` report counted 1,353 tests and exposed the slow seeded
executor, snapshot, eval, receipt, TUI, route, and serve cases above.

## Consequences

Larry's dev builds compile only its three measured workspace packages with
opt-level 1. Dependencies, build scripts and proc macros keep opt-level 0.
The suite stays below its 90 s budget, but the cold build remains above 60 s
until the named estate owners reduce their compile paths.

## Regression

`tests/cold_build_profile.rs` fails when the three package overrides are absent,
rejects a package-wide wildcard, and requires the build override at opt-level
0. `tests/serve_journey.rs` remains the run-path proof for the estate app.
