---
type: Evidence
title: Bounded workspace continuation slice
description: Source provenance, public-interface falsifiers and explicit limits of the first workspace coordinator.
resource: /crates/rocco-contracts/WORKSPACE-EVIDENCE.md
tags: [rocco, workspace, contracts, evidence]
generated: {by: agent:codex, at: 2026-09-19}
verified: {by: agent:codex, method: source review and public Rust interface tests}
status: component-verified-owner-integration-unavailable
stale_after: 2026-10-19
sources: [/specification.md, /research-ledger.md, /crates/rocco-contracts/src/workspace.rs, /crates/rocco-contracts/tests/workspace.rs]
---

# Outcome and scope

MEASURED: a bounded coordinator dispatches a typed Continue request through a
replaceable owner boundary. An unavailable owner returns AwaitingDependency;
the observed run state never advances because somebody clicked a work card.
Selecting work and typing Continue must use this same boundary in the surface.
The UI integration and complete fixture journey are recorded by the frontend
change, separately from these public contract tests.

READ: R20/R44/R51/R52/R64 and D14/D25/D28 require system facts, explicit
lifetimes, scoped projections and honest admission. E28/E35/E36/E48 remain
full product falsifiers; this component exercises only their local boundary
predicates. Moo owns workspace claims/effects; Logical-DB owns durable state.
No durable admission, memory implementation, provider call, workspace mutation,
authentication, real Moo connection or production backend is claimed here.

The problem is preserving a selected work identity through continuation without
turning display data into execution authority. A whole Forge scheduler import
was rejected because its MooPastureManager allocates synthetic claims. Direct
UI state advancement was rejected because a view cannot prove admission.
The accepted tradeoff is a disposable bounded projection and a narrow adapter:
useful local coordination now, explicit unavailable execution until the owners
supply authenticated atomic admission and recovery.

## Sources and semantic delta

READ on 2026-09-19: Forge `crates/forge-runtime/src/coordinator/scheduler/moo.rs`
lines 201–239 supplies the baseline/fence refusal pattern. SHA-256:
`3f15ebbb19b1724adca714f13e3408c508c1a14523c5cc317210e0fe7b695200`.
The adapted module retains the complete MIT notice. It replaces string identities
with existing typed IDs plus WorkspaceId, checks the complete version binding,
and excludes synthetic allocation, process spawning and workspace ownership.

READ: Moo `src/claims.rs` (SHA-256
`bae249e51421d45db05f5b80d3532c5cf40c062d993afdbd4b248a1c310b61a9`)
uses an IMMEDIATE transaction for atomic scope claims; `src/live_workspace.rs`
(SHA-256 `a30c6726030e8644ccdb7789e879e7da0accde5496f745d9f3a81ba77d188757`)
supplies project-scoped workspace summaries. Neither file is modified or copied.
CodeGraph confirmed Forge callers and existing index; Moo exploration followed
workspace_summary to its real consumers. The Rust corpus returned irrelevant
hits; Context7 resolution succeeded but its initial docs result was a HashMap
example and historical BTreeMap note, not performance evidence.

`src/workspace.rs` owns projection validation, retry identity and owner handoff;
`src/ids.rs` adds WorkspaceId using the resident UUID newtype implementation;
`src/lib.rs` exposes the module and corrects the previous all-stateless claim;
`tests/workspace.rs` exercises the public boundary with explicit test owners.
No dependency, feature, lockfile or third-party admission changed in this slice.

## Declared bounds and verification

Domain: one authenticated host-provided tenant/workspace scope per coordinator,
at most 256 work descriptors and 256 retained Continue identities, title at most
256 UTF-8 bytes, arbitrary RunId/CommandId values, u64 snapshot revisions/fences.
Empty/blank/control-bearing titles fail. New snapshots can skip revisions but
cannot regress, conflict under one revision, or resurrect a terminal state.
All bounds use safe rejection without eviction. Exclusive mutable access
serializes coordinator calls; it does not establish distributed concurrency.

Owner: the view owns these allocations until drop; it creates no process, port,
poller or disk record. Owner adapters must revalidate authenticated authority,
revision, baseline and fence atomically and deduplicate requests durably.
Reconnect retries retain the command identity. Process-loss reconstruction and
durable reconciliation are UNVERIFIED, not implemented by this projection.

The bounded behavior inventory below targets every predicate passing with zero
unauthorized adapter invocations or observed-state mutations. MEASURED 12/12
named public-interface falsifiers passed. This is not structural coverage or
proof of all product assertions, real effects, concurrent isolation or recovery.

| Public-interface falsifier | Required local predicate |
|---|---|
| title_byte_bound_and_controls_are_enforced_at_construction | Empty/control/overlong input rejects; exact multibyte boundary accepts |
| board_rejects_scope_staleness_and_conflicting_snapshots_without_mutation | Wrong tenant/workspace and stale/conflicting revisions preserve original row |
| terminal_projection_cannot_be_resurrected_with_newer_revision | Every terminal disposition refuses resurrection |
| full_board_keeps_all_rows_and_permits_existing_row_refresh | Row 257 refuses, existing row refreshes, iteration is stable |
| disconnected_continue_is_real_awaiting_dependency_not_running_or_success | 100 identical retries preserve observed state; reconnect invokes owner |
| queued_identity_replays_original_receipt_without_redispatch_or_completion_claim | Duplicate returns identical receipt with one owner call, including later observation |
| invalid_selection_never_reaches_the_owner | Wrong scope, missing run and stale selection make zero owner calls |
| terminal_and_cancelling_work_cannot_dispatch | All terminal states and cancelling make zero owner calls |
| uncertain_or_refused_attempt_cannot_reuse_identity_for_other_content | Failure retains request binding; identical retry remains possible |
| owner_binding_changes_and_foreign_receipts_are_refused | Revision/baseline/fence mismatch and foreign receipt fail visibly |
| retry_capacity_never_silently_evicts_an_existing_identity | Identity 257 refuses; first identity and conflict detection survive |
| identical_run_and_command_ids_in_different_tenants_remain_separate | Two scoped coordinators retain distinct rows and receipt scope |

