---
type: ADR
title: "ADR 0094: a recovery fault keeps the journal, and only refused bytes set it aside"
description: Why a dead run's journal is set aside as refused only when its bytes are refused, and why a keyed retry never runs that run's body again.
resource: /docs/adr/0094-a-recovery-fault-keeps-the-journal.md
tags: [larry, adr, recovery, journal, idempotency]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: a_keyed_retry_after_a_recovery_fault_runs_nothing_and_keeps_the_journal failing on main then green, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#394, larry#321, larry#182, larry#334, ADR 0023, ADR 0037]
---

# 0094: a recovery fault keeps the journal

Status: accepted, 2026-11-23 (larry#394).

## Context

ADR 0023 sets a dead run's journal aside as `<task>.recovered`, or as `<task>.refused` when
its bytes are refused. The code set it aside as refused on every error except the lock: a
device that refused the open, a clock, a setting or a capacity too. A keyed retry (ADR 0037)
answered only from `.recovered`. So one passing fault at the next start made the journal
`.refused`, the retry found no evidence, and the body ran a second time.

## Decision

- `RuntimeError::refuses_the_record` names the refusals of the bytes: corrupt or
  over-ceiling committed bytes, a record that does not decode, a replay the contracts refuse.
  Only these set the journal aside as refused.
- Any other fault keeps the file a journal and is reported as one typed `"recovery":"kept"`
  line on stderr. The sweep goes on, and the next start recovers the file.
- A `.refused` journal is evidence too: a keyed retry is answered from it, never run.
- A retry that opens a journal already there and fails keeps it, whatever the failure.

Rejected: fail the whole start on any fault that is not a refusal of the bytes. One persistent
fault, such as a journal past the run capacity, would then refuse every later task on the
host, which is the larry#334 failure. Keeping the file and reporting it keeps the evidence
and the host.

## Consequences

- A journal with a persistent fault is tried again at each start, at most 1024 a start, and
  reported each time until an operator clears the fault.
- Falsifiers: `tests/executor_idempotency_journey.rs`
  (`a_keyed_retry_after_a_recovery_fault_runs_nothing_and_keeps_the_journal`) and
  `executor::recovery::sim::sim_each_recovery_fault_class_keeps_or_sets_aside_its_journal`.
