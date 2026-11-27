---
type: ADR
title: "ADR 0124: Candidate b kept out of the dev profile"
description: Why Larry keeps main's suite-safe profile after the Oracle comparison.
resource: /docs/adr/0124-cold-build-profile.md
tags: [larry, adr, build, performance]
generated: {by: opencode, at: 2026-11-26}
verified: {by: opencode, method: lgwks-box Oracle cold builds and nextest, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#414, ADR 0049, logicalworks-crates#1138, logicalworks-crates#1139, logicalworks-crates#1145]
---

# ADR 0124: Candidate b kept out of the dev profile

## Context

Cold `cargo build --bins --locked` on Oracle (ARM64, 4 vCPU,
rustc 1.100.0, empty target, `RUSTC_WRAPPER` and sccache off) took
about 264 s on main. The budget is 60 s. Suite time is paid on every
pull request, so a colder build must not regress the hosted suite.

## Decision

Main's profile stays: workspace opt-level 1, dependencies at
opt-level 2, Cargo defaults for build scripts. Candidate b
(workspace 1, every dependency 1, build scripts 0) is kept out:
it failed the judge.

## Evidence

Six Oracle rows, alternating main and b, each a clean-target cold build
followed at once by the full workspace suite (1,360 tests each row).

| Run | Profile | Cold build s | Suite Summary s | Result |
| --- | --- | ---: | ---: | --- |
| 1 | Main: workspace 1, deps 2 | 261.172 | 96.317 | 1,360 passed |
| 2 | Candidate b | 202.177 | 102.551 | 1,360 passed |
| 3 | Main: workspace 1, deps 2 | 267.596 | 98.738 | 1,360 passed |
| 4 | Candidate b | 206.356 | 100.044 | 1,360 passed |
| 5 | Main: workspace 1, deps 2 | 260.542 | 95.986 | 1,360 passed |
| 6 | Candidate b | 213.706 | 101.080 | 1,360 passed |

Medians: main cold 261.172 s, candidate b cold 206.356 s, lower
by 54.816 s (-21.0%). Oracle suite medians: main 96.317 s,
candidate b 101.080 s (+4.9%, every b row slower than every main row).
The cold win is real but the suite pays it on every run.

The hosted pair judges the suite. Latest main run of the `test` job
(run 45905701155, `5accb02`): wall 3m08s, Summary 64.160 s for
1,363 passed. Candidate b head (PR #467, `fb61509`): wall 6m26s,
Summary 89.458 s for 1,364 passed. That is 39.4% over main, past
the 2% judge. Main's own recent runs read 64.160 s, 74.803 s and
75.812 s, so the b head sits outside main's range too.

Rejected with their numbers: candidate b (above); candidate c
(hot estate deps at opt-level 2, 264.82 s cold with the governance
percentile test timed out at 300.023 s); `lgwks_bot` and `bevy_ecs`
at 2 (334.33 s cold, 314.175 s suite); the prior Larry-only profile
(170.91 s cold, 296.343 s suite).

Where the time goes (lead, `cargo build --bins --locked --timings`, Oracle, main `5accb02`, empty
target and build dir, cache off, load 2.9 to 5.1): 260.26 s wall, 251 units, 925 s of unit time,
214 normal dependencies (`cargo tree -e normal`). Slowest units: `lgwks_bot` 57.0 s, the
`libsqlite3-sys` build script 51.6 s, `bevy_ecs` 43.8 s, `syn` 2 36.7 s, `larry-demo` 36.3 s,
`syn` 3 34.2 s, `lgwks_deps` 32.2 s, `derive_more-impl` 30.7 s, `rustls` 29.7 s, `lgwks_std`
27.7 s, `lgwks_ast` 12.8 s. The critical path: `bevy_ecs_macro_logic` (starts 97.9 s), then
`bevy_ecs_macros` (137.3 s), then `bevy_ecs` (155.5 s to 199.3 s), then `lgwks_bot` (196.4 s to
253.4 s), then the Larry binaries (254.5 s to 260.2 s). About 160 s of the 260 s is that chain.

The remaining critical path is estate-owned: `lgwks_bot` and its
`edge` path through `bevy_ecs` (logicalworks-crates#1138),
`lgwks_ast` and its Rust grammar (logicalworks-crates#1139), and
bundled `libsqlite3-sys` (logicalworks-crates#1145). Larry does not
fork those owners.

## Consequences

The shipped dev profile is unchanged. Cold build stays above 60 s
until the named estate gaps land. Reversal trigger: a hosted suite
more than 2% over main on three consecutive main runs reopens this
ADR and restores main's profile.

## Regression

`tests/cold_build_profile.rs`
(`dev_profile_optimizes_workspace_hot_path_only`) fails on any
other profile and passes on main's. The hosted `test` job remains
the final suite judge.
