---
type: ADR
title: "ADR 0124: Candidate b dev profile"
description: Why Larry ships workspace and dependency opt-level 1 with build scripts at 0.
resource: /docs/adr/0124-cold-build-profile.md
tags: [larry, adr, build, performance]
generated: {by: opencode, at: 2026-11-26}
verified: {by: opencode, method: lgwks-box Oracle cold builds and nextest, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#414, ADR 0049, logicalworks-crates#1138, logicalworks-crates#1139, logicalworks-crates#1145]
---

# ADR 0124: Candidate b dev profile

## Context

Cold `cargo build --bins --locked` on Oracle (ARM64, 4 vCPU,
rustc 1.100.0, empty target, `RUSTC_WRAPPER` and sccache off) took
about 264 s on main. The budget is 60 s. Suite time is paid on every
pull request, so a colder build must not regress the hosted suite.

## Decision

Candidate b ships: workspace opt-level 1, every dependency opt-level 1,
build scripts at opt-level 0. Release settings do not move.

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
candidate b 101.080 s. The judge for the suite is the hosted `test`
job, not Oracle: it must stay at most 2% over main's latest run.

The hosted pair (same workflow, `test` job wall plus Summary line)
is recorded beside this ADR once the head runs.

Rejected with their numbers:

- Candidate c (hot estate deps at opt-level 2): 264.82 s cold and
  the governance percentile test timed out at 300.023 s.
- `lgwks_bot` and `bevy_ecs` at 2: 334.33 s cold, 314.175 s suite.
- The prior Larry-only profile: 170.91 s cold, 296.343 s suite.
- Restoring main's profile: closes nothing; the cold cost stays.

The remaining critical path is estate-owned: `lgwks_bot` and its
`edge` path through `bevy_ecs` (logicalworks-crates#1138),
`lgwks_ast` and its Rust grammar (logicalworks-crates#1139), and
bundled `libsqlite3-sys` (logicalworks-crates#1145). Larry does not
fork those owners.

## Consequences

The shipped dev profile is candidate b. Cold build stays above 60 s
until the named estate gaps land. Reversal trigger: a hosted suite
more than 2% over main on three consecutive main runs reopens this
ADR and restores main's profile.

## Regression

`tests/cold_build_profile.rs` (`dev_profile_uses_candidate_b_profile`)
fails on main's profile and passes on candidate b. The hosted `test`
job remains the final suite judge.
