---
type: ADR
title: "ADR 0037: a run id never repeats, and a client retry names one request"
description: Why an executor run id is a UUID version 7 rather than a digest, pid and counter, why a task may name an idempotency key, and how a retry is answered from evidence instead of a second dispatch.
resource: /docs/adr/0037-run-identity-and-idempotency-keys.md
tags: [larry, adr, executor, idempotency, uuid, larry-182]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: nextest run of the keyed journeys, the identity sim and the e47 duplicate case on Oracle}
status: accepted
stale_after: 2027-02-14
sources: [larry#182 defect 3, RFC 9562 section 5.7 and 6.2, draft-ietf-httpapi-idempotency-key-header-07, AWS client tokens, Idempotent Receiver pattern, docs/adr/0019-executor-tasks-pass-the-runtime-gate.md, docs/adr/0025-durable-admission.md]
---

# 0037: a run id never repeats, and a client retry names one request

Status: accepted, 2026-11-14 (larry#182 defect 3)

## Context

A run id was the task's BLAKE3 prefix, `std::process::id()` and a per-process counter. Two consequences
followed. A client retry of the same task got new ids, so the runtime's duplicate rule never fired and the
body ran twice. In a container with a fixed pid, a new process could mint the id of a killed earlier run,
and `owner_alive` was true for its own pid, so recovery skipped that journal, the command restored as a
duplicate replay, and a signed report could say `gate: denied` for work that was never judged. The ingress
also hid the admission result: it returned only the version, so a caller could not tell a new request from
a duplicate.

## Decision

- A run id is one canonical UUID version 7 text from `lgwks_std::id::Uuid::new_v7` (RFC 9562 section 5.7,
  the dedicated counter of section 6.2). It carries a 48-bit millisecond stamp, a 12-bit counter and 62 bits
  of OS entropy, and no process id at all; a refusal is the typed `ExecError::Id`, never a substituted value.
- A task document may name an `idempotency_key`: 1 to 256 bytes, no control byte, refused typed otherwise.
  With a key, the run id is derived from the task digest, that key and the state directory's tenant, so a
  retry of the same document maps to the same run id and the same command id.
- `submit_protocol` returns the `CommandAcceptance` with the version, so a caller sees
  `DuplicateReplay`.
- The executor answers a duplicate from evidence and never re-runs the body: the sealed report the ledger
  keeps under that run id when the earlier run settled, or the statement that the run was recovered from its
  journal as unknown when it died mid-task. A journal's own file lock is the liveness proof, so neither the
  sweep nor the recovery asks a process id whether a run is live.

Rejected:

- Keying the run id on the task digest alone. It needs no new field, but two deliberate runs of one task
  collapse into one: the second answers from the first's evidence and the operator cannot run a task twice.
- Letting the client name the run id itself. It is one field fewer, but nothing binds the id to the tenant
  or the content, so one run could adopt another's journal or a foreign tenant's evidence.

## Consequences

- A keyed run costs one extra ledger row and one ledger read on its retry; an unkeyed run costs neither.
- The run id is no longer derivable from the task bytes alone, so the journals and workspaces of one task
  are told apart by one canonical UUID text and nothing else. This is what makes the name safe under a
  fixed pid.
- A keyed run that died mid-task is refused as recovered rather than retried; the operator decides whether
  to resubmit under a new key.
- `larry eval` and `larry_submit_task` name no key, so every task they run is a fresh run.
- Covered by `tests/executor_idempotency_journey.rs`, `executor::identity::tests` and
  `tests/e47_protocol.rs`.
