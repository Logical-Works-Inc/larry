---
type: ADR
title: "ADR 0046: bare larry runs each approved command in a kernel-confined copy of the folder"
description: Why the terminal session's commands run in a snapshot behind sandbox-exec or Landlock, and why the person's folder changes only when they answer yes to the diff.
resource: /docs/adr/0046-tui-isolated-commands.md
tags: [larry, adr, tui, sandbox, isolation, workspace, landlock, seatbelt]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: cargo nextest on macOS (Seatbelt enforced); Linux Landlock argv checked, enforcement NOT RUN on this host, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#243, larry#241, larry#93, logicalworks-crates#846, logicalworks-crates#1075, docs/adr/0045-one-split-terminal.md]
---

# 0046: isolated commands for the terminal session

Status: accepted, 2026-11-16 (larry#243, a part of larry#241)

## Context

Bare `larry` ran a model-chosen shell command only when `LARRY_POLICY` named `model.command`, a scope
no default policy holds (ADR 0035). A person on their own machine had to hand the whole host to the
model to use the terminal at all. The issue asks that each approved command run in an isolated copy of
the workspace, and its proof reads: the model asks for a write outside the workspace, the person
approves, the origin tree stays byte-identical.

Nova is to sandbox commands (logicalworks-crates#846). Its confinement wrapper does not exist yet, and
the estate's process type spawns through its own path with no seam for one. The gap is
logicalworks-crates#1075: a std-spawn confinement wrapper, `SandboxProfile::allow_write_beneath`, public
`PathAccess` lists.

## Decision

- The session opens an island inside the gate: a snapshot of the person's folder under
  `<state>/ws/<task_id>`, listed with `git ls-files` (tracked plus untracked, ignores applied) and
  copied file by file, links kept as links, then one baseline commit. A folder that is not a
  repository is walked only when no `.git` exists in any ancestor. A repository git cannot read is
  refused, never walked. Bounds: 20,000 files, 256 MiB, 8 MiB of listing, 64 directory levels; past one,
  `ExecError::SnapshotTooLarge` names it and the copy is removed.
- Each approved command runs behind the kernel: `sandbox-exec -p` on macOS with a profile that denies
  `file-write*` outside the copy and its scratch directory, `setpriv --landlock-access` rules on Linux.
  `Confine::probe` runs a write before the first command, so a wrapper that is present but does not
  enforce refuses the session. Another host, or a missing wrapper, is `ConfineError`, and nothing starts:
  no copy, no journal.
- The model is told its working directory is `.`. The origin's path is never in a request.
- Git calls in the person's folder carry `-c core.fsmonitor=false`, so a program named in the folder's
  own `.git/config` never runs (`sim_snapshot_folder_config_cannot_run_a_program` proves it, with a
  control that shows git does run it without the flag).
- The session ends by showing the person a summary built from the diff against the base commit id
  recorded at snapshot (every file name made safe: no control or bidi character). `y` applies it with
  `git apply`, `n` discards it. Only that answer applies. A diff cut at 1 MiB, or whose digest differs
  from the one shown, is never applied. Applying twice cannot double a change.
- A kill between the apply and the sealed report is covered by the gate's journal (ADR 0023): the same
  run answers from that evidence and its body never runs again. The next start sweeps the copy.
- `/status` and the header say commands are off, with the reason, when the host cannot confine, and say
  they run in an isolated copy when it can.
- `larry acp` keeps `Isolation::Host`: a harness runs it with no person present, under `model.command`.

## Consequences

- No host-wide grant is needed. The default policy holds no `model.command`, and the terminal runs
  commands.
- Limits, stated because the kernel rule is only a write rule: reads are open (a command can read any
  file the person can), and so is the network (a command can send what it reads). The macOS profile was
  measured to deny writes outside the copy, hard links, symlink write-through and `mv`; it does not
  deny reads. Closing them needs the estate wrapper of #1075.
- `git commit` in a command fails against the copy's config, and an ignored build output (`target/`) is
  absent from the copy, so a build starts cold. A diff of a link the command planted is shown too.
- Linux: the `setpriv` argv is asserted exactly, and enforcement was NOT RUN on this host. A colon in
  a path may break the rule syntax; that is not tested.
- Cost, measured on this macOS host with the debug binary: one command through `run_command` took p50 3
  ms plain and 8 ms confined (30 samples, `bench_confine_overhead_is_measured_against_the_unconfined_spawn`);
  Enter to the first question over a 600-file, 2 KiB-a-file folder took p50 308.9 ms, p95 310.0 ms
  against 85.6 ms and 94.2 ms for a plain `cp -R` of it (5 rounds; the path includes the PTY, the mock
  gateway, the snapshot and the probe).
- The summary and the review are the only places the folder changes, so a person who answers `n` keeps
  a byte-identical folder, `.git` included.

## Rejected

- `model.command` as the default: a host-wide grant, the thing the issue removes.
- A plain `cp -R` copy: it copies ignored build output, and it has no file or byte bound.
- Applying the copy back by copying files: nothing checks what is applied; `git apply` refuses a diff
  that does not fit the folder.
- A second sandbox of our own in Rust over `libc`: `unsafe_code` is forbidden and the estate owns it.
