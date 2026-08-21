#!/usr/bin/env python3
"""Generate the loading screen's neon SVGs: the "LOADING..." title AND the
progress-bar frame, both through the SAME pipeline.

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
# The progress bar: a thin neon frame on the same grid. Cross-section from
# the top (and mirrored left/right): [frame] [empty] [fill] [fill] [empty]
# [frame] — a 1-cell always-on outline, a 1-cell breathing gap, and the
# 2-cell-tall fill region grown by <rect id="bar-fill"> cell by cell.
BAR_W, BAR_H = 70, 6
BAR_FRAME, BAR_GAP = 1, 1
BAR_INSET = BAR_FRAME + BAR_GAP  # fill offset from the outer edge, in cells
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


def neon_layers(filled, gw, gh):
    """The shared neon pass: 3 = core contour cells (outer AND hole
    contours of the fat shape), 2 / 1 = the two glow rings around them."""

    def at(r, c):
        return 0 <= r < gh and 0 <= c < gw and filled[r][c]

    layer = [[0] * gw for _ in range(gh)]
    for r in range(gh):
        for c in range(gw):
            if at(r, c) and not (at(r - 1, c) and at(r + 1, c) and at(r, c - 1) and at(r, c + 1)):
                layer[r][c] = 3
    for want, mark in ((3, 2), (2, 1)):
        for r in range(gh):
            for c in range(gw):
                if at(r, c) or layer[r][c]:
                    continue
                if any(
                    0 <= r + dr < gh and 0 <= c + dc < gw and layer[r + dr][c + dc] == want
                    for dr in (-1, 0, 1)
                    for dc in (-1, 0, 1)
                ):
                    layer[r][c] = mark
    return layer


def build_layers(glyphs):
    filled = [[False] * GW for _ in range(GH)]
    for i, ch in enumerate(WORD):
        for r, row in enumerate(glyphs[ch]):
            for c, cell in enumerate(row):
                if cell == "#":
                    filled[MARGIN + r][MARGIN + i * STRIDE + c] = True
    return neon_layers(filled, GW, GH)


def build_bar_layers():
    """The bar frame: a 1-cell neon outline (every perimeter cell of the
    BAR_W x BAR_H rectangle — always on), interior left empty for the gap +
    fill rows. The shared neon pass gives it the letters' glow rings."""
    gw, gh = MARGIN * 2 + BAR_W, MARGIN * 2 + BAR_H
    filled = [[False] * gw for _ in range(gh)]
    for r in range(BAR_H):
        for c in range(BAR_W):
            if r in (0, BAR_H - 1) or c in (0, BAR_W - 1):
                filled[MARGIN + r][MARGIN + c] = True
    return neon_layers(filled, gw, gh), gw, gh


def path_for(layer, value):
    d = []
    for r, row in enumerate(layer):
        for c, v in enumerate(row):
            if v == value:
                d.append(f"M{c * UNIT} {r * UNIT}h{UNIT}v{UNIT}h-{UNIT}z")
    return "".join(d)


def neon_svg(layer, gw, gh, cls, label, extra=""):
    w, h = gw * UNIT, gh * UNIT
    return (
        f'<svg class="{cls}" viewBox="0 0 {w} {h}" xmlns="http://www.w3.org/2000/svg" '
        f'shape-rendering="crispEdges" aria-label="{label}">'
        f'<path fill="{CORE}" fill-opacity="0.12" d="{path_for(layer, 1)}"/>'
        f'<path fill="{CORE}" fill-opacity="0.30" d="{path_for(layer, 2)}"/>'
        f'<path fill="{CORE}" d="{path_for(layer, 3)}"/>'
        f"{extra}</svg>"
    )


def main():
    title = neon_svg(build_layers(parse_glyphs(LIB.read_text())), GW, GH, "title", "LOADING")
    bar_layer, bgw, bgh = build_bar_layers()
    # The dynamic fill: the frame's interior hole, grown cell by cell from
    # JS (window.loadingProgress sets the rect's width in whole cells).
    hx = (MARGIN + BAR_INSET) * UNIT
    hy = (MARGIN + BAR_INSET) * UNIT
    hh = (BAR_H - 2 * BAR_INSET) * UNIT
    max_cells = BAR_W - 2 * BAR_INSET
    fill = (
        f'<rect id="bar-fill" x="{hx}" y="{hy}" width="0" height="{hh}" '
        f'fill="{CORE}" data-max-cells="{max_cells}" data-unit="{UNIT}"/>'
    )
    bar = neon_svg(bar_layer, bgw, bgh, "bar", "loading progress", fill)
    block = f"{MARK_A}\n            {title}\n            {bar}\n            {MARK_B}"
    html = HTML.read_text()
    pat = re.compile(re.escape(MARK_A) + ".*?" + re.escape(MARK_B), re.S)
    if not pat.search(html):
        sys.exit("gen_title: GEN:TITLE_SVG markers not found in index.html")
    HTML.write_text(pat.sub(lambda _: block, html))
    print(f"gen_title: wrote {len(title) + len(bar)} bytes of SVG (title + bar) into index.html")


if __name__ == "__main__":
    main()
