---
type: ADR
title: "ADR 0124: Suite-safe Cargo dev profile"
description: Why Larry keeps the hosted suite profile while measuring cold-build cuts.
resource: /docs/adr/0124-cold-build-profile.md
tags: [larry, adr, build, performance]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: lgwks-box Oracle cold builds and nextest, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#414, ADR 0049, logicalworks-crates#1138, logicalworks-crates#1139, logicalworks-crates#1145]
---

# ADR 0124: Suite-safe Cargo dev profile

## Context

The old PR profile used workspace opt-level 1 and every dependency at 2.
Hosted CI measured 74.755 s on main and 195.348 s on the prior PR head.
The cold-build target is 60 s, but suite time is paid on every pull request.

## Decision

Keep main's profile. Larry workspace packages use opt-level 1. Dependencies
use the package wildcard at opt-level 2. Build scripts and proc macros keep
Cargo's default. Release settings do not move.

The selective profiles were rejected because they did not preserve the suite:
all dependencies at opt-level 1 measured 215.00 s cold and 98.140 s for 1,349
tests on Oracle. The narrower estate opt-2 profile timed out the governance
percentile test at 300.023 s. Optimizing only `lgwks_bot` and `bevy_ecs` at 2
measured 334.33 s cold and 314.175 s for 1,349 tests.

## Evidence

Every row used `lgwks-box --host oracle`, an empty target, cache-off build
variables, and a clean target before the build. The suite ran immediately
after the build with the same 1,349-test workspace workload.

| Profile | Cold build s | Suite s | Result |
| --- | ---: | ---: | --- |
| Main: workspace 1, deps 2 | 264.00 | 97.266 | 1,349 passed |
| Workspace 1, all deps 1 | 215.00 | 98.140 | rejected, slower suite |
| Hot estate deps 2 | 264.82 | timeout | rejected, governance timeout |
| `lgwks_bot` and `bevy_ecs` 2 | 334.33 | 314.175 | rejected, slower suite |
| Prior Larry-only profile | 170.91 | 296.343 | rejected, slower suite |
| Hosted CI on head `2083e45` | n/a | 63.630 | 1,360 passed |

The hosted main reference was 74.755 s for 1,353 tests. The new head's hosted
`test` Summary is 63.630 s for 1,360 tests. Oracle is a measurement
environment, not a replacement for that hosted judge.

No candidate reduced cold build time while preserving suite speed. The
remaining critical path is estate-owned: `lgwks_bot` and its `edge` path
through `bevy_ecs` (logicalworks-crates#1138), `lgwks_ast` and its Rust
grammar (logicalworks-crates#1139), and bundled `libsqlite3-sys`
(logicalworks-crates#1145). Larry does not fork those owners.

## Consequences

The shipped dev profile preserves main's suite behaviour. Cold build remains
above 60 s until the named estate gaps land. The regression test rejects a
missing workspace opt-1 setting, a missing dependency opt-2 setting, any
selective override, and a build-override section.

## Regression

`tests/cold_build_profile.rs` fails before this profile is restored and passes
after it is restored. The hosted `test` job remains the final suite judge.
