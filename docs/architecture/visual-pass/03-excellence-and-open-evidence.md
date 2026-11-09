---
type: Design
title: docs/architecture/visual-pass.md, part 3 of 3
description: Part 3 of /docs/architecture/visual-pass.md, in order; the index lists every part.
resource: /docs/architecture/visual-pass/03-excellence-and-open-evidence.md
tags: [larry, demo, appcui, fixtures, evidence]
generated: {by: agent:rust-coder, at: 2026-09-17}
updated: {by: agent:claude-code, at: 2026-09-18, note: Forge terminal language pass}
verified: {by: agent:rust-coder, method: source inspection; execution receipts in log}
status: forge-look-implemented-visual-review-pending
stale_after: 2026-10-18
sources: [Director continuation 2026-09-17, /specification.md, /log.md, https://github.com/gdt050579/AppCUI-rs, https://crates.io/api/v1/crates/lgwks_std/0.6.3, https://crates.io/api/v1/crates/lgwks_deps]
---
## Excellence and open evidence

Predeclared: at most three events, sixteen notes, 4096 bytes per command, one
owned session, no worker threads/queues/network/storage effects. Fixed fixture
size means bounded snapshot work; no measured speed comparison is implied.
The driver uses blocking stdin with no polling or async runtime. Target the
canonical <=512 MiB steady/1 GiB peak memory and <=100 ms input-feedback p95,
but physical Ryzen 3 and constrained profiles remain NOT RUN.

ESTIMATE: 6–10 commands and 30–90 seconds for an informed person to exercise
inspect/feedback/hold/Continue, based on the listed command sequence; no human
participants or observed human outcomes. Baseline native UI does not exist.
No superiority, fleet capacity, portability or production isolation is claimed.
Native clipping is deliberate: long rows/input show the visible tail or width;
this is not a full editor. Mouse selection, clipboard integration and automatic
background fixture timing are absent, not successful no-ops.

Per-file predicted outcomes: Cargo manifest/lock/register give a reproducible
published-estate graph; `.gitignore` retains bounded local build artifacts out
of VCS; Rust library/driver provide effect-free fixture transitions; Rust and
Python tests falsify VF1–VF14; this record and log retain simulations,
admission requirements and actual receipts. All nine architecture axes in
specification Cell 06 remain binding. Idiomatic Rust/generalized/decoupled and
fixture-ephemeral evidence are partial; frontier/hyperscale/portable/production
multi-tenancy/fastest-correct-Rust, physical hardware, native visual interaction,
branch/condition coverage and independent review remain unverified. Measured
Rust line coverage from a fresh isolated native-pass target is 118/118 in
`src/lib.rs`, 320/372 in `src/native.rs`, 0/43 in `src/main.rs` and 0/11 in
`src/bin/larry.rs`, 438/544 overall (80.51%). The separately executed PTY and
JSON CLI suites are not included. Native uncovered paths include runtime entry,
close, conversation eviction, help/scroll/navigation branches and some input
paths. The initial reused-target coverage report double-counted the old library
artifact and was rejected; the isolated target replaces it without deleting
the earlier evidence.
The earlier 14/14 fixture inventory is now expanded to 23 assertions with
native tests; final counts/coverage are retained in log. Independent visual
review and full product acceptance remain pending, not established by finite
tests. Retained failures: initial native clippy rejected a test initializer,
fixed without lint suppression; pyte was unavailable (no dependency installed);
both first PTY tests timed out on quit because the harness stopped draining
terminal output. Draining the owned PTY during exit removed that backpressure;
both real journeys then passed. No terminal failure was silently relabeled.

Forge-look pass, retained failures and limits: the row-width invariant
VF21 was added only after the first 120x32 render showed a clipped `+1 -1`, and
it then failed five more times on copy this record had described as
acceptable — the empty-state hint, two panel footnotes, the welcome sentence,
the help table and the held notice. Each was reflowed rather than the limit
relaxed. The paint-site debug assertion then caught a seventh, a landing row
that only overran below 80 columns; three of the seven would have shipped
unnoticed at the default terminal width. The `Fit` split was introduced because
holding diff source lines to the panel floor would have required either
falsifying the fixture's code or wrapping it. Coverage was **not** re-measured
for this pass: the 80.51% figure below predates `src/theme.rs` and the
rewritten `src/native.rs`, so it no longer describes the current tree and is
retained only as the earlier pass's evidence. `docs/architecture/visual-pass.md`
itself changed label references without re-verifying the simulation table row
by row against the new surface.
