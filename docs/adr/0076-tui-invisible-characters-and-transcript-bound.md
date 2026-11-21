---
type: ADR
title: "ADR 0076: invisible characters never reach a frame, and a transcript entry is bounded"
description: Format characters are dropped or shown as a marker, and a Ran entry keeps only the rows a frame draws.
resource: /docs/adr/0076-tui-invisible-characters-and-transcript-bound.md
tags: [larry, adr, tui, security, bound]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: sim_tui and sim_parts tests, larry#337, 2026-11-21}
status: accepted
stale_after: 2027-05-21
sources: [larry#337, CVE-2021-42574, Unicode 16.0.0 DerivedGeneralCategory.txt, /docs/adr/0045-one-split-terminal.md]
---

# ADR 0076: invisible characters never reach a frame, and a transcript entry is bounded

Status: accepted, 2026-11-21 (larry#337).

## Context

`sanitize` dropped ASCII control characters, the 8-bit control block and bidi
controls and kept every `Cf` character, so a paste or a model reply could carry
zero-width joiners, tag characters or a soft hyphen into a frame. A command shown for approval could then differ from the
command that runs, and a terminal can reorder text around bidi controls
(CVE-2021-42574). The gateway refusal text had the same gap.

A `Ran` transcript entry also kept every output line. With 512 entries of large
output, the transcript held tens of MiB while a frame drew five rows of each.

No locked crate exposes Unicode general categories: `unicode-width` gives display
width only, and a new dependency is not allowed.

## Decision

- `src/tui/style.rs` holds one sorted table of the Unicode 16 `Cf` ranges plus
  `Zl` (`U+2028`) and `Zp` (`U+2029`), taken from
  `extracted/DerivedGeneralCategory.txt`. `sanitize` drops every character in it.
- `sanitize_shown` is `sanitize` for a command the person approves: each dropped
  character is written as `⟨U+XXXX⟩`, so the command on screen is the one that
  runs. Tabs stay four spaces and newlines stay.
- `src/model/wire.rs::bounded` replaces bidi controls with a space, as it already
  did for controls.
- `Entry::ran` keeps the last `OUTPUT_TAIL` (5) output lines, each cut to 512
  characters, and the count of the lines it did not keep. 512 entries therefore
  hold at most 8 MiB of UTF-8;
  `sim_tui_a_full_transcript_of_large_outputs_stays_bounded` checks it.

## Consequences

- A frame carries no invisible, separator or control character; the approval view
  shows a marker instead, and the marker is plain text a terminal can draw.
- The transcript's size no longer follows the size of a command's output.
- A row longer than 512 characters keeps its first 512; the frame drew fewer
  cells anyway, and the artifact pane keeps the same row bound.
