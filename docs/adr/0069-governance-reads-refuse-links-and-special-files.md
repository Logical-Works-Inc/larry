---
type: ADR
title: "ADR 0069: governance reads refuse links and special files"
description: Why one reader walks each path component, refuses a symlink anywhere, requires a regular file and reads one byte past its bound.
resource: /docs/adr/0069-governance-reads-refuse-links-and-special-files.md
tags: [larry, adr, governance, security]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#336 cases on macOS 27.3 and Ubuntu, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#336, /INVARIANTS.md, https://cwe.mitre.org/data/definitions/59.html, https://pubs.opengroup.org/onlinepubs/9699919799/functions/mkfifo.html]
---

# 0069: governance reads refuse links and special files

Status: accepted, 2026-11-21 (larry#336).

## Context

`larry governance` checked three readers with `fs::metadata`, which follows a symbolic link, and
then read the whole file with `fs::read`. A link to a device of length zero passed the size check
and was read with no end; a FIFO blocked the reader. `governance::documents` checked only the last
path part, so a symlinked `docs` directory was read through. INVARIANTS already said a symlinked
document is refused, so three readers did not keep their own rule.

## Decision

- One reader, `governance::read::read_bounded(root, relative, limit)`, holds the rule for all four
  callers: the receipts, the tracker, the temp-path sources and the documents.
- It walks each path component from the governed root with `symlink_metadata` and refuses a
  symbolic link anywhere on the path, never following it (CWE-59).
- It requires a regular file before it opens one, so a FIFO, a device, a socket or a directory is
  refused by name without a blocking or endless read.
- It reads through `take(limit + 1)`, so one byte past the bound is refused and memory never holds
  more than the bound plus one byte.
- Each caller maps the typed `ReadFault` to its own wording, so the Python checker's messages stay.

## Consequences

- A link, a FIFO, a device and an oversize file are refused in bounded time and memory, and the
  refusal names the path.
- `governance::documents` now refuses any file under a symlinked `docs` directory instead of
  reading it.
- The receipts reader names an unreadable directory as an error rather than reporting no receipts,
  so an operator sees the failure (larry#336).
- Tests: `governance::sim_governance::sim_a_link_to_a_device_is_refused_by_name`,
  `sim_a_fifo_is_refused_by_name`, `sim_a_symlinked_docs_directory_is_refused_by_name`.

## Rejected

- Four per-file checks. The defect larry#336 names is three readers that each got the rule wrong;
  a fourth copy of it would drift the same way.
- Trust the metadata length alone. `fs::metadata` follows the link, so a device of length zero
  passes the bound; the file type must be judged before the open.
- Open with `O_NONBLOCK` and rely on the read to refuse. A FIFO still opens, and the read then
  decides the answer from a special file rather than a regular one.

Reversal: if a host cannot report a file type with `symlink_metadata`, this ADR reopens with that
measurement.
