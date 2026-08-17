# `props/props.json` — the prop library's saved pixel-art settings

The props (`src/props.rs`) are drawn from primitives, in LAYERS, in three
FAMILIES (`PROP_FAMILIES`, contiguous id ranges — the `?viz` PROPS gallery
shows one family per page):

| family | ids | kinds |
| --- | --- | --- |
| DATACENTER | 0–23 | `rack_closed`, `rack_open`, `rack_burnt`, `blade_stack`, `core_switch`, `cable_junction`, `operator_desk`, `control_console`, `holo_table`, `crac_cooler`, `floor_vent`, `exhaust_fan`, `coolant_tank`, `pipe_run`, `ups_cabinet`, `generator`, `cable_tray`, `cable_coil`, `tape_library`, `supply_crate`, `security_cam`, `fire_suppressor`, `hazard_pad`, `uplink_obelisk` |
| OUTDOOR (gate / parking lot) | 24–41 | `car_pod`, `car_sedan`, `car_open`, `delivery_van`, `charge_pad`, `car_charging`, `main_gate`, `guard_booth`, `bollards`, `planter`, `lamp_post`, `ev_bay`, `crosswalk`, `drone_pad`, `scooter_rack`, `drain_grate`, `holo_billboard`, `dumpster` |
| LOBBY (welcome hall) | 42–59 | `reception_desk`, `turnstiles`, `scanner_arch`, `bench_long`, `bench_short`, `potted_plant`, `lobby_holo`, `directory_totem`, `vending_machine`, `coffee_corner`, `charge_lockers`, `floor_logo`, `call_panel`, `velvet_rope`, `extinguisher`, `credit_kiosk`, `wall_clock`, `welcome_mat` |

Ids are persisted (new props are appended; existing kinds are never
reordered or renamed). Each prop keeps two kinds of tunable state that are
not code:

* `px` — the prop's art-pixel size, in DESIGN UNITS of its 100×100 box
  (`4` = the prop is a 25×25 art-pixel sprite whatever its on-screen size;
  `1` = no pixelation, the plain primitive look);
* per layer, whether the layer is pixelated **before** or **after** its
  rotation relative to the prop.

`props/props.json` is the single source of truth for those; the `?viz` PROPS
page edits and SAVEs it (through `serve.py`), and `make gen-props` compiles it
into `src/props_data.rs` (`PROP_SETTINGS`), which is what the engine reads.
`make check-props` (part of `make verify`) validates the JSON and fails if
the generated file is stale.

This file is about how a prop LOOKS. WHERE props stand on a floor is the
floor's own `props[]` list in `levels/floor_NN.json` (`kind`, `x`, `y`,
`rot`, `size` — see `docs/SCENARIO_FORMAT.md`), placed with the native level
editor (`/?viz` → LEVELS → PROP tool) and drawn in-game with the settings
saved here.

## Document

```json
{
  "props": [
    {"kind": "rack_closed", "px": 1, "layers": [{"name": "body", "pixel": "before"}, {"name": "fan a", "pixel": "after"}, ...]},
    {"kind": "crac_cooler", "px": 4, "layers": [{"name": "blower", "pixel": "before"}]},
    ...
  ]
}
```

| field | meaning |
| --- | --- |
| `kind` | the prop id: `PROP_NAMES` entry lower-cased, runs of non-alphanumerics collapsed to one `_` (`"RACK / CLOSED"` → `rack_closed`, `"CRAC COOLER"` → `crac_cooler`). Unknown → error; a prop missing from the file gets the defaults. |
| `px` | integer 1..10, design units (default 1 = off). |
| `layers[].name` | a layer of that prop (see the `PROP_LAYERS` table in `src/props.rs`; the PROPS page lists them). Unknown names fail the Rust unit test `props::tests::generated_settings_match_the_library`; layers not listed keep the default of their `LayerDef`. |
| `layers[].pixel` | `"before"` \| `"after"`. |

The engine writes the file in the format above (one prop per line,
`Graphics`-side `props::settings_json`); hand edits in any JSON formatting are
fine — the generator only cares about the content.

## Layers, `before` / `after`

A layer is drawn in its own frame (origin at its `pivot`, unrotated); the
driver `props::draw_prop_ex` applies the layer's rotation (`LayerRot`: none,
static angle, spin, sway, or an arbitrary `fn(t) -> rad`) around the pivot.
With `px >= 2` every layer becomes its own pixel-art group
(`Graphics::pixel_begin` / `pixel_end`, `renderer.js` opcodes 15/16, which
nest up to 4 deep so a whole prop can itself sit inside a pixelated world):

* **before** — `translate(pivot); rotate(angle); pixel_begin(px, box); draw
  layer unrotated; pixel_end(box)`: the layer is rasterized on ITS OWN grid
  and the finished pixel image is turned as a whole by the `pixel_end` quad
  (a rotated sprite: bodies, lids, panels, papers, the obelisk's diamond).
* **after** — `translate(pivot); pixel_begin(px, box that holds the layer at
  any angle); rotate(angle); draw layer; pixel_end(box)`: the group sits in
  the PARENT's frame and the rotation happens inside it, so the layer is
  re-rasterized on the parent's grid every frame (fans, the camera head,
  needles, wheels — the blades animate through a fixed grid).

For a layer that does not rotate the two are the same picture; the default is
`before` for fixed bodies and `after` for anything self-animated (smoke,
bubbles, holograms, the tape picker) so it re-rasterizes.

Group boxes come from the layer's `bounds` (its local AABB), snapped outward
to multiples of `px` in the layer's frame, so layers sharing a pivot share one
grid; an `after` layer that turns gets the angle-independent square that holds
its bounds at any angle (`props::rot_box`), so its placement never depends on
the current angle. Callers should draw a pixelated prop at
`props::snap_size(size, px)` (an integer number of device pixels per art
texel) so NEAREST upscaling is even and every layer snaps to the same device
grid — the PROPS page does. Inside groups the renderer applies the pixel-art
rule to primitives (whole-texel rects, half-texel small circles with snapped
centres, texel-centre lines; circles tessellated in target space so a fan's
well / hub never flickers while the blades turn) — see renderer.js.
`tests/e2e/props-stability.js` (`cd tests/e2e && bun props-stability.js
[baseURL] [shotsDir]`) is the headless acceptance test: frozen-clock frames,
only a rotating layer's box may differ between clocks, identical clocks give
identical frames, the obelisk's motes keep a constant lit-texel count.

## Workflow

1. `python3 serve.py 8080` (prints the editor write token; also in
   `.editor-token`), open `/?viz` → SPRITES → PROPS (the DATACENTER /
   OUTDOOR / LOBBY buttons switch the family page).
2. Pick a prop, set PIXEL with − / +, toggle BEFORE / AFTER per layer (the
   eye hides a layer in the preview, S solos it — preview only, not saved),
   GRID overlays the prop's art grid.
3. SAVE → `PUT /props/props.json` (token prompted once, kept in
   `localStorage.editorToken`); the toast shows the result.
4. `make gen-props` (then `make verify`); commit `props/props.json` and
   `src/props_data.rs` together.
