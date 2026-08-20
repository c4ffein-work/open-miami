#!/usr/bin/env python3
"""Generate the loading-screen "LOADING..." SVG in the neon title style.

Single source of truth: the 12x16 title glyphs live in src/lib.rs
(`title_glyph`). This script parses them out, runs the SAME boundary + glow
pass the menu title uses (only the contour cells of the fat letterforms,
plus two rings of pixel glow, the same pink), lays out one flat line of
"LOADING..." (no rotation), and writes it as an inline SVG between the
GEN:TITLE_SVG markers of index.html — so it shows instantly, before any
JS/wasm loads.

Python stdlib only. Usage: python3 tools/gen_title.py   (or: make gen-title)
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LIB = ROOT / "src" / "lib.rs"
HTML = ROOT / "index.html"

UNIT = 8  # svg units per art pixel (matches the in-game cell)
WORD = "LOADING..."
STRIDE = 14  # glyph width 12 + gap 2
MARGIN = 2  # cells of breathing room (the glow needs 2)
GW = MARGIN * 2 + 12 + (len(WORD) - 1) * STRIDE
GH = MARGIN * 2 + 16
CORE = "#FF3399"  # Color::new(1.0, 0.20, 0.60) — the in-game neon pink
MARK_A = "<!-- GEN:TITLE_SVG (tools/gen_title.py — do not hand-edit) -->"
MARK_B = "<!-- /GEN:TITLE_SVG -->"


def parse_glyphs(src):
    """{char: [16 row strings]} out of title_glyph's match arms."""
    body = re.search(r"fn title_glyph.*?\n(.*?)\n\s*}\n", src, re.S)
    if not body:
        sys.exit("gen_title: title_glyph not found in src/lib.rs")
    glyphs = {}
    for ch, rows in re.findall(r"'(.)' => \[(.*?)\]", body.group(1), re.S):
        glyphs[ch] = re.findall(r'"([.#]{12})"', rows)
        if len(glyphs[ch]) != 16:
            sys.exit(f"gen_title: glyph {ch!r} has {len(glyphs[ch])} rows, want 16")
    return glyphs


def build_layers(glyphs):
    filled = [[False] * GW for _ in range(GH)]
    for i, ch in enumerate(WORD):
        for r, row in enumerate(glyphs[ch]):
            for c, cell in enumerate(row):
                if cell == "#":
                    filled[MARGIN + r][MARGIN + i * STRIDE + c] = True

    def at(r, c):
        return 0 <= r < GH and 0 <= c < GW and filled[r][c]

    layer = [[0] * GW for _ in range(GH)]  # 3 core, 2 glow, 1 faint
    for r in range(GH):
        for c in range(GW):
            if at(r, c) and not (at(r - 1, c) and at(r + 1, c) and at(r, c - 1) and at(r, c + 1)):
                layer[r][c] = 3
    for want, mark in ((3, 2), (2, 1)):
        for r in range(GH):
            for c in range(GW):
                if at(r, c) or layer[r][c]:
                    continue
                if any(
                    0 <= r + dr < GH and 0 <= c + dc < GW and layer[r + dr][c + dc] == want
                    for dr in (-1, 0, 1)
                    for dc in (-1, 0, 1)
                ):
                    layer[r][c] = mark
    return layer


def path_for(layer, value):
    d = []
    for r in range(GH):
        for c in range(GW):
            if layer[r][c] == value:
                d.append(f"M{c * UNIT} {r * UNIT}h{UNIT}v{UNIT}h-{UNIT}z")
    return "".join(d)


def main():
    layer = build_layers(parse_glyphs(LIB.read_text()))
    w, h = GW * UNIT, GH * UNIT
    svg = (
        f'<svg viewBox="0 0 {w} {h}" xmlns="http://www.w3.org/2000/svg" '
        f'shape-rendering="crispEdges" aria-label="LOADING">'
        f'<path fill="{CORE}" fill-opacity="0.12" d="{path_for(layer, 1)}"/>'
        f'<path fill="{CORE}" fill-opacity="0.30" d="{path_for(layer, 2)}"/>'
        f'<path fill="{CORE}" d="{path_for(layer, 3)}"/>'
        f"</svg>"
    )
    block = f"{MARK_A}\n            {svg}\n            {MARK_B}"
    html = HTML.read_text()
    pat = re.compile(re.escape(MARK_A) + ".*?" + re.escape(MARK_B), re.S)
    if not pat.search(html):
        sys.exit("gen_title: GEN:TITLE_SVG markers not found in index.html")
    HTML.write_text(pat.sub(lambda _: block, html))
    print(f"gen_title: wrote {len(svg)} bytes of SVG into index.html")


if __name__ == "__main__":
    main()
