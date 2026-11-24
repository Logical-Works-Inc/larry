---
type: ADR
title: "ADR 0106: The executor's timeout, retry budget and admission lease read the injected clock"
description: Why the command timeout, the model retry deadline and the admission lease each read the one lgwks_bot Clock the caller injects, built once per run, instead of Instant::now or SystemTime::now.
resource: /docs/adr/0106-executor-decisions-on-the-injected-clock.md
tags: [larry, adr, executor, model, clock, determinism]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, 1320 of 1320 passed, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#436, larry#74, larry#182, larry#268]
---

# 0106: The executor's timeout, retry budget and admission lease read the injected clock

Status: accepted, 2026-11-24 (larry#436). Extends ADR 0105 and the larry#74 one-timeline rule.

## Context

- Three decisions read the wall directly, so no seeded sim could drive them (SD-4):
  `wait_for_status` (`Instant::now`, the `timed_out` flag in the sealed report), the model
  client's retry deadline (`Instant::elapsed`), and the admission lease (`SystemTime::now`,
  then a second `live_clock` for the tick).
- INVARIANTS rule 25: the tick instant and every authority expiry share one timeline.

## Decision

- `run_command` takes its deadline as a reading of the clock it already builds, and
  `wait_for_status` compares against that clock.
- `ModelClient` holds a `Clock`; `ModelClient::new(config, clock, pause)`. Live callers pass
  `larry_runtime::live_clock()`, sims pass `Clock::virtual_at`.
- `admit_and_run` and `recover_one` build one live clock, start the lease at its reading
  (`gate::lease_start`, a typed `ClockOutOfRange` past the platform's range) and hand the
  same clock to `restore`.

Rejected: a fake-time crate over `Instant` and `SystemTime`. It is a second clock beside
lgwks_bot's and breaks rule 25.

## Consequences

- Five new sims drive each decision on a virtual clock, one seed one trace hash.
- `gate::lease_start` repeats the epoch placement of `larry_runtime::runtime::instant::sample`,
  which is private; reverse this when the runtime exports it.
