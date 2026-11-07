---
type: ADR
title: "ADR 0014: the tick runs on lgwks_bot's clock"
description: Why larry-runtime deleted its clock and reads lgwks_bot::clock as time since the Unix epoch, with expiry on skew.
resource: /docs/adr/0014-tick-clock-on-lgwks-bot.md
tags: [larry, adr, runtime, clock, determinism]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: full nextest run before and after on the same 256 replay seeds}
status: accepted
stale_after: 2027-02-07
sources: [larry#74, larry#68, logicalworks-crates#845, lgwks_bot 7.1.1 clock and skew]
---

# 0014: the tick runs on lgwks_bot's clock

Status: accepted, 2026-11-07 (larry#74, larry#68 row 1)

## Context

`larry-runtime` had its own `Clock` trait with `FixedClock` and `SystemClock`. larry#68 moves each
capability onto its one estate owner and deletes the Larry copy in the same change. `lgwks_bot::clock`
owns time: a virtual clock that only moves on `advance`, and a wall clock that never reads earlier.
Its readings are a `Duration` with no recorded epoch yet (logicalworks-crates#845, open).

## Decision

- `Runtime` and `restore` take `lgwks_bot::clock::Clock`; the `C` parameter and the Larry clock are gone.
- `runtime::instant::sample` is the one place a reading becomes an instant: `UNIX_EPOCH` plus the
  reading, or the typed `RuntimeError::ClockOutOfRange` when `SystemTime` cannot hold it.
- `larry-contracts`' `decide` judges expiry with `lgwks_bot::skew::lease_holds` at zero skew, on the
  observer's own timeline: the same strict `observed_at >= valid_until` denial as before.
- `lgwks_bot` is taken with default features off, so only `clock` and `skew` (std only) compile.

## Consequences

- A clock rollback can no longer happen inside one process. The E71 rollback case restores one slept
  history under two clocks a year apart and asserts the same fence answer.
- Replay over the same 256 seeds: p50 58.1 to 62.5 us, p99 154.3 to 108.9 us, RSS 4336 to 4384 KiB.
  larry-runtime normal dependencies 22 to 27, cold build 4.19 s to 6.44 s.
- When #845 lands an epoch-anchored instant, `sample` is the only function that changes.
- `larry_keeps_no_clock_of_its_own` fails if a clock module or trait returns to this crate.
