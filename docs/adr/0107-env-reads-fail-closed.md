---
type: ADR
title: "ADR 0107: Every environment read fails closed through one snapshot"
description: Why a variable Larry reads is unset, UTF-8 text or a typed refusal naming it, why an unset PATH is refused, and why model settings are parsed only when a task reaches a model.
resource: /docs/adr/0107-env-reads-fail-closed.md
tags: [larry, adr, executor, environment, fail-closed]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: lgwks-box cargo nextest run --workspace --locked, all tests passed, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#213, larry#239, larry#343, /docs/adr/0106-executor-decisions-on-the-injected-clock.md]
---

# 0107: Every environment read fails closed through one snapshot

Status: accepted, 2026-11-24 (larry#213). Extends the larry#239 and larry#343 snapshot.

## Context

- A read that turns bytes it could not read into "unset" lets a default decide. A
  non-UTF-8 `LARRY_RECEIPT_KEY` read as unset verifies against this machine's own key, a
  report the offline key never signed.
- Every door already reads through `executor::process_env`, which refuses a non-UTF-8
  `PATH`, `HOME` or `LARRY_*` name or value as `ExecError::NotUnicode`. One default was
  left: `Settings::from_env` read an unset `PATH` as the empty string.
- The model's settings are read through the route a task resolves to, in `model_step`,
  which a task with no brief never reaches.

## Decision

- The one helper is `executor::process_env`, over `snapshot`, a function of its variables
  (`OsString` pairs) so a sim passes them in and never sets the process environment. Its
  answers: absent is unset, valid UTF-8 is kept exactly, anything else refuses the whole
  snapshot naming the variable and never its value.
- An unset `PATH` is `ExecError::PathUnset`: one stderr line, exit 1, nothing run.
- `ModelConfig` stays parsed where a task reaches a model; a bad `LARRY_MODEL_TIMEOUT_MS`
  fails a brief task and nothing else, held by a journey.

Rejected: keep `env::var().ok()` and validate at each use. Every caller decides again, and
a non-UTF-8 key silently becomes the host key. Also rejected: a second `env_setting`
helper beside `process_env`, which is the equivalent already.

## Consequences

- Three sims prove the snapshot (64 seeds each) and three journeys prove the shipped
  doors: `verify`, `exec` with no brief, `exec` with no `PATH`.
- A caller that built `Settings` from a table with no `PATH` is refused; the tests that do
  now carry one.
- Out of scope, filed separately: per-task shell, base, timeout and deadline with tenant
  ceilings, and the local-CI receipt directory.
