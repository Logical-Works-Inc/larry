---
type: ADR
title: "ADR 0145: a missing program is refused before the wrapper"
description: "The spawn owner looks a command's program up before setsid or the confinement stands in front of it."
resource: /docs/adr/0145-a-missing-program-is-refused-before-the-wrapper.md
tags: [larry, adr, executor, processes, eval]
generated: {by: claude, at: 2026-11-28}
verified: {by: claude, method: lgwks-box cargo nextest run --locked on the Linux build box, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#598, larry#217, INVARIANTS.md, src/executor/run.rs, src/executor/program.rs]
---

## Context

On Linux a command runs as `setsid <argv>`. A program that is not on the PATH let
`setsid` start and then exit 127. `larry eval` read that exit as the verifier's
judgement and scored the task rejected. macOS has no `setsid`, so there the same
fault was a typed spawn error. A confined command had the same fault on both
hosts, because the wrapper answers with its own exit code.

## Decision

`command_for`, the one spawn owner, looks the program up before it wraps it. The
rules are those of execvp(3): a name with a slash is that file; a bare name is
the first executable file on the command's PATH. A miss is `ExecError::Spawn`
naming the program.

One case stays with the wrapper: a relative path with a slash in a confined tree.
The model writes that tree, so a missing file there is the model's failure.

## Rejected

Read exit 127 and the `setsid` message after the run. The command itself can exit
127 and print those words, so a real judgement would read as a host fault.

## Consequences

The lookup costs one `stat` per PATH entry. `command_for` with the lookup took
p50 9.7 us, p95 10.2 us and p99 17.7 us (4,000 samples, 10 PATH entries with the
program in the last, dev profile, Linux ARM64 build box).

Two cases are not covered and still end as the wrapper's exit code: a script
whose `#!` interpreter is missing, and a file with an execute bit that this user
may not run. Reverse this ADR if `lgwks_bot` gives a spawn that reports an
exec failure behind `setsid`.

## Proof

Before the change all five new tests failed on Linux, and both binary journeys
exited 0 with the task scored. After it, all five pass.

## Traceability

Issue #598. The invariant line is in INVARIANTS.md.
