---
type: ADR
title: "ADR 0014: The pixel mascot is drawn from one Rust grid"
description: Why larry mascot replaces tools/rocco_pixel.py, and why it carries its own lossless WebP encoder.
resource: /docs/adr/0014-mascot-in-rust.md
tags: [larry, adr, mascot, python-ban]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: libwebp webpinfo and dwebp on larry mascot --pet output, mascot::sim}
status: accepted
stale_after: 2027-02-07
sources: [larry#53, larry#152, RFC 9649, RFC 1951, Director order 2026-10-13 (Python ban)]
---

# 0014: The pixel mascot is drawn from one Rust grid

Status: accepted, 2026-11-07 (larry#152, step 4 of larry#53)

## Context

`tools/rocco_pixel.py` held the 24x24 raccoon grid and printed four things from it: a terminal preview,
the Rust constant the native view painted, the pet package (`pet.json` plus a WebP spritesheet through
Pillow) and the base SVG. Python is banned in product repositories (2026-10-13), and the grid lived twice,
in the script and in `native::render`.

## Decision

- `mascot::OPEN` is the one grid. The native view, `larry mascot --svg`, `--preview` and `--pet` all draw
  from it. `--tui` is retired, because the constant it printed is now the source.
- `larry mascot --check` refuses a `docs/larry-mascot.svg` that differs from the render. It runs in both
  governance jobs and in `cargo local-ci`.
- No estate crate encodes WebP, so `mascot::webp` writes lossless VP8L (RFC 9649). It is the smallest
  subset that compresses flat 8x8 blocks:
  - no transforms and no colour cache;
  - literals plus copies from the previous pixel and from the pixel above;
  - complete canonical prefix codes, which libwebp requires.

  The 1536x1872 sheet is 2,158 bytes.
- Deviations from the script:
  - The manifest says `larry` and `Larry` (ADR 0005).
  - The preview paints the top pixel in the upper half, as the native view does. The script drew it
    inverted.
- The tracker's temp-file-and-rename write moved to `replace_file` in the library, so the tracker page
  and every mascot file is replaced whole or not at all.

## Consequences

- `mascot::sim` proves the encoder against a test-side reader written from the RFC: 64 seeded images
  (one to 300 colours), edge sizes, and the sheet, whose pixels match the BLAKE3 digest of libwebp's
  `dwebp` decode.
- A grid change updates the committed SVG through `larry mascot --svg docs/larry-mascot.svg`, or CI fails.
