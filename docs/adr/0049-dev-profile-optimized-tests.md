---
type: ADR
title: "ADR 0049: the dev profile runs optimized code"
description: Why Cargo.toml sets opt-level 1 for the workspace and 2 for dependencies in the dev profile.
resource: /docs/adr/0049-dev-profile-optimized-tests.md
tags: [larry, adr, ci, cargo, profile, tests]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: rust job of CI runs 43944474093, 43949143055 and 43950854219, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#246, docs/adr/0048-macos-journey-lane.md]
---

# 0049: the dev profile runs optimized code

Status: accepted, 2026-11-16 (larry#246)

## Context

The workspace suite must finish under 90 s on CI. On `ubuntu-latest` (4 cores) `main` took 107.5 s to
191.9 s across its last four runs. Most of that time is seeded sims that keep a core busy: the TUI paint
sims, the governance checks (regex), and the runtime e35 replays.

- Parallel lanes inside a test do not help a CPU-bound sweep. nextest already runs one test per core.
  On run 43949143055 lanes made the TUI sims 1.4 to 1.7 times slower and the tests beside them slower too.
- Lanes do help a sweep that waits: exec gate 11.6 to 7.4 s, model delay 9.9 to 2.5 s, esc 5.6 to 1.8 s.
- The developer Mac already builds dependencies at opt-level 3 through its home cargo config. CI built
  them at opt-level 0, so the same suite ran different code on CI.

## Decision

- `[profile.dev] opt-level = 1` for the workspace members.
- `[profile.dev.package."*"] opt-level = 2` for every dependency.
- Debug assertions and overflow checks stay on. They follow the dev profile, not opt-level.
- Lanes (`src/sim_lanes.rs`) stay only on sweeps that wait. CPU-bound sweeps run on one lane.

## Consequences

- Every seed and every assertion stays. Each seed costs less CPU.
- A cold build takes longer. The CI rust job prints the build and test step times. The cold build budget
  is 60 s.
- `cargo nextest --release` was set aside: it turns off debug assertions and builds a second tree.
