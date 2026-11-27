---
type: ADR
title: One environment probe at task start, sealed as receipt 0
description: Fixed read-only task-folder probe, injected, sealed and prompt-riding.
resource: /docs/adr/0131-env-probe.md
tags: [larry, executor, probe, receipt, model]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: design-gate and seeded executor tests}
status: accepted
stale_after: 2026-12-26
sources: [larry#274, larry#255, /INVARIANTS.md, /docs/product/SHIP.md]
---

# ADR 0131

## Status

Accepted. Shipped in the larry#274 PR on this branch.

## Context

Meta-Harness probes the working folder once and puts the facts in the first
prompt; its README says this saves 2 to 5 turns. Its probe hardcodes `/app`
and swallows every failure. Larry tasks started blind: the model discovered
the folder with extra commands and the person never saw what it saw. A probe
run live inside every task would also break two shipped invariants: SD-4
(the same seed replays the same gateway trace) and ADR 0043 (every completion
of a brief begins with the same bytes, so a provider's prefix cache hits).

## Decision

After the clone and before the model step, `run_task` takes the task's probe
from an injected input (`Settings::probe`): `LARRY_PROBE` unset runs the live
fixed read-only probe of the task folder (`executor::probe`) through the same
seams its simulations script; a set value is exactly that probe, refused typed
when it is not JSON, not an object, or over 64 KiB, never a silent fallback to
the live probe. The probe reads the folder's path, a bounded tree listing
(depth 2, 200 entries), the versions of eight fixed tools found on PATH, the
sizes of ten fixed manifests at the root, free memory and disk, and the
`git status` head. Every tool runs its own fixed argv from a table (`go
version`, `java -version` read from stderr, the rest `--version`); a tool line
is `ok` only on exit 0, else the typed failure with the exit status or the
bound kill, never stderr as a version. Every section records its own typed `ok`
outcome; a failure is a field, never a dropped fact and never a refused task.
The probe runs in the tree but homes in its own directory beside it
(`Place::Probe`, removed with the workspace): the child keeps `RUSTUP_HOME`
and `CARGO_HOME` resolved from the host's real home (read only) and sets
`RUSTUP_AUTO_INSTALL=0`, so a missing toolchain is a fast typed failure, never
a download. A live probe once homed in the tree (`Place::Open`), so a
toolchain wrote its home there: 48 s and 780 MB beside one README, and the
kept diff head no longer named the task's change; then rustup synced a whole
toolchain on every task (wall 16 s on macOS) until the homes moved out.

The value seals as the report's `probe` member. Its rendering rides in the
model's user turn after the byte-stable prefix and before the file line, in
both prompt modes, counted by the fits check. Nothing secret is recorded:
fixed commands with fixed arguments, names, versions and sizes only. No
environment value and no file content ever reaches the record.

## Why

The model and the person must see the same facts before the first edit. The
probe reuses `run_command` for every spawned call, so timeouts, group kills
and bounded capture are the tested ones, not a second spawn path. Injection
keeps the replay invariants whole: the same seed with the same probe value
drives byte-identical prompts, so the same gateway trace and card replay
(SD-4), and the probe never moves a brief's cached prefix bytes (ADR 0043).
The probe rides the admitted task body, so a denied task seals no probe for
work that never ran.

## Consequences

Replay journeys pin `LARRY_PROBE` to one fixed value, so their trace and card
replay assertions hold exactly as before; a live probe would make each run's
prompt unique. A malformed value refuses the task before any model call. The
sealed probe grows each report by at most 64 KiB and each briefed prompt by
at most 2048 characters. A live probe can never be distinguished from a fixed
one by the receipt alone: the caller owns its environment, as with every
`LARRY_*` setting.

## Proof

`sim_probe_each_fixture_shape_reports_its_expected_probe` drives five
fixture folders; `sim_probe_every_section_failure_is_typed_never_dropped`,
`sim_probe_hostile_bytes_never_panic_and_stay_bounded`,
`sim_probe_render_is_bounded_and_names_the_facts` and
`sim_probe_reports_no_secret_values` hold the failure, bounds and secrecy
rules; `sim_probe_a_failed_tool_is_never_ok` (exit 2, empty output, bound
kill) and `sim_probe_every_tool_runs_its_own_argv` (go, java on stderr, rest)
hold the sixth-pass argv and status rules. `sim_a_probe_rides_both_prompt_
modes_after_the_prefix_and_the_fits_check_counts_it` holds the prompt wiring.
`a_probe_place_runs_in_the_tree_but_homes_beside_it` holds the homes and the
no-install switch. The journeys
`an_env_probe_is_sealed_as_receipt_zero_and_reaches_the_model_first`,
`the_same_seed_with_the_same_probe_replays_the_same_trace`,
`a_malformed_probe_value_refuses_the_task_typed_and_seals_nothing` and
`a_live_probe_never_asks_a_toolchain_to_install` drive the shipped binary
through the mock model and the fake rustup proxy. Every other journey pins
`LARRY_PROBE` through the one fixed-probe helper (`tests/support/exec_cli.rs`
for the executor, receipt and idempotency binaries), so no verdict pays a
live probe. The matched-set turn-count comparison rides the
#94 runner, which is not built: NOT RUN.
