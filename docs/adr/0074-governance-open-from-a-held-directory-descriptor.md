---
type: ADR
title: "ADR 0074: governance open from a held directory descriptor"
description: Why the governance reader opens each governed file through a held directory with no-follow and non-blocking flags and judges the opened file.
resource: /docs/adr/0074-governance-open-from-a-held-directory-descriptor.md
tags: [larry, adr, governance, security]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#359 cases on macOS 27.3 and Ubuntu, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#359, larry#336, /docs/adr/0069-governance-reads-refuse-links-and-special-files.md, /INVARIANTS.md, https://cwe.mitre.org/data/definitions/367.html]
---

# 0074: governance open from a held directory descriptor

Status: accepted, 2026-11-21 (larry#359). It replaces the open step of ADR 0069 and keeps its rule.

## Context

ADR 0069 checked every path component with `symlink_metadata` and then opened the same path with
`fs::File::open`. Those are two calls, and a concurrent writer can replace the file between them.
The open follows a link swapped in, so the reader reads another file; a FIFO swapped in blocks the
reader forever. The check is true of a name that no longer denotes the object it checked (CWE-367).

## Decision

- The reader opens the governed root once with `lgwks_std::fs::capability::Dir` and resolves every
  directory component from that held descriptor, so a name swapped under the walk cannot redirect
  it.
- It opens the final component with `Dir::open_entry` and `OpenFlags::read`: `O_NOFOLLOW` refuses a
  final link and `O_NONBLOCK` makes a FIFO open return at once.
- The opened descriptor is judged regular by the estate's `fstat`, and `File::metadata` reads its
  length against `limit`. The read still goes through `take(limit + 1)`.
- One seam, `read_bounded_with`, runs after the check and before the open. Production passes a
  no-op; the simulation swaps a link and a FIFO through it.
- The `fs-raw` feature of the estate's own `lgwks_std` provides the descriptor. It adds no crate.

## Consequences

- A link or FIFO swapped in between the check and the open is refused by its own `ReadFault`
  variant, and the FIFO case returns in bounded time.
- The four callers keep their wording, because each still maps the same typed faults.
- Tests: `governance::sim_governance::sim_a_swapped_symlink_between_the_check_and_the_open_is_refused_by_name`,
  `sim_a_swapped_fifo_between_the_check_and_the_open_is_refused_by_name`.

## Rejected

- Keep the check and open two separate path calls. That is the defect larry#359 names.
- Build the flags in Larry with `custom_flags` and platform constants. It is a second copy of the
  estate helper and needs a new manifest edge; `Dir` already owns the syscall.
- Open blocking and judge the descriptor. A FIFO open waits for a peer, so the reader never returns.

Reversal: if a target cannot provide `openat`, this ADR reopens with that measurement.
