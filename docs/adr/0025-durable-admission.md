---
type: ADR
title: "ADR 0025: durable admission before acknowledgement"
description: How a protocol command is journaled and synced before it is acknowledged, how the pending set is rebuilt from the journal, and why a correction waits for its run's effects to settle.
resource: /docs/adr/0025-durable-admission.md
tags: [larry, adr, runtime, admission, e28, recovery]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: nextest run of runtime::admission::tests, e47_protocol and the executor journeys}
status: accepted
stale_after: 2027-02-09
sources: [larry#100, larry#21, specification R44, D25, D04, E28, docs/adr/0020-protocol-negotiation-before-ingress.md, docs/adr/0023-journal-on-record-journal.md]
---

# 0025: durable admission before acknowledgement

Status: accepted, 2026-11-09 (larry#100, the E28 port of PR #21)

## Context

`submit_protocol` queued a command in memory and returned the negotiated version, which is the caller's
acknowledgement. The command reached the journal only when a tick admitted it, so a kill -9 between the two
lost an input the caller had been told was taken (R44, E28). A `Steer` was recorded and nothing else: no
rule said when a correction may take effect while one of its run's effects is still open (D25).

## Decision

- `Runtime::admit` journals the command through the contract's `accept` rule and syncs the journal's file
  before it returns. Every protocol command goes through it, so the acknowledgement follows the durable
  write. A retry under the same identity and content is `DuplicateReplay`. Other content under that
  identity is `ConflictingRetry`, and nothing is written.
- Admitted is not accepted. `CommandAccepted` records the reduction, and the pending set is the journal's
  commands without it, kept in journal order and bounded by `MAX_READY_WORK`. `restore` rebuilds the set
  from the journal alone, so a restored runtime reduces each acknowledged command exactly once. A journaled
  command whose payload is not a proposal was never acknowledged here (both paths decode first), so it
  is not pending, and the consumer's digests expose the altered file (E35).
- A command for a run another tenant owns is refused. The owner is taken from the run table or from a command
  already pending for that run, and the refusal writes nothing.
- A `Steer` is promoted, meaning its `CommandAccepted` is recorded, only once every live effect of its run is
  `Reconciled` as succeeded, failed or compensated. A reserved, observed-but-unresolved or permanently
  unknown effect keeps it pending. Later commands of that run wait behind it, and other runs pass it. The run's
  active correction is its last promoted `Steer` in the journal.
- A displayed submission (every executor task) is admitted the same way. Its display is host evidence
  (E65), checked against the command at admission and held in memory beside it. A restored command has
  none, so its reach refuses an owner that needs a binding. A ready command that `submit_displayed` queued
  and that lands behind a pending one joins the pending set with its display.

Rejected:
- PR #21's caller-owned `JournalStorage` seam. The journal file is `lgwks_bot::journal::RecordJournal` since
  ADR 0023, so a second storage trait would be a second owner.
- A correction table in the runtime. The journal already records each promotion, so a map beside it would
  be a second record. It would also be a `pub` reader with no caller.
- Promoting a correction at any tick. That would let a correction contradict an effect whose outcome is
  still unknown, and D25 forbids rewriting a committed effect.

## Consequences

- An acknowledged protocol command costs one journal sync at admission, on top of the tick's group commit.
- `submit` and `submit_displayed` keep their memory queue for in-process callers. Only protocol ingress is
  an acknowledgement, and E47's overload is now the pending set's bound: a duplicate takes no capacity.
- Covered by `runtime::admission::tests` (the seeded crash sim, correction, permanent unknown, tenant and bound
  cases) and `tests/e47_protocol.rs`.
