---
type: ADR
title: "ADR 0019: every executor task passes larry-runtime's gate"
description: Why executor::execute submits each task to a larry_runtime Runtime as a displayed WorkProposal and runs its body only inside the bound owner reach, under the LARRY_POLICY scopes.
resource: /docs/adr/0019-executor-tasks-pass-the-runtime-gate.md
tags: [larry, adr, executor, runtime, gate, policy]
generated: {by: claude-code, at: 2026-11-08}
verified: {by: claude-code, method: full nextest run and larry exec timed before and after}
status: accepted
stale_after: 2027-02-08
sources: [larry#164, larry#68, OWASP ASVS 4.0.3 V4, arxiv.org/html/2606.28679v1, learn.microsoft.com least privilege for AI agents]
---

# 0019: every executor task passes larry-runtime's gate

Status: accepted, 2026-11-08 (larry#164, larry#68 step 4)

## Context

`larry exec`, `larry_submit_task` and `larry eval` ran a task with no authority check, and no package
outside `larry-runtime` called the runtime, so its gate guarded no path a user reaches (SD-1, SD-6). The plan
puts Track 3 on the same gate as Track 2.

## Decision

`executor::execute` submits every task to a `larry_runtime::runtime::Runtime` as a displayed `WorkProposal`.
The E65 binding covers the repo path, its canonical path, the tenant and the task bytes. The task body runs
only inside the runtime's `EffectOwner::dispatch_bound`, at most once. Every task needs `file.read` and
`process.spawn`; a brief adds `file.write` and `model.invoke`. `LARRY_POLICY` (comma-separated dotted scope
names) is the state directory's policy: unset grants those four, empty grants none, an unknown name is refused
with exit 1. A refusal is a sealed report, never accepted, carrying `denied: ["gate: policy_refuses: <scope>"]`,
and nothing is cloned, written, spawned or invoked. Scope names come from `replay::capability`, so the
mapping has one owner.

Rejected:
- A policy check in each of the three callers: three enforcement points, any one of which can be forgotten.
- A check in `execute` without the runtime: a second coverage rule beside lgwks_bot's `GrantSet` (ADR 0018).

## Consequences

The task's runtime journal lives in memory for that call and becomes durable with larry#75. Each state
directory is one tenant until larry#89 brings tenant keys. The runtime's public items now have a caller
outside the crate, so the falsifier ports (#98 to #101) can land.
