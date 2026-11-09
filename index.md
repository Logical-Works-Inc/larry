---
type: Index
title: Larry specification index
description: Final proposed specification v1, harness research, repository governance and falsifiable completion plan.
resource: /index.md
tags: [larry, strategy, governance]
generated: {by: opencode, at: 2026-09-15}
updated: {by: claude-code, at: 2026-11-09, note: repository, inventory counts and decision range re-read from log.md and the specification}
verified: {by: opencode, method: source inspection; document validation recorded separately}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-12-09
sources: [Director conversation 2026-09-15, /log.md, /docs/adr/0005-rename-rocco-to-larry.md]
---

# Larry: a persistent assistant with an accountable runtime

**Final proposed specification v1, not an approved implementation or runtime readiness claim.**

The strategic bet: spend developer effort on durable intent, effect safety,
context construction, learning and verification so that a weaker model and its
human can accomplish more with less supervision and total computation.

## Read in this order

1. [Specification and completion notebook](specification.md): PRD, all source
   requirements, candidate decisions, graph, state/failure semantics, evaluation
   design, nine-axis targets and implementation cells.
2. [Research and reuse ledger](research-ledger.md): inspected code, primary
    sources, external reviews, rejected claims and unresolved dependencies.
3. Finalized structure: [architecture](/docs/architecture/README.md),
   [dependency root](/docs/dependencies/README.md) and
   [evaluation structure](/docs/evaluation/README.md).
4. [Repository governance](GOVERNANCE.md), [agent instructions](AGENTS.md), and
   [decision/change record](log.md): strict maintenance rules and actual checks.

## Authority and scope

- The Director selected the working name Rocco and authorized a private
  `srinji-kaggss/Rocco` documentation repository; these documents were moved
  there from the Forge scratchpad, and no product code was imported. It is now
  `Logical-Works-Inc/larry`, a public alpha since 2026-11-06, renamed Larry
  (ADR 0005), and it carries the implementation the specification governs.
- Preserve `forge-harness` and `forge-sdk` intact as reference implementations.
  Do not silently repair, import, restructure or publish their engine code.
- The Director has authorized specification finalization and repo governance. Notable
  decisions require human approval before implementation. `approval: pending`
  is not a veto on research or on completing this package.
- Sentinel, Secure-Authority, Logical-DB, Nova, Moo and Logicalworks-Crates retain
  their named responsibilities. Discover and reuse contracts; do not create
  parallel authorities when an integration is missing.
- The notebook consolidates the active strategy. It does not delete historical
  requirements or rewrite old repositories as if their contents were approved.
- Markdown cells and embedded Mermaid are the canonical notebook source. No
  Jupyter server, Python runtime, generated website or duplicate `.ipynb` is
  required to read it. Executable evaluation cells come after approval and are
  explicitly NOT RUN today.

## How to approve

Approve or amend decisions by `D01`–`D53`, or approve the specification as a package
with explicit exceptions. Record the exact document revision and approver;
never infer approval from silence or a model review. Approval adopts decisions
and targets, not claims that tests, human studies or million-user trials passed.

Already settled by the Director: core + TUI first; bounded unattended work;
adapters rather than compatibility clones; memory implementation waits for
Logical-DB; adjustable automatic decay and user-facing tuning; one Larry
repository with strict self-maintaining governance, public alpha under HL3
(2026-11-06). Other architecture decisions remain proposed until the Director
ratifies each one.

The 96-requirement inventory preserves R01–R43, adds R44–R70 from the
Codex/OpenCode/OpenClaw/Letta review, R71–R78 from the 2026-09-16 SOTA gap
research and R79–R96 from the 2026-09-17 hostile-model security review. There
are 80 planned falsifiers (E01–E80), not 80 passes; the ones that run are named
with their tests in /INVARIANTS.md.

## File-specific excellence record

Requirement: one navigable source of current strategy without another docs maze.
Axes: all nine are indexed by the notebook; decoupling and epistemic honesty are
directly affected here. Predicted outcome: one entry point with explicit approval
and governance boundaries; no competing specification in the old scratchpad.
Verification: local link and requirement-inventory validation. Unresolved:
human approval, implementation and all product acceptance evidence.
