---
type: ADR
title: "ADR 0078: the workspace sweep resets modes through held descriptors"
description: Why the workspace mode reset walks held directory descriptors and never changes a mode by path, and how each platform applies a no-follow reset.
resource: /docs/adr/0078-the-workspace-sweep-resets-modes-through-held-descriptors.md
tags: [larry, adr, workspace, security]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#366 sims and leftover journeys on Oracle Linux, clippy, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#366, larry#334, /docs/adr/0075-bounded-drain-and-leftover-sweep.md, /docs/adr/0074-governance-open-from-a-held-directory-descriptor.md, /INVARIANTS.md, https://cwe.mitre.org/data/definitions/367.html, https://cwe.mitre.org/data/definitions/59.html]
---

# 0078: the workspace sweep resets modes through held descriptors

Status: accepted, 2026-11-21 (larry#366). It replaces the mode reset of ADR 0075 and keeps its rule.

## Context

A command can leave a directory mode 000, and removing a tree has to list each directory, so ADR
0075 reset every directory to owner rwx before removal (larry#334). It checked each entry with
`symlink_metadata` and then called `std::fs::set_permissions` by path. Those are two calls, and a
process that left the command's process group is still alive while the workspace drops. It can
replace the directory with a link between them, and the chmod then follows the link and changes a
file the operator owns outside the tree (CWE-367, CWE-59).

## Decision

- The walk opens the workspace's parent once with `lgwks_std::fs::capability::Dir` and resets and
  opens every directory from that held parent, so a name is never resolved through a path. The
  path listing is only a hint; every mode change and every descent re-resolves the name from the
  held descriptor.
- Darwin and the BSDs reset a mode with `rustix::fs::chmodat(..., AT_SYMLINK_NOFOLLOW)`. Their
  `fchmodat` changes a link's own mode instead of following it, so a link swapped in is not
  followed.
- Linux's `fchmodat` implements no no-follow flag, and rustix reports `OPNOTSUPP` there. The reset
  opens the name with `O_PATH | O_NOFOLLOW | O_DIRECTORY` and then calls `set_permissions` on the
  descriptor's own `/proc/self/fd` name, which pins the object the open admitted. A link or a file
  swapped in is refused by the open itself.
- A link found where a directory was is kept as it is, reported as one typed JSON event
  (`{"workspace":"link","entry":...}`) on stderr, and the walk goes on.
- One seam, `reset_with`, runs after an entry is judged a directory and before its mode is reset.
  Production passes a no-op; the simulation swaps a link or removes the name through it.
- The reset lives in `src/executor/workspace/mode.rs`, so `workspace.rs` stays under the size law.

## Consequences

- A link swapped in between the check and the reset never changes the target's mode. The simulation
  measured the sentinel at 0o700 before this ADR (the escape) and 0o640 after.
- A mode-000 directory is still reset and its tree still removed, by both journeys.
- A link the tree already holds is no longer silently skipped: the operator gets one line per link.
  A name that vanishes before it is judged, or a plain file, is skipped without a fault.
- `rustix` is a direct edge of `larry-demo`, already in the lock through `lgwks_std`'s `fs-raw`, so
  no crate is added. Its manifest line cites Logical-Works-Inc/logicalworks-crates#1113, the estate
  call that replaces it.
- MacOS is compiled only on a Mac run; Linux CI covers the `O_PATH` arm.
- The commit gate showed a pre-commit hook's `GIT_DIR` reaching a journey fixture that starts git
  directly (`tests/executor_journey.rs`), so its probe commit landed in the repository that ran the
  suite. That call now clears the four inherited repository variables, as `tests/support/origin.rs`
  does since larry#375.

## Rejected

- Keep `set_permissions` by path and re-check the entry after it. The sentinel's mode is already
  changed by then, so the check reports a loss it cannot undo.
- Reset by opening the child `O_RDONLY`. A mode-000 directory refuses that open, so the tree stays
  undeletable.
- Hardcode `O_NOFOLLOW` in Larry with `custom_flags`. It is a platform constant and a second copy
  of a syscall the estate or rustix already owns.
- Treat an unreadable directory as a fault and keep the workspace. A command can make every run's
  workspace permanent that way, which is what larry#334 fixed.

Reversal: when Logical-Works-Inc/logicalworks-crates#1113 lands a `Dir` mode reset, this ADR's
`rustix` calls move onto that one estate call and the direct edge leaves.
