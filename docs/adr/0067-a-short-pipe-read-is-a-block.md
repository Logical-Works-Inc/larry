---
type: ADR
title: "ADR 0067: a command's report states the bytes the stream yielded"
description: Why the report carries stdout_bytes and stderr_bytes, and why the noisy journey writes a fixed 3 MiB from /dev/zero.
resource: /docs/adr/0067-a-short-pipe-read-is-a-block.md
tags: [larry, adr, executor, truncation]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#319 reproduction on macOS 27.3 and Ubuntu (GNU coreutils 9.4), 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#206, larry#319, /INVARIANTS.md, https://pubs.opengroup.org/onlinepubs/9699919799/utilities/dd.html]
---

# 0067: a command's report states the bytes the stream yielded

Status: accepted, 2026-11-21 (larry#319).

## Context

`a_noisy_command_is_truncated_not_blocked` asserted `stdout_dropped == 2097152` for the
child `yes x | dd bs=1024 count=3072`. Under load it reported 2096502, 650 bytes short, and
passed alone.

The executor's reader is exact: `capture` reads the pipe to EOF, and every byte it reads
lands in the head, in the tail, or in the dropped count. So a short count means the child
wrote fewer bytes. `dd count=N` counts reads, not bytes: a read that returns fewer than `bs`
bytes is one whole block. Its input is the shell's pipe from `yes`, and when that pipe holds
a partial block, dd writes that much less. Measured with a 10 KiB target fed in 1-byte
writes: `dd bs=1024 count=10` wrote 212 to 1322 bytes on macOS and 212 to 579 on Linux.
The count was exact for a timing-dependent total.

## Decision

- `CommandResult` carries `stdout_bytes` and `stderr_bytes`: every byte the stream yielded,
  kept or dropped. The report states the stream's own length, so a person checks kept plus
  dropped against it and no byte is unaccounted.
- The noisy journey writes its 3 MiB from `/dev/zero`, which always fills a read, so `dd
  count=3072` writes 3,145,728 bytes on every host and the exact assertion holds.
- The capture sim pins that kept plus dropped is the bytes the stream yielded, for every
  seed and piece size.

## Consequences

- A dropped count that disagrees with `stdout_bytes` is now visible in the sealed report, not
  only in a test's arithmetic.
- A journey child whose output total depends on read timing is a defect in that child, not in
  the reader. `dd count=` over a pipe is one such child.
- Tests: `executor::run::tests::sim_capture_keeps_head_and_tail_and_counts_every_dropped_byte`
  and `tests/executor_journey.rs` (`a_noisy_command_is_truncated_not_blocked`).

## Rejected

- Keep `yes x | dd ...` and loosen the assertion to a range. The invariant is an exact count;
  a range hides a lost byte as readily as a slow block.
- Make the reader wait for the child's own byte report. The command is arbitrary and reports
  nothing; the reader's count is the only fact it owns.

Reversal: if a host is found where `/dev/zero` returns a short read, this ADR reopens with
that measurement.
