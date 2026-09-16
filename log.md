---
type: Log
title: Rocco decisions and change evidence
description: Accepted directions, scoped corrections and current specification inventory.
resource: /log.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversation 2026-09-15, /specification.md, /research-ledger.md]
---

# Current specification inventory

requirements: 70
decisions: 35
falsifiers: 54
execution_cells: 13
axes: 9
planned_crash_cases: E02 E13 E19 E28 E31 E39 E40 E42 E50

Counts are document structure, not implemented features or passing tests.

## 2026-09-15: settled Director directions

| Direction | Disposition |
|---|---|
| Codex means the OSS harness | Inspect actual source structure; do not confuse it with a model review |
| Rocco is the new placeholder name; create private repository now | Private `srinji-kaggss/Rocco`, documentation/governance only |
| Core + TUI first | D19 delivery order settled; phone/app/web preserved |
| Yes to bounded unattended work | D31 capability required, no background job installed now |
| Adapters, not clones | D28 compatibility commitment settled |
| Defer memory until Logical-DB is done; auto decay, customizable on a scale | D33 records the hold and real typed controls; memory remains required |
| Strict and self-updating governance in the repo being worked on | AGENTS/GOVERNANCE maintenance, deterministic report and CI; not a runtime subsystem |
| Add control flow and flowchart/DAG | Specification includes component map, runtime flow, effect states and implementation DAG |
| MatrAIx is one evaluation source; do not modify it | D34 portfolio with independent benchmarks, fault tests and human trials |

Finalizing the specification is not blanket approval of every proposed
architecture decision, implementation work or billable benchmark execution.

## Corrective lessons with concrete repair

- Repo-governance language was misread as a runtime-policy choice, causing an
  unnecessary clarification. Repair: keep it repo-local in AGENTS/GOVERNANCE;
  regression: review the product inventory for an invented governance feature.
- A narrow earlier Forge search missed existing runtime types. Repair: preserve
  corrected paths and revision evidence in the ledger; do not claim global
  absence from an index/search failure or a partial directory search.
- Patch attempts with nonexistent context failed without changing files. Repair:
  remove the invalid hunk, apply against inspected bytes, verify moved paths and
  run the full structural validator. Never describe a rejected patch as applied.
- The first repo check failed with `FAIL: AGENTS.md: missing governance anchor memory`
  because the checker treated capitalization as semantic. Repair: case-insensitive
  anchor matching; positive capitalization and missing-owner negative controls.
- Independent review found that substring `kill` also matched `skill`, fences
  missed tilde/indented forms, link fragments were unchecked, a test froze counts
  against legitimate additions, and CI compared HEAD to itself. Repair: word-bound
  fault actions, Markdown fence matching, heading checks, extensible positive
  controls and actual base-to-head whitespace checks; retain adversarial tests.
- Independent specification review found E06 unallocated, E36 assigned before its
  implementation, C04 missing its durable-store prerequisite and compensation
  looping into the original effect's success state. Repair: explicit case ownership,
  a Logical-DB-to-C04 edge and compensation as a new identity with original failure
  retained. Final review checks these exact counterexamples, not general reassurance.
- Follow-up gate review found phantom headings inside code fences, inconsistent
  tilde-diagram counting, arbitrary case mentions counted as allocations, and
  remaining hardcoded growth-test fixtures. Repair: one shared fence parser,
  explicit execution-cell assignment forms, dynamic two-step growth fixtures and
  retained false-positive/false-negative regressions. The focused document reviewer
  confirmed all five earlier specification findings resolved.

## Specification v1 semantic change

R01–R43 preserved; R44–R70 specify previously implicit operational boundaries.
D25–D35 define admission, config, compaction, adapters, provider/artifact handling,
routing, schedules, lifecycle, user controls, evaluation and evolution.
E28–E54 give falsifiers; 9/54 planned cases induce actual crash/recovery.
Four harness snapshots and the completed Gemini/ChatGPT review dispositions are
in the ledger. Sources support design comparisons, not runtime readiness.

Authority: the Director requested finalization, additional requirements, visual
flows, a private repository and strict self-maintaining repo governance. No
upstream harness code or MatrAIx files were edited; unrelated work was preserved.

Verification commands live in README and AGENTS. Their actual results are the
local command receipts and the PR's CI report for its exact commit; this entry
does not predeclare a pass. Unresolved: architecture ratification, Logical-DB and
other owner readiness, P1–P8, all runtime/physical-device/fleet/human results.

Visual verification: Mermaid 11.4.1 rendered all four specification diagrams and
the governance diagram from the actual Markdown bytes in BrowserOS (5/5).
Component and DAG screenshots were inspected. This verifies that renderer, not
GitHub's runtime. The first preview script failed with a JavaScript regex syntax
error; replacing the transport-escaped regex with literal fence splitting repaired
the preview without changing diagram semantics. The preview server is owned,
loopback-only and time-bounded; no persistent service was installed.
The preview listener subsequently closed, verified by a failed loopback connect.

Local gate verification: checker passed with 70 requirements, 35 decisions,
54 falsifiers, 13 execution cells and 9 axes; `unittest discover -s tests -v`
passed 34 tests. Four tests drive isolated Python subprocess journeys: successful
CLI, failing CLI, JSON report and actual SIGKILL followed by successful re-run
with unchanged document digests. One of four is a crash/recovery journey for the
read-only GOVERNANCE CHECKER, not evidence that any Rocco runtime crash case passed.
No test changes the repository documents. `git diff --check` passed locally.

Structural coverage remains separately labeled: the standard-library in-process
trace does not include the separately executed CLI subprocesses; branch/condition
coverage is unmeasured. An initial trace reporting command failed sorting a
Python 3.14 non-line sentinel, then was corrected to report actual positive line
numbers. No total-coverage or universal checker correctness claim is made.
