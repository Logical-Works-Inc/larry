---
type: ADR
title: "ADR 0047: the artifact pane shows a task's output, diff and sealed receipt"
description: Why the wide terminal's right side is a tabbed pane fed by two new session events, and what bounds and keys it has.
resource: /docs/adr/0047-artifact-pane.md
tags: [larry, adr, tui, artifact-pane, diff, receipt, observer]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: cargo nextest of tui::pane::sim and the PTY journey, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#244, larry#241, docs/adr/0045-one-split-terminal.md, docs/adr/0046-tui-isolated-commands.md]
---

# 0047: the artifact pane

Status: accepted, 2026-11-16 (larry#244, a part of larry#241)

## Context

ADR 0045 kept the right side of a screen of 108 columns or more for artifacts and left it empty. A
person asked to apply a change saw only a summary in the chat, and the sealed receipt never reached the
frame: the worker dropped the report.

## Decision

- `tui::pane` owns the pane: three tabs (Output, Diff, Receipt), Tab walks them, PgUp and PgDn scroll
  the shown tab. While the pane shows, those keys belong to it, the question included, so the diff can
  be read before the answer; Tab still completes while the slash popup is open. A narrower screen has
  no pane and the chat keeps one line, `diff: N files, +a -r`.
- Two events join the worker's: `Event::Diff(String)`, the change (at most 1 MiB), sent from
  `Island::hand_off` through the new defaulted `Observer::diff` just before the review question; and
  `Event::Receipt`, built from the sealed report (BLAKE3 prefix of its bytes, accepted, commands run,
  declined and interrupted, the hand-off), sent after the session's last command and before `Ended`.
  The default `diff` does nothing, so `larry acp` is unchanged.
- Every artifact is cut and sanitized once, on arrival: output 512 rows (the oldest drops, counted),
  diff 4,096 hunk rows after the file list (the rest counted), 64 files listed, 512 characters a row.
  A frame paints only the body rows, so its cost follows the screen, and an unchanged pane writes
  nothing. A new task releases every artifact.
- The diff is read by a hunk state machine: a header before a file's first `@@` is muted, inside a hunk
  `+` is good, `-` is bad, so a removed line that reads `--- x` is removed, not a header.

## Consequences

- Output appears when a command ends, not while it runs: `run_command` captures to the end and the
  observer has no stream hook. The pane shows `running <command>` meanwhile. Live output needs a
  streaming seam in `executor::run`, a separate change.
- On a wide screen PgUp and PgDn no longer scroll the chat; the chat scrolls on a narrow one.
- The receipt shows what the sealed report says; `larry verify` stays the check of the seal.

## Rejected

- Painting the diff from the sealed report after `Ended`: the person answers before it exists.
- Reusing `Entry::Info` for the diff: it re-wraps every row of every entry on every frame.
