---
type: ADR
title: "ADR 0124: Unoptimized dev profile for cold builds"
description: Why Larry's dev profile avoids dependency-wide optimization during cold binary builds.
resource: /docs/adr/0124-cold-build-profile.md
tags: [larry, adr, build, performance]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: lgwks-box cargo build --bins --locked --timings on Oracle, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#414, ADR 0049, logicalworks-crates#1138, logicalworks-crates#1139, logicalworks-crates#1145]
---

# ADR 0124: Unoptimized dev profile for cold builds

## Context

Larry's dev profile optimized every dependency. A clean Oracle build paid that
codegen cost even though the binary run path is a debug-profile build.

## Decision

Larry's dev profile keeps `debug = 0` and `incremental = false`, but does not
set `opt-level` and does not add a package-wide dev override. Cargo's default
dev codegen level is the cold-build path. Release settings stay unchanged.

The shipped `larry serve` path still uses the estate-owned
`lgwks_bot::app::App` through the existing `edge` feature. Splitting that
feature would belong to logicalworks-crates#1138, not a private Larry server.

## Evidence

Both runs used `lgwks-box --host oracle`, Rust 1.100.0, Oracle ARM64,
4 vCPU, 251 build units, empty target and build directories, and
`RUSTC_CACHE=off` with both rustc wrappers empty. The Oracle kernel was
Linux 7.0.0-1016-oracle.

- Before, start load 6.37, end load 5.54, `real 264.09` seconds.
- After, start load 4.69, end load 5.41, `real 89.05` seconds.
- Reduction: 66.3 percent. The after timing report is 89.0 seconds.
- `cargo tree --locked --edges normal,build` stayed at 224 unique packages.

The before critical path was `lgwks_bot` 55.9 s, bundled SQLite 55.6 s,
`bevy_ecs` 46.4 s, and the two Syn units at 36.5 s and 34.1 s. The matched
after report's largest units were
`lgwks_bot` 23.3 s, `bevy_ecs` 13.7 s, and bundled SQLite 9.2 s. No after
unit held the path for more than 60 seconds. Remaining estate-owned cost is
tracked by logicalworks-crates#1138, #1139 and #1145.

## Consequences

Cold builds are faster, while release builds and the estate-owned serve path
keep their existing semantics. A cold build still measures above the global
60-second target, so the remaining owner work is recorded rather than hidden.

## Regression

`tests/cold_build_profile.rs` fails while either dev optimization override is
present and passes after this decision. `tests/serve_journey.rs` remains the
run-path proof that the existing estate app still serves the user journey.
