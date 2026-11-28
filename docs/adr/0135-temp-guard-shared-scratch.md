---
type: ADR
title: "ADR 0135: the temp guard sits in a shared scratch directory"
description: "On Linux a plain configured temp dir is left for /var/tmp, so the Landlock walk never lists sibling guards."
resource: /docs/adr/0135-temp-guard-shared-scratch.md
tags: [larry, adr, tests, confinement]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: lgwks-box toronto nextest and llvm-cov nextest, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#577, larry#557, INVARIANTS.md, tests/support/temp_dir.rs]
---

## Status

Accepted.

## Context

Route journeys failed under coverage. The coverage run puts `TMPDIR` on disk, in a plain directory.
The Landlock walk lists every level above the state directory that is not sticky and world-writable
(ADR 0081). It listed the other tests' temp dirs. One was gone before `setpriv` opened it, and exec
was refused: `could not open file for landlock`. The victim changed from run to run.

## Decision

The one temp guard of the integration tests uses `/var/tmp` on Linux when the configured temp dir is
not sticky and world-writable. `/var/tmp` stays on disk. The walk never lists it.

Rejected: retries in each journey, and more coverage environment forwarding. The profile path was not
the cause, and a retry hides the refusal.

Reverse this when larry#557 lets a confinement start beside entries that vanish.

## Consequences

An explicit plain `TMPDIR` no longer holds the journeys' temp dirs on Linux. The product still refuses
a command when a listed entry vanishes. That stays open as larry#557.

## Proof

- `a_guard_never_sits_in_a_directory_the_confinement_lists` failed before the change.
- `only_a_plain_directory_sends_a_guard_to_the_shared_one` covers the modes and the missing dir.
- The two route journey binaries and `coverage_forward`: 40 of 40, plain and under coverage.