MEASURED commands, all exit 0: `cargo test -p rocco-contracts --locked --lib`
(57 existing unit tests); `cargo test -p rocco-contracts --locked --test workspace`
(12 public-interface tests); `cargo clippy -p rocco-contracts --all-targets
--locked -- -D warnings`; `cargo test -p rocco-contracts --no-default-features
--locked --test workspace` (12 tests). Workspace-wide gates and delivery belong
to the integrated change, not this agent's return. Crash cases: 0; no durable
owner or process recovery exists in this slice. Structural coverage NOT RUN.

## Required nine-axis edit report

| Axis | Required invariant | Evidence required |
|---|---|---|
| Frontier | The selected design beats credible current alternatives on the actual problem without sacrificing correctness. | Dated primary research, named alternatives and matched capability/cost measurements. Familiarity or small size cannot establish superiority. |
| Hyperscale | More than 1,000,000 active concurrent users remain correct under heterogeneous compute, network conditions, tenant skew and failures. | Sustained load, burst, saturation and recovery results with workload definitions, resource bounds and raw telemetry. |
| Idiomatic Rust | Ownership, state transitions, errors and resource lifetimes are explicit and sound. | File-specific design review, compiler/lint results, applicable soundness checks and exercised failure paths. The existing Python hook does not satisfy this requirement. |
| Generalized | Behavior works across the declared input and deployment family without special-casing test examples, identities or topology. | Independently chosen cases, boundary/property tests and configuration variation; no hardcoded acceptance shortcuts. |
| Decoupled | Components have explicit owners and contracts and can be replaced independently without changing required behavior. | Replacement tests, dependency analysis, contract compatibility and version-skew results. |
| Ephemeral | Correctness survives process loss; durable state has an explicit owner outside disposable workers. | Crash/restart/replay tests proving state integrity, idempotency, cancellation and recovery. |
| Portable | The same required semantics hold on every declared OS, architecture and runtime target. | Reproducible builds and actual end-to-end results per target, with untested targets visible. |
| Multi-tenanted by default | Identity, data, caches, receipts, quotas and recovery remain isolated under concurrent use. | Cross-tenant negative tests, authenticated scope propagation and noisy-neighbor/failure-isolation measurements. |
| Fastest possible correct Rust | Performance comparisons preserve every required semantic and account for the complete execution path. | Matched CPU, allocation, memory, I/O, contention, throughput and p50/p95/p99 results against credible alternatives; justified lower bounds where applicable. |

File-specific evidence for all four Rust files: compiler/clippy and default/no-
default-feature tests above support local Idiomatic Rust and Generalized
predicates only. Adapter replacement/absent-owner/receipt tests support a bounded
Decoupled predicate; real owner compatibility and version skew remain UNKNOWN.
Cross-scope tests support local Multi-tenanted isolation, not authenticated
concurrent fleet isolation. Frontier matched alternatives, Hyperscale, Ephemeral
process recovery, Portable non-macOS/Ryzen runs and Fastest possible correct Rust
full-path comparisons are UNVERIFIED. All original numerical targets remain.

Human outcome ESTIMATE: routing selection through the same Continue action may
remove one explicit command per return-to-work journey, assuming the selected
work is current and its owner connected. Real participants: 0. Time, cognitive
burden, error/rework reduction, accessibility and recovery benefit are UNKNOWN.
Whole-client memory/CPU/latency/energy, sustained lifecycle growth, physical Ryzen
3/Ryzen 5, graphics and million-user acceptance remain NOT RUN by this slice.

## Retained tool failures and query evidence

The first query failed because Framework Python lacked torch; the second failed
because stray empty global numpy-2.5.3 metadata masked installed numpy 2.4.4.
A task-local symlink overlay points to genuine installed 2.4.4 metadata; runtime
and metadata versions were verified equal, and no shared package was altered.
The third query passed with the exact pinned local model; its retained receipt
is `tools/receipts/20260919-backend-workspace-pre-exec-3.json`. Ranked source
relevance selected existing contracts and Moo ownership; ranks are not authority.
The post-execution query also passed with the same pinned model, retaining
`tools/receipts/20260919-backend-workspace-post-exec.json` over the observed
tests, failures and unsupported product claims. `rustfmt --check --edition 2024`
on the four changed Rust files exited 0. The shared environment overlay remains
task-owned until the frontend finishes its own queries, then moves to OS Trash.
Initial rg/CodeGraph/Python startup stalled in dyld; stopped only owned searches.
CodeGraph's guessed --depth option failed; inspected help and used --max-files.

Compute: one delegated backend agent, three pre-query attempts (two failed before
model inference) and one successful post-query, existing local weights/packages reused, no additional agent,
no provider model call or package/model download. Exact token spend is UNKNOWN.
