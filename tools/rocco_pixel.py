#!/usr/bin/env python3
"""Rocco blocky pixel mascot — single source of truth.

Canonical asset: lgwks-frontend/static/assets/logo/rocco-flat.webp (Director,
2026-09-08: "flatter Rocco in the navigation center"). Identity features that
must survive translation (recovered via ChatGPT memory, 2026-09-18): paired
ears with dark interiors, contrasting bandit mask, LARGE eyes with white
catchlights, central dark nose, open smile with pink-red tongue.

Grid: 24x24 (Gemini craft rules: 16x16 is a trap for raccoons, 32x32 loses the
chunky 8-bit look). Palette: 4 colors + transparent (ANSI16-safe so the AppCUI
build compiles without TRUE_COLORS and renders on every terminal):
  d = dark   #2A2620 -> AppCUI Black  (outline, mask, nose, mouth)
  g = gray   #7A7373 -> AppCUI Gray   (fur)
  c = cream  #F4EFE2 -> AppCUI White  (muzzle, brows, catchlights)
  b = brown  eye body  -> AppCUI Olive (large brown eyes)
  t = tongue #C9584A -> AppCUI Red    (open smile)

Outputs:
  --preview   terminal half-block render (truecolor ANSI, for eyeballing)
  --tui       Rust const: (char, Color) rows for native.rs half-block hero
  --pet DIR   write Codex pet package (pet.json + spritesheet.webp, 8x9 atlas)
  --svg PATH  write base SVG (24-unit grid on a Clawd-style canvas)
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# The grid: LEFT HALF (12 columns, x0..11) mirrored to x12..23.
# '.' = transparent. d=dark outline/mask, g=gray fur, c=cream, b=brown eye,
# t=tongue. Center seam falls between x11 | x12.
# Structure: dark-interior ears with gray rim, gray forehead, two bandit-mask
# patches (NOT one band) wrapping LARGE brown eyes with cream catchlights,
# cream muzzle, dark nose, cream gap, open dark smile with red tongue.
LEFT_OPEN = [
    "............",
    "...dd.......",
    "..dddd......",
    "..dddd......",
    "..dggd......",
    "..dggd......",
    "..dgg.......",
    "..dgggg.....",
    "..dggggggggg",
    ".dgggggggggg",
    ".dggdddddddg",
    ".dggddbbbbdg",
    ".dggddbcbbdg",
    ".dggddbcbbdg",
    ".dggddbbbbdg",
    ".dggdddddddg",
    ".dggggggcccc",
    ".dgggggccccc",
    ".dggggccccdd",
    ".dggggccccdd",
    ".dgggggccccc",
    ".dgggggcdddd",
    ".dgggggcdttt",
    "...ddddddddd",
]

LEFT_BLINK = [row for row in LEFT_OPEN]
LEFT_BLINK[11] = ".dggdddddddg"
LEFT_BLINK[12] = ".dggdddddddg"
LEFT_BLINK[13] = ".dggdddddddg"
LEFT_BLINK[14] = ".dggdddddddg"


def mirror(left: list[str]) -> list[str]:
    rows = []
    for row in left:
        assert len(row) == 12, f"bad row {row!r}"
        rows.append((row + row[::-1]).replace("#", "d"))
    return rows


GRID_OPEN = mirror(LEFT_OPEN)
GRID_BLINK = mirror(LEFT_BLINK)

assert len(GRID_OPEN) == 24 and all(len(r) == 24 for r in GRID_OPEN)
assert len(GRID_BLINK) == 24 and all(len(r) == 24 for r in GRID_BLINK)

PALETTE = {
    "d": (42, 38, 32),      # #2A2620
    "g": (122, 115, 115),   # #7A7373
    "c": (244, 239, 226),   # #F4EFE2
    "b": (128, 96, 60),     # warm brown eye
    "t": (201, 88, 74),     # #C9584A
}

ANSI = {
    "d": "Black",
    "g": "Gray",
    "c": "White",
    "b": "Olive",
    "t": "Red",
}


def half_blocks(grid: list[str]) -> list[tuple[str, str]]:
    """Pair rows into half-block cells: char + fg(top)+bg(bottom) spec."""
    out = []
    for y in range(0, 24, 2):
        cells = []
        for x in range(24):
            top, bot = grid[y][x], grid[y + 1][x]
            if top == "." and bot == ".":
                cells.append(("space", ".", "."))
            elif bot == ".":
                cells.append(("upper", top, "."))
            elif top == ".":
                cells.append(("lower", bot, "."))
            else:
                cells.append(("both", top, bot))
        out.append(cells)  # type: ignore[list-item]
    return out  # type: ignore[return-value]


def render_preview(grid: list[str]) -> str:
    """Truecolor ANSI half-block preview for the local terminal."""
    lines = []
    for y in range(0, 24, 2):
        line = []
        for x in range(24):
            top, bot = grid[y][x], grid[y + 1][x]
            tfg = f"{PALETTE[top][0]};{PALETTE[top][1]};{PALETTE[top][2]}" if top != "." else None
            bfg = f"{PALETTE[bot][0]};{PALETTE[bot][1]};{PALETTE[bot][2]}" if bot != "." else None
            if tfg and bfg:
                line.append(f"\x1b[48;2;{tfg};38;2;{bfg}m▀\x1b[0m")
            elif tfg:
                line.append(f"\x1b[38;2;{tfg}m▀\x1b[0m")
            elif bfg:
                line.append(f"\x1b[38;2;{bfg}m▄\x1b[0m")
            else:
                line.append(" ")
        lines.append("".join(line))
    return "\n".join(lines)


def rust_tui() -> str:
    """Emit the MASCOT rows as Rust (char, Color, Color) tuples."""
    parts = []
    for cells in half_blocks(GRID_OPEN):
        row = []
        for kind, fg, bg in cells:
            fgc = "Color::Transparent" if fg == "." else f"Color::{ANSI[fg]}"
            bgc = "Color::Transparent" if bg == "." else f"Color::{ANSI[bg]}"
            ch = {"space": "' '", "upper": "'▀'", "lower": "'▄'", "both": "'▀'"}[kind]
            row.append(f"({ch}, {fgc}, {bgc})")
        parts.append("        [" + ", ".join(row) + "],")
    body = "\n".join(parts)
    return f"pub const MASCOT: [[(char, Color, Color); 24]; 12] = [\n{body}\n];\n"


def pet_frames() -> list[list[list[int]]]:
    """8 idle-block frames: 5 open, 2 blink, 1 open (simple loop)."""
    grids = [GRID_OPEN] * 5 + [GRID_BLINK] * 2 + [GRID_OPEN]
    frames = []
    for grid in grids:
        canvas = [[0, 0, 0, 0] for _ in range(192 * 208)]
        ox, oy = 0, 8  # 24*8 = 192 wide; centered vertically-ish
        for gy in range(24):
            for gx in range(24):
                ch = grid[gy][gx]
                if ch == ".":
                    continue
                r, g, b = PALETTE[ch]
                for py in range(8):
                    for px in range(8):
                        idx = (oy + gy * 8 + py) * 192 + (ox + gx * 8 + px)
                        canvas[idx] = [r, g, b, 255]
        frames.append(canvas)
    return frames


def write_pet(out_dir: Path) -> None:
    """Write the Codex pet package: pet.json + spritesheet.webp (8x9 atlas)."""
    from PIL import Image

    out_dir.mkdir(parents=True, exist_ok=True)
    # 8x9 atlas, one row of 8 idle frames; remaining rows transparent.
    sheet = Image.new("RGBA", (1536, 1872), (0, 0, 0, 0))
    for i, frame in enumerate(pet_frames()):
        img = Image.new("RGBA", (192, 208), (0, 0, 0, 0))
        img.putdata([tuple(px) for px in frame])  # type: ignore[arg-type]
        sheet.paste(img, (i * 192, 0))
    sheet.save(out_dir / "spritesheet.webp", "WEBP", lossless=True)
    manifest = {
        "id": "rocco",
        "displayName": "Rocco",
        "description": (
            "Logical Works' Toronto raccoon. Warm, intelligent, calm, concrete. "
            "Blocky 24x24 translation of the canonical rocco-flat.webp."
        ),
        "spritesheetPath": "spritesheet.webp",
    }
    (out_dir / "pet.json").write_text(json.dumps(manifest, indent=2) + "\n")


def write_svg(path: Path) -> None:
    """Base SVG: one rect per pixel run, 1 unit per pixel, 2px padding."""
    runs = []
    for y, row in enumerate(GRID_OPEN):
        x = 0
        while x < 24:
            ch = row[x]
            if ch == ".":
                x += 1
                continue
            x0 = x
            while x < 24 and row[x] == ch:
                x += 1
            r, g, b = PALETTE[ch]
            runs.append(
                f'<rect x="{x0 + 1}" y="{y + 1}" width="{x - x0}" height="1" '
                f'fill="#{r:02X}{g:02X}{b:02X}"/>'
            )
    body = "\n  ".join(runs)
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 26 26" '
        'width="260" height="260" shape-rendering="crispEdges">\n'
        f"  {body}\n</svg>\n"
    )
    path.write_text(svg)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", action="store_true")
    ap.add_argument("--tui", action="store_true")
    ap.add_argument("--pet", type=Path)
    ap.add_argument("--svg", type=Path)
    args = ap.parse_args()
    if args.preview:
        print(render_preview(GRID_OPEN))
        print()
        print(render_preview(GRID_BLINK))
    if args.tui:
        sys.stdout.write(rust_tui())
    if args.pet:
        write_pet(args.pet)
        print(f"wrote {args.pet}/pet.json + spritesheet.webp")
    if args.svg:
        write_svg(args.svg)
        print(f"wrote {args.svg}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
