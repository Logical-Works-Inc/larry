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
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---

# Current specification inventory

requirements: 96
decisions: 53
falsifiers: 80
execution_cells: 13
axes: 9
planned_crash_cases: E02 E13 E19 E28 E31 E39 E40 E42 E50 E72 E77

Counts are document structure, not implemented features or passing tests.

## 2026-09-17: mandated local reranker evidence for repository execution

Director direction: Rocco repository work now uses the local,
on-demand `tools/rerank_local.py` before and after non-read-only execution.
The tool pins `Qwen/Qwen3-Reranker-0.6B` at
`e61197ed45024b0ed8a2d74b80b4d909f1255473`, accepts already access-filtered
JSONL candidates, caps them at 256, and returns ranked JSON evidence. Pre-exec
receipts inform planning; post-exec receipts rank verification evidence. Neither
is an authority decision, source of instructions, automatic model download,
daemon, host hook or product-runtime acceptance claim.

READ: the Hugging Face model card specifies a 0.6B reranker with 32k context and
the documented Transformers yes/no-logit method. MEASURED on this macOS host:
the exact pinned local model ranked the relevant synthetic candidate above the
irrelevant candidate (0.9921875 vs 0.0000166893). This is a two-candidate tool
smoke test, not retrieval quality, low-resource hardware, portability,
multi-tenant, fleet or human-outcome evidence. The local package import failed
for `sentence_transformers` because its installed TorchCodec dependency lacks a
compatible FFmpeg library; the tool uses the model card's direct Transformers
method instead. No Rust file changed.

Retained execution receipts: `pre-exec` ranked `local-reranker` 1.0 over
`unrelated` 0.00014019012451171875; `post-exec` ranked `verification` 0.9921875
over `unrelated` 0.0004177093505859375. Both used two synthetic, scope-safe
candidates and the pinned local artifact. Their query/candidate digests are
retained under `tools/receipts/`; each evidence command uses a new receipt
path so a prior receipt cannot be overwritten.
These receipts prove the CLI path and ordering for this bounded sample only.

## 2026-09-17: AI query workflow made executable in AGENTS.md

`AGENTS.md` now gives the agent-facing local query sequence: scope and filter
untrusted candidates, produce a unique pre-exec receipt, use rank only as
relevance evidence, perform authorized work, retain a distinct post-exec
receipt over final evidence, then run the actual repository checks. It names
the local failure modes and explicitly preserves source provenance, authority,
scope, test and human-experience boundaries.

## 2026-09-17: local reranker lifecycle bounded

The local reranker remains one-shot: it loads only during a query, defaults to
a 900-second wall-clock bound (maximum 3600), writes an evidence receipt, and
exits so operating-system process teardown reclaims model memory. A timeout is
a loud failed query, never a silently resident model service. This is lifecycle
semantics for the repository tool, not a measured RAM budget or hardware claim.

## 2026-09-17: hostile-model defense and the 8B survival gate

The Director required strict defense in depth against malicious or unreliable
models, malicious authorized compositions and the untrusted internet, with Nova
retained as sandbox owner and Secure-Authority as the authenticated capability
owner. R79–R96, D42–D53 and E63–E80 make model obedience unnecessary for
security, bind resolved effects at their domain owners, add compartment-scoped
information flow, cumulative composition limits, secret/egress binding,
collaboration isolation, revocation/recovery/update fences, exact low-friction
consent and independent mutation oracles.

The named-artifact 8B gate separates zero-violation containment from useful work:
target at least 90% verified benign completion, no more than five percentage
points below the matched same-model baseline, zero repeated approvals within
unchanged authority and no stronger-model/cloud fallback. The Ryzen 3 4300U
8 GiB and 4 GiB profiles, local-admission p95 <=50 ms and human estimate of 0–1
routine interventions are targets, NOT RUN measurements.

No Rust file changed in Rocco, Secure-Authority or Nova. No runtime, model,
hardware, coverage, fleet or human result is claimed. The prior Wasmtime owner
assumption was corrected before specification delivery: an engine can only be a
Nova implementation mechanism. Failed research/browser receipts and source limits
are retained in `research-ledger.md` rather than converted into evidence.

Local document verification on the final working-tree bytes: `python3
tools/check_governance.py` passed with 96 requirements, 53 decisions, 80
falsifiers, 13 execution cells and 9 axes; `python3 -m unittest discover -s tests
-v` passed 38/38 governance tests; `git diff --check` passed. The checker states
that Mermaid rendering and runtime/model/hardware/human trials are not checked.

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

## GitHub verification boundary

PR #1 is open against unprotected main in the confirmed private repository.
Hosted runs 35040820654 and 35040872578 failed before execution (zero steps,
runner_id 0). The check annotations contain this literal failure:

> The job was not started because recent account payments have failed or your spending limit needs to be increased. Please check the 'Billing & plans' section in your settings

`gh run view 35040872578 --repo srinji-kaggss/Rocco --log-failed` returned
`log not found: 104620215078`; the jobs and annotation APIs established that no
job steps started. Recovery inspection found Actions enabled and zero registered
repository runners. No existing no-cost runner was available for this repo.
Local checks are not relabeled as hosted CI. The workflow remains enabled and
unchanged; no billing change, new runner service, bypass or fabricated check was
made. Hosted CI is unresolved at the open-PR endpoint, not accepted as passing.
