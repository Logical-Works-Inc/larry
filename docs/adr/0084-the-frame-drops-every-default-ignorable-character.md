---
type: ADR
title: "ADR 0084: the frame drops every default-ignorable character"
description: Why the frame's invisible filter is the Unicode default-ignorable property together with Cf, Zl and Zp, and how the one table is proved.
resource: /docs/adr/0084-the-frame-drops-every-default-ignorable-character.md
tags: [larry, adr, tui, security, unicode]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#369 sims and marker test on Oracle Linux, clippy, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#369, /INVARIANTS.md, https://www.unicode.org/Public/16.0.0/ucd/DerivedCoreProperties.txt, https://www.unicode.org/Public/16.0.0/ucd/extracted/DerivedGeneralCategory.txt, https://cve.mitre.org/cgi-bin/cvename.cgi?name=CVE-2021-42574]
---

# 0084: the frame drops every default-ignorable character

Status: accepted, 2026-11-21 (larry#369). It widens the filter of ADR 0045's frame and keeps
its marker rule.

## Context

An approved command is shown with every character that does not draw filtered out or named
(larry#337). The filter held general category `Cf` and the `Zl` and `Zp` separators, so it missed
the characters Unicode 16.0.0 marks `Default_Ignorable_Code_Point` that are not `Cf`:

- U+034F COMBINING GRAPHEME JOINER (`Mn`).
- U+115F, U+1160, U+3164 and U+FFA0, the Hangul fillers (`Lo`). They draw as blank space and are
  used in "invisible variable" homoglyph attacks.
- U+FE00..U+FE0F and U+E0100..U+E01EF, the variation selectors (`Mn`).
- U+17B4..U+17B5 and U+180B..U+180D, U+180F, the Khmer and Mongolian variation selectors (`Mn`).

A command shown with one of them reads differently from the command that runs
(CVE-2021-42574). The property also lists reserved ranges (`Cn`) that a later Unicode version
can assign.

## Decision

- `style.rs` holds one `INVISIBLE` table: the union of Unicode 16.0.0
  `Default_Ignorable_Code_Point` (`DerivedCoreProperties.txt`), general category `Cf`, and the
  `Zl` and `Zp` separators, taken from `extracted/DerivedGeneralCategory.txt`.
- The union is merged into 25 sorted, disjoint, non-adjacent inclusive ranges. Every range is
  `(char, char)`, so no range can hold a surrogate.
- `is_invisible` is a binary search over that table. Sortedness is a precondition and a test
  proves it.
- `sanitize` drops every such character from a frame. `sanitize_shown` shows it as `⟨U+XXXX⟩` in
  an approved command, so the command on screen is the command that runs.
- The union keeps every `Cf` the property excludes from its own list, such as U+FFF9..U+FFFB and
  U+13430..U+1343F, because the issue names `Cf` as well as the property.

## Consequences

- `sim_tui_hostile_output_cannot_write_escapes` carries one character from each group the issue
  names and asserts none reaches the wire. It failed on the old table at `seed 0` with
  `'\u{34f}' reached the wire`.
- `sim_the_invisible_table_is_sorted_disjoint_and_complete` proves the table sorted, disjoint and
  complete, every character the issue names inside a range, and U+0041 and U+AC00 outside.
- MEASURED frame path, debug binary, 120x32 PTY: input p99 1.4 ms before and 1.4 ms after; first
  paint 28.1 ms before, 39.5 ms after; peak RSS 7800 KiB before, 7740 KiB after.
- No crate is added.
- A Unicode update regenerates the table from the two source files. Until then a character the
  newer standard marks default-ignorable is still missed, which is why the reversal below names
  the estate owner.

## Rejected

- Keep the `Cf`, `Zl`, `Zp` table and add the default-ignorable groups as separate ranges. A
  second list drifts from the property and misses whatever Unicode adds next.
- Use `unicode_width`'s zero-width set instead of a property table. U+115F and U+3164 are width
  2, so width is not the property, and the crate gives no default-ignorable answer.
- Drop a default-ignorable character from an approved command without a marker. The person then
  cannot see the difference between the shown command and the one that runs, which larry#337
  rejected.

Reversal: when a `lgwks_std` or `lgwks_deps` release ships the derived property table, this const
moves onto that one estate call and the hand-written table leaves.
