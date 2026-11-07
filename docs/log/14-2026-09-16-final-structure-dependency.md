---
type: Log
title: log.md, part 14 of 15
description: Part 14 of /log.md, in order; the index lists every part.
resource: /docs/log/14-2026-09-16-final-structure-dependency.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-16: final structure, dependency root and SOTA gap fold-in

| Direction | Disposition |
|---|---|
| Create the full final repository structures, READMEs and documentation in full OKF so the package is finalized | docs/ tree created (README, architecture, dependencies, evaluation); every document carries the complete OKF field set; structural gate extended to admit and validate the tree with adversarial negative controls; governance version 1.1 |
| Logicalworks-Crates is the core starting point for dependencies | Settled starting point recorded in AGENTS.md and /docs/dependencies/README.md with ledger-pinned revisions; D21 reuse mechanics unchanged and still pending ratification |
| Fold the 2026-09-16 SOTA gap research into the specification | R71–R78, D36–D41 and E55–E62 added as proposed, approval pending; triad review dispositions and arXiv/protocol receipts recorded in research-ledger.md |

Semantic change: specification v1.1 adds pure-authorization replay identity,
unified reconciliation metadata, ecosystem protocol adapters (Agent Skills,
pinned MCP revision, A2A boundary), operations telemetry with cost
attribution and request shedding, budget leases, memory edge semantics and
evaluation hardening. Falsifier inventory grows 54 to 62; the planned crash
fraction stays 9/62 (above 10 percent). Every new case is allocated to an
execution cell in Cell 11. The docs/ tree adds navigation only; no canonical
inventory item is defined twice. No Rust files changed; no runtime, model,
hardware or human result is implied.

Authority: the Director requested the final structure, the OKF finalization
and the Logicalworks-Crates dependency root in conversation 2026-09-16.
External reviewer content (Gemini, ChatGPT, Claude) remains advisory.

Local gate verification: `python3 tools/check_governance.py` passed with 78
requirements, 41 decisions, 62 falsifiers, 13 execution cells and 9 axes;
`unittest discover -s tests -v` passed 38 tests (34 prior plus 4 new
docs-tree negative controls). `git diff --check` passed. Two intermediate
failures were retained and repaired: the checker correctly reported log
inventory drift while the specification was already extended (12 downstream
test failures until counts were reconciled), and one new negative control
first mutated a string absent from docs/README.md and then asserted a wrong
error-message substring; the control now mutates a real link and matches the
validator's actual "unresolved or escaping local link" message. The drift
detection was exercised, not weakened. Mermaid rendering is NOT claimed for
this change: no new diagrams were added, and fence checks do not render.

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
