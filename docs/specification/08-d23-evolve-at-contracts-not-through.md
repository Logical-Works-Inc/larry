---
type: Design
title: specification.md, part 8 of 18
description: Part 8 of /specification.md, in order; the index lists every part.
resource: /docs/specification/08-d23-evolve-at-contracts-not-through.md
tags: [rocco, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
### D23 — evolve at contracts, not through permanent generic machinery

Choose additive versioned schemas, adapter conformance fixtures, capability
negotiation, migrations with resumable checkpoints, and policy/artifact rollback.
Reject unknown required semantics; preserve unknown optional fields where the
format contract permits. Pin a reducer version for in-flight runs; new versions
enter at explicit checkpoints, never reinterpret old events silently. Rust traits
serve in-process boundaries; explicit messages serve process boundaries. Do not
promise Rust ABI stability or invent a plugin microservice for every function.
Breaking changes are possible with an explicit migration, not forbidden forever.
E25; owner compatibility/release.

### D24 — frontload decisions; keep empirical questions honestly conditional

Choose this notebook as the current candidate source of truth, with requirement,
decision, experiment and evidence IDs. Human approval records a revision and
exceptions; external model critique is advisory. Research through CodeGraph,
primary sources, cookbook and requested browser reviewers; preserve rejected
claims. All notable known choices receive a recommended default, alternative
and falsifier. Unknown future requirements cannot be decided now. Freeze
invariants and ownership; leave model/backend/hyperparameter selection inside
measurable replacement contracts. E24; owner governance.

### D25 — durable admission, human decisions and transport are separate protocols

Choose versioned Request/Run/Turn/Item identities. Acknowledgement means the input
and its idempotency digest were durably accepted, not that execution or a reply
finished. Inputs have Accepted, Pending, Promoted, Consumed and Cancelled
dispositions. Default new input steers at a safe boundary; explicit queued input
waits for the next turn. Never interrupt an already committed effect to pretend a
correction applied earlier. Same ID/different content or delivery mode is an error.
Client disconnect detaches observation; explicit cancellation stops new dispatch
and settles owned descendants. Wakes are coalesced hints, never durable work.

Questions/approvals bind requester, authorized respondent, task/effect digest,
schema, expiry and allowed answer scope. One answer cannot resolve another pending
question. Repeated/stale answers have explicit outcomes; timeout is not consent.
No UI present means AwaitingAuthority, not automatic approval. Human wait time is
not CPU execution time, but grant expiry and resource leases continue to apply.

Use the same domain implementation for embedded and served access. Declare
process, tenant, workspace, session and invocation lifetimes; no process-global
private mutable state. Version/capability negotiation precedes mutation. Headless
output is machine-readable; diagnostics go to a separate stream. Define success,
failure, awaiting-input and interrupted statuses independently of exit zero.
Public schemas are generated from the canonical contract and checked for drift.

Durable catch-up uses scope-bound cursors and bounded pages; live notifications
are disposable hints. Subscribe before reading history, deduplicate by sequence,
disconnect overflowed consumers explicitly and resume from durable history.
Expired cursors require a named resynchronization path, not silent gaps. Defaults:
256 queued notification descriptors subject to the client byte budget, and 100
items/page with a 2 MiB decoded page bound. E28/E30/E35/E36/E47; owner protocol.

### D26 — effective policy is explainable and atomically activated

Separate preference merging from authority constraints. Resource-owner policy
restricts grants; a project file or session flag cannot enlarge it. For preferences
within the same authority scope: explicit current input, project setting,
user setting, then default; more-specific trusted directory guidance applies
only to its declared subtree. Organization rules govern organization resources,
not unrelated private memory. Invalid security/config input fails visibly rather
than disappearing from the merge. Repository text is untrusted until admitted;
opening a workspace must not execute configuration, plugins or instructions.

Every setting records source, revision, scope and override reason. Scalars replace;
maps merge by key; lists replace unless their schema explicitly names composition;
ordered permission rules use an owner-defined decision algebra, not preference
precedence. Relative paths resolve against the declaring source environment.
Reject unknown required fields and conflicting equal-authority instructions.

Expose desired and effective revisions. Validate the entire candidate, activate
atomically at a declared checkpoint, retain last-known-good configuration on
failure, and report pending restart. Prompt-affecting changes enter the next
context epoch; revocation applies at the next effect boundary without waiting for
compaction. E29/E43; owner configuration with Secure-Authority constraints.

### D27 — compaction and background memory have transactional boundaries

Choose prepare, summarize, validate, commit. Failure/cancellation leaves the old
context usable. Count tool schemas and system context in the token budget; permit
at most one automatic compaction retry per overflow incident, then return an
explicit capacity failure. Summary prose cannot replace canonical commitments,
pending approvals, call/result pairings or effect outcomes. Require a smaller
admissible context before claiming successful capacity recovery.

Context epochs bind instruction, tool, source and policy versions. Preserve an
old baseline only with a visible reason and fresh effect authorization; append
versioned changes or replace the baseline at a safe checkpoint without duplicating
instructions. Provider caches do not decide whether stale instructions are valid.

After Logical-DB readiness, background memory consumers track accepted, scheduled
and applied positions separately. Applied position advances atomically with the
memory result, or an explicit failed/skipped disposition. Turns admitted after an
acknowledged shared correction consume at least that revision or visibly wait;
already-running turns retain their declared snapshot. This is a required owner
contract, not a claim that current Logical-DB already supplies it. E31/E40/E41.
