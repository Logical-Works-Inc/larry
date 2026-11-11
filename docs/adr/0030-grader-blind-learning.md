---
type: ADR
title: "ADR 0030: grader-blind learning"
description: Why evidence provenance is a type, so a grader verdict, rubric or score cannot reach a proposer, composer, selector or reflector, and what stays open.
resource: /docs/adr/0030-grader-blind-learning.md
tags: [larry, adr, contracts, evidence, epic-60]
generated: {by: claude-code, at: 2026-11-10}
verified: {by: claude-code, method: nextest run of crates/larry-contracts/tests/sim_evidence.rs, cargo test --doc, and the workspace}
status: accepted
stale_after: 2027-02-10
sources: [larry#63, larry#60, crates/larry-contracts/src/evidence/mod.rs, logicalworks-crates#846]
---

# 0030: grader-blind learning

Status: accepted, 2026-11-10 (larry#63 part 1, epic larry#60)

## Context

Epic #60 has Larry learn its own harness. A model that proposes, composes, selects or reflects must
never see what a grader, verifier, rubric or score produced, or it learns the grader and not the
work. A convention or a warning does not hold that line. The requirement is a refusal that does not
depend on anyone remembering to call a check.

## Decision

- Provenance is a type. `Evidence<P>` is generic over the markers `Execution`, `Deliverable`,
  `Source` and `Grader`. The trait `ImproverSafe` is sealed and implemented for the first three only.
- `ImproverItem::new` takes an `Evidence<P: ImproverSafe>`, and `ImproverContext` holds only
  `ImproverItem`s. `Evidence<Grader>` therefore cannot enter a context: the compiler refuses it. Two
  `compile_fail` doctests pin this, and the first has a passing twin for `Execution`. Removing the
  guard (making `Grader` safe) fails that doctest, which is the negative control.
- Bytes whose provenance is known only at run time come as `AnyEvidence`. It converts through
  `into_improver`, which refuses grader provenance with `LeakKind::GraderProvenance`.
- `ImproverContext::assemble` scans every item before the context exists and fails closed with a
  typed `LeakError { path, kind }`: `GraderVerdict`, `RubricLine` or `Score`. The scan is plain text
  matching, no regex: JSON keys such as `"verdict"`, `"score"` and `"rubric"`, lines starting with
  those words, and `score` followed by a digit. It can over-refuse and cannot prove a text clean, so
  it is defence in depth. Items and contexts are bounded (see the module doc).
- No improver call site exists yet. #62 builds the proposer, composer and selector, and their input
  parameter type must be `ImproverContext`, so this module is the only door.

Rejected:
- A runtime string check alone: it is a convention, and a missed call leaks.
- Regex-only detection standing in for the type: it is a guess about text, not a boundary.
- A provenance field the improver could carry: then a `Grader` value could be built into its input.

## Consequences

- Open, not done: executor network egress during learning, deny-by-default with a per-benchmark
  blocklist of rubric hosts, enforced at the effect boundary. Larry has no network sandbox. Isolation
  is Nova's (logicalworks-crates#846, warden needs wasmtime and seccompiler). Nothing here fakes it.
- Rubric text pasted into a deliverable by a person is caught only by the scan, and only if it uses a
  marker the scan knows.
- Covered by `tests/sim_evidence.rs` (`sim_evidence_falsifier_each_plant_is_refused_before_any_proposer_call`,
  `sim_evidence_negative_control_without_the_guard_the_plants_reach_the_proposer`,
  `sim_evidence_clean_assembles_and_one_planted_leak_is_always_refused`, 320 seeds) and the doctests.
  Nextest does not run doctests; `cargo test --doc -p larry-contracts` does (MEASURED).
