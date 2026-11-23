---
type: ADR
title: "ADR 0095: the mutation lane judges every mutant on x86_64 and ARM64"
description: Why each mutants shard runs on hosted x86_64 and hosted ARM64, and why the verdict fails on a mutant the two judge apart.
resource: /docs/adr/0095-the-mutation-lane-judges-two-architectures.md
tags: [larry, adr, ci, mutants, portability]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: the_mutants_lane_judges_every_mutant_on_both_linux_architectures red on Oracle then green; the verdict script run on four synthetic outcome sets, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#404, larry#310, larry#339, larry#116, /docs/adr/0064-ci-mutants-lane.md, /docs/adr/0066-mutants-lane-renames-and-capacity.md]
---

# 0095: the mutation lane judges two architectures

Status: accepted, 2026-11-23 (larry#404). Amends ADR 0064's runner and its "never on main".
Director: "add x86 as well in the testing portfolio".

## Context

The mutants lane ran on `ubuntu-latest`. That image is x86_64, but the lane named no
architecture and checked none. The Mac and Oracle are ARM64, and no lane judged ARM64 Linux.

## Decision

- Each shard runs twice: x86_64 on `ubuntu-24.04` and aarch64 on `ubuntu-24.04-arm`. The
  shard checks its rustc host triple and fails closed on the wrong one.
- The verdict judges each architecture on its own, with the same plan and the base-ref floor.
- A mutant that one architecture catches and the other misses or cannot build, or that only
  one judged, fails the verdict by name. A timeout on one side is load, so it is only printed.
- cargo-mutants 27.1.0 ships no aarch64 Linux binary. A push to main builds it from source,
  compiles the workspace tests on ARM64, and saves both under `mutants-aarch64`
  (`cache-bin` keeps `~/.cargo/bin` and `.crates2.json`). The ARM64 shards restore that key.

Rejected: the self-hosted x64 org runners. A public repository never runs fork code on a
self-hosted machine (larry#116). Rejected: x86_64 alone. A test that passes only on one
architecture would go unseen.

## Consequences

- A matrix holds at most 2 x 96 = 192 jobs, inside GitHub's 256. A PR near the cap queues twice
  the jobs ADR 0066 sized. ESTIMATE: about 50 minutes at 20 concurrent jobs.
- Reverse the warm job if a warm ARM64 shard is cut at its 15-minute timeout. Reopen the
  doubling if the queue alone pushes a median PR's lane past 30 minutes.
- Falsifier: `tests/ci_fallback_job.rs`
  (`the_mutants_lane_judges_every_mutant_on_both_linux_architectures`).
