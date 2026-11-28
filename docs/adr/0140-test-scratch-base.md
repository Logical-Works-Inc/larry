---
type: ADR
title: "ADR 0140: one rule for where a test scratch sits"
description: "Both temp guards take one base rule that never makes a scratch inside a git checkout, and each test git call has a ceiling."
resource: /docs/adr/0140-test-scratch-base.md
tags: [larry, adr, tests, hermetic]
generated: {by: claude, at: 2026-11-28}
verified: {by: claude, method: lgwks-box cargo nextest run --locked on the build boxes, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#595, larry#577, INVARIANTS.md, tests/support/scratch_base.rs, git(1)]
---

## Context

With `TMPDIR` a plain directory inside a checkout, 9 of 17 island and snapshot sims
failed. Two causes. The crate's guard made scratch directories where the Linux
confinement lists them, and a sibling gone before `setpriv` opened it refused the
command. And git climbed from a scratch to the checkout above it.

## Decision

`tests/support/scratch_base.rs` is the one rule. `temp_dir.rs` includes it as source,
because the runtime's tests load that guard twice. `executor::sim_tree` loads it by
`#[path]`. A configured temp dir is left for `/var/tmp` when it lies inside a git
checkout, on every host, or when on Linux it is not sticky and world-writable.
Each test git call also sets `GIT_CEILING_DIRECTORIES` to the parent of its directory.

## Rejected

- The ceiling alone: the snapshot sims call the product's git, which runs under a
  cleared environment, and the product also refuses a folder with a `.git` above it.
- A second copy of the rule in `sim_tree`: two owners drift.

## Consequences

A scratch does not follow `TMPDIR` into a checkout. The product's walk of the paths
above a state directory is unchanged and stays open as larry#557. Reverse this when
the product and every test git call take an explicit repository boundary: the
checkout half of the rule is then not needed.

## Proof

`a_scratch_inside_a_checkout_is_left_for_the_shared_one`,
`a_bounded_git_never_sees_the_checkout_above_its_scratch` and
`sim_tree_git_never_sees_the_checkout_above_its_directory`.

## Traceability

Issue #595, part of #423.
