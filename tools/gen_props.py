#!/usr/bin/env python3
"""Generate `src/props_data.rs` from `props/props.json`.

Python standard library only. The JSON holds the SAVED per-prop pixel-art
settings of the datacenter prop library (`src/props.rs`): the art-pixel size
`px` of each prop and, per layer, whether the layer is pixelated BEFORE or
AFTER its rotation. The `?viz` PROPS page edits and saves the file
(PUT /props/props.json through serve.py); this script compiles it into a
static Rust table — the drawing code itself stays in props.rs.

Usage:
    python3 tools/gen_props.py            # write src/props_data.rs
    python3 tools/gen_props.py --check    # validate + verify the checked-in
                                          # file is up to date (exit 1 if not)

The JSON contract is documented in docs/PROPS_FORMAT.md:

    { "props": [ { "kind": "rack_closed", "px": 4,
                   "layers": [ {"name": "lid", "pixel": "before"}, ... ] }, ... ] }

  * `kind` = snake_case of the prop's display name in `PROP_NAMES`
    (`"RACK / CLOSED"` -> `rack_closed`), read from src/props.rs so the
    generated array is in library order; unknown kinds are an error, missing
    ones get the defaults (px 1, no layer overrides).
  * `px`: integer 1..10 (1 = no pixelation), design units of the prop's
    100x100 box.
  * `layers[].name`: a layer of that prop (validated by the Rust unit test
    `props::tests::generated_settings_match_the_library`, which knows the
    layer lists); `pixel`: "before" | "after". Layers not listed keep the
    default of their `LayerDef`.
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
JSON_PATH = os.path.join(ROOT, "props", "props.json")
PROPS_RS = os.path.join(ROOT, "src", "props.rs")
OUT_PATH = os.path.join(ROOT, "src", "props_data.rs")

MAX_PX = 10
MAX_LAYERS = 8
PIXEL_MODES = {"before": "PixelMode::Before", "after": "PixelMode::After"}


class Invalid(Exception):
    pass


def kind_id(display_name):
    """Mirror of `props::prop_kind_id`: lower-case, runs of non-alphanumerics
    collapsed to one `_`, trimmed."""
    out = []
    for ch in display_name:
        if ch.isascii() and ch.isalnum():
            out.append(ch.lower())
        elif out and out[-1] != "_":
            out.append("_")
    return "".join(out).rstrip("_")


def load_prop_names():
    """The library order: `pub const PROP_NAMES: [&str; N] = [ "..", .. ];`
    in src/props.rs."""
    with open(PROPS_RS, encoding="utf-8") as fh:
        src = fh.read()
    m = re.search(r"pub const PROP_NAMES: \[&str; (\d+)\] = \[(.*?)\];", src, re.S)
    if not m:
        raise Invalid("cannot find PROP_NAMES in src/props.rs")
    names = re.findall(r'"((?:[^"\\]|\\.)*)"', m.group(2))
    if len(names) != int(m.group(1)):
        raise Invalid("PROP_NAMES length mismatch in src/props.rs")
    return names


def load_settings():
    with open(JSON_PATH, encoding="utf-8") as fh:
        doc = json.load(fh)
    return validate(doc)


def validate(doc):
    """Validate the document; return {kind_id: (px, [(name, mode), ...])}."""
    if not isinstance(doc, dict) or not isinstance(doc.get("props"), list):
        raise Invalid('top level must be {"props": [...]}')
    known = [kind_id(n) for n in load_prop_names()]
    seen = {}
    for i, p in enumerate(doc["props"]):
        if not isinstance(p, dict):
            raise Invalid(f"props[{i}]: not an object")
        kind = p.get("kind")
        if kind not in known:
            raise Invalid(f"props[{i}]: unknown kind {kind!r} (known: {', '.join(known)})")
        if kind in seen:
            raise Invalid(f"props[{i}]: duplicate kind {kind!r}")
        px = p.get("px", 1)
        if not isinstance(px, int) or isinstance(px, bool) or not 1 <= px <= MAX_PX:
            raise Invalid(f"{kind}: px must be an integer 1..{MAX_PX}, got {px!r}")
        layers = p.get("layers", [])
        if not isinstance(layers, list):
            raise Invalid(f"{kind}: layers must be a list")
        if len(layers) > MAX_LAYERS:
            raise Invalid(f"{kind}: more than {MAX_LAYERS} layers")
        out_layers = []
        names = set()
        for j, l in enumerate(layers):
            if not isinstance(l, dict):
                raise Invalid(f"{kind}: layers[{j}] is not an object")
            name = l.get("name")
            if not isinstance(name, str) or not name:
                raise Invalid(f"{kind}: layers[{j}] needs a non-empty name")
            if name in names:
                raise Invalid(f"{kind}: duplicate layer {name!r}")
            names.add(name)
            mode = l.get("pixel", "before")
            if mode not in PIXEL_MODES:
                raise Invalid(f"{kind}/{name}: pixel must be 'before' or 'after', got {mode!r}")
            out_layers.append((name, mode))
        seen[kind] = (px, out_layers)
    return seen


def rstr(s):
    out = s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "").replace("\t", "\\t")
    return f'"{out}"'


def generate(settings):
    names = load_prop_names()
    out = [
        "// @generated by tools/gen_props.py from props/props.json — DO NOT EDIT.",
        "// Re-run `make gen-props` after editing the JSON (the ?viz PROPS page saves it).",
        "//",
        "// The saved pixel-art settings of the datacenter prop library: per prop the",
        "// art-pixel size, per layer whether it is pixelated before or after its",
        "// rotation. See docs/PROPS_FORMAT.md; the layers themselves live in",
        "// src/props.rs (PROP_LAYERS).",
        "#![allow(clippy::all)]",
        "",
        "use crate::props::{LayerSetting, PixelMode, PropSettings, PROP_COUNT};",
        "",
        "/// One entry per prop, in library order (index = prop id).",
        "pub static PROP_SETTINGS: [PropSettings; PROP_COUNT] = [",
    ]
    for i, display in enumerate(names):
        kind = kind_id(display)
        px, layers = settings.get(kind, (1, []))
        out.append(f"    // {i} {display}")
        out.append("    PropSettings {")
        out.append(f"        kind: {rstr(kind)},")
        out.append(f"        px: {px},")
        if layers:
            out.append("        layers: &[")
            for name, mode in layers:
                out.append(f"            LayerSetting {{ name: {rstr(name)}, pixel: {PIXEL_MODES[mode]} }},")
            out.append("        ],")
        else:
            out.append("        layers: &[],")
        out.append("    },")
    out.append("];")
    out.append("")
    return "\n".join(out)


def main(argv):
    check = "--check" in argv
    try:
        settings = load_settings()
        text = generate(settings)
    except (Invalid, KeyError, OSError, ValueError, json.JSONDecodeError) as e:
        print(f"gen_props: error: {e}", file=sys.stderr)
        return 1
    if check:
        try:
            with open(OUT_PATH, encoding="utf-8") as fh:
                current = fh.read()
        except OSError:
            current = None
        if current != text:
            print(f"gen_props: {os.path.relpath(OUT_PATH, ROOT)} is out of date — run `make gen-props`",
                  file=sys.stderr)
            return 1
        print(f"gen_props: {len(settings)} props valid, {os.path.relpath(OUT_PATH, ROOT)} up to date")
        return 0
    with open(OUT_PATH, "w", encoding="utf-8") as fh:
        fh.write(text)
    print(f"gen_props: wrote {os.path.relpath(OUT_PATH, ROOT)} ({len(settings)} props)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
