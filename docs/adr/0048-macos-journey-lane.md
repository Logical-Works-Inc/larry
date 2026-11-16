---
type: ADR
title: "ADR 0048: CI runs the PTY journeys on hosted macOS"
description: Why ci.yml adds an e2e/journey leg on macos-latest beside sim.yml's Linux leg.
resource: /docs/adr/0048-macos-journey-lane.md
tags: [larry, adr, ci, macos, journey, seatbelt]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: e2e/journey (macos-latest) on the PR, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#245, larry#241, larry#116, docs/adr/0046-tui-isolated-commands.md]
---

# 0048: the macOS journey lane

Status: accepted, 2026-11-16 (larry#245, a part of larry#241)

## Context

`e2e/journey` ran on `ubuntu-latest` only. PR #240 said CI ran a macOS lane. No job did. Since ADR 0046,
a TUI command runs under `sandbox-exec` on macOS and under Landlock on Linux. Linux CI proved only the
Landlock path. The Seatbelt path a Mac user lives had no CI proof.

## Decision

- `ci.yml` job `e2e-journey-macos`, named `e2e/journey (macos-latest)`, runs `tests/e2e_journey.rs` on
  hosted `macos-latest`.
- It runs only while the repository is public. A public repository never uses the self-hosted Mac
  (larry#116). While private, `sim.yml`'s self-hosted leg is already a Mac.
- A last step runs the probe tests again with `--nocapture`, so the log prints p50/p95/p99 and RSS.
- Actions are pinned by commit SHA, as in `sim.yml`.

## Consequences

- A PR shows the Seatbelt path green or red by name.
- Hosted macOS minutes are spent on each PR. The leg has a 15 minute cap.
- The job lives in `ci.yml`, not `sim.yml`, by the Director's order naming `ci.yml` (2026-11-16).
