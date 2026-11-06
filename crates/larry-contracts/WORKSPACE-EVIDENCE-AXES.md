---
type: Evidence
title: "Workspace continuation slice: nine-axis report and tool failures"
description: Source provenance, public-interface falsifiers and explicit limits of the first workspace coordinator.
resource: /crates/larry-contracts/WORKSPACE-EVIDENCE-AXES.md
tags: [larry, workspace, contracts, evidence]
generated: {by: agent:codex, at: 2026-09-19}
verified: {by: agent:codex, method: source review and public Rust interface tests}
status: component-verified-owner-integration-unavailable
stale_after: 2026-10-19
sources: [/specification.md, /research-ledger.md, /crates/larry-contracts/src/workspace.rs, /crates/larry-contracts/tests/workspace.rs]
---

# Workspace continuation slice: nine-axis report and tool failures

Part 1 is [WORKSPACE-EVIDENCE.md](WORKSPACE-EVIDENCE.md).

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
