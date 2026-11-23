---
type: ADR
title: "ADR 0098: Larry's mutation verdict runs from main on workflow_run"
description: Why the mutation lane's verdict moved out of the pull request's workflow into a workflow_run job that builds main's larry mutants verdict.
resource: /docs/adr/0098-the-mutation-verdict-runs-from-main.md
tags: [larry, adr, ci, mutation, security]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: sim_every_generated_run_gets_the_reference_verdict red then green on Oracle; larry mutants verdict over the real artifacts of run 45272759157, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#387, larry#339, GitHub Security Lab "Preventing pwn requests" (2021), CWE-829]
---

# 0098: The mutation verdict runs from main

Status: accepted, 2026-11-23 (larry#387). Amends ADR 0064 and ADR 0095.

## Context

On a `pull_request` event GitHub runs the head's workflow file. #385 read the floor and the
caps from the base ref, but the verdict job itself was the pull request's: a PR could edit its
script to `exit 0`, or delete the step that read the base ref, and pass itself.

## Decision

- `mutants.yml` plans and runs the shards and judges nothing. Its plan also uploads
  `mutants-plan` (cargo-mutants' `list.json`).
- `mutants-verdict.yml` runs on `workflow_run` of `mutants`, from the default branch's file. It
  checks out main (no `ref:`, no token kept), builds main's `larry`, downloads the judged run's
  artifacts, lists its latest jobs, runs `larry mutants verdict` and posts the commit status
  `mutants / verdict` on the run's head commit: pending on `requested`, then success or failure.
- `larry mutants verdict` reads main's `mutants.yml` for the floor and caps, and every artifact
  through the governance bounded reader (no link, no FIFO, at most 4 MiB). It refuses an
  unknown artifact, an extra file, a malformed or oversized file, a failed or missing plan or
  shard job, and outcomes that are not exactly one per planned mutant. Each refusal exits 1;
  a run under the floor or judged apart exits 2.
- The verdict is Rust because the repository holds no shell source (Director 2026-10-13).

Rejected: `pull_request_target` with a checkout of the head. It runs pull request code with the
base repository's token and cache scope.

## Consequences

- The artifacts are still made by pull request code. A PR that rewrites its own shard jobs can
  upload forged outcomes. The verdict holds them to the plan, but a forged plan and forged
  outcomes that agree still pass. Only shards on trusted infrastructure close that: logical_ci
  owns verdicts.
- A PR that renames or removes the `mutants` workflow gets no status. A missing
  `mutants / verdict` is not a pass.
- The pull request that adds this file gets no verdict: `workflow_run` reads the default
  branch's copy.
- Falsifier: `tests/mutants_verdict_journey.rs`.
