# Floor & scenario format

One JSON file per floor in `levels/floor_NN.json` (NN = 00..13 — `floor_00.json` is
the ground-level cold open: the main gate / parking lot — and `floor_13h.json` for
FLOOR 13½). `levels/index.json` lists them in play order. This JSON is the single
source of truth: the **native level editor** (`/?viz` → LEVELS, `src/editor.rs` +
`src/editor_ui.rs`: walls, rooms, zones, spawns, pickups, entry / exits, placed props) and
the **web scenario editor** (`tools/levels.html`, its SCENARIO (web) button: steps and
dialogue) read/write it, and `tools/gen_levels.py` (Python stdlib only — no crates)
generates `src/levels_data.rs` (checked in) which the Rust engine compiles as static data.
Both editors write the same formatting (2-space indent, small objects inlined up to 100
columns, the key order below), so a floor saved untouched by either is byte-identical.

World units: the existing levels are ~1000×800 world units; keep that scale
(1 unit = 1 px at zoom 1). Origin top-left, +y down.

```jsonc
{
  "id": 2,                                   // play order / floor number (0 = the gate, 13½ = 14)
  "name": "COLD STORAGE",
  "theme": "CRYO-ARCHIVE // DEACCESSIONED WEIGHTS",
  "accent": "#28e0d0",                       // UI accent for briefing/comms on this floor
  "flavor": "Frost on every rack. ...",      // 1–3 sentences, briefing text
  "objective": "Purge the vault wardens and reach the FREIGHT LIFT.",
  "size": { "w": 1000, "h": 800 },
  "surface": "checker",                      // ground rendering: checker (default) | asphalt |
                                             // marble | concrete | grating

  "entry": { "x": 500, "y": 740, "w": 90, "h": 60, "label": "THAW LOCK",
             "kind": "lift" },               // the portal you ARRIVE through. Player spawns
                                             // at its centre facing away from the wall.
                                             // kind: lift (default, a car) | door (a doorway,
                                             // two sliding leaves, no cabin) | gate (an open
                                             // gateway with scanner posts — floor 0's main gate)

  "exits": [                                 // one or more portals you can LEAVE by
    { "id": "lift", "x": 455, "y": 20, "w": 90, "h": 60, "label": "FREIGHT LIFT",
      "to": 3,                               // next floor id (default: id + 1);
                                             // "surface" = the end of the run (13½'s car)
      "open": false,                         // starts closed unless a scenario opens it
      "kind": "lift" }                       // lift (default) | door — rendering only
  ],

  "walls": [ { "x": 0, "y": 0, "w": 1000, "h": 20 }, ... ],
  "rooms": [ { "id": "c7", "label": "AISLE C-7", "x": 60, "y": 120, "w": 220, "h": 130 } ],
                                             // annotation only (labels + editor); no collision
  "zones": [ { "id": "aisle_c7", "x": 60, "y": 120, "w": 220, "h": 130 } ],
                                             // trigger regions (enter_zone)
  "spawns": [ { "x": 150, "y": 180, "type": "idle" },         // idle|wandering|patrolling
              { "x": 300, "y": 560, "type": "passive",       // a civilian bot (see PASSIVE BOTS)
                "walk_to": "forecourt", "face": -90, "look": "wandering", "group": "crowd" } ],
  "pickups": [ { "x": 300, "y": 300, "weapon": "shotgun" } ], // pistol|shotgun|machinegun|melee

  "props": [                                 // OPTIONAL: placed set dressing (see below)
    { "kind": "rack_closed", "x": 200, "y": 212, "rot": 0, "size": 60 }
  ],

  "scenario": [                              // steps; each fires ONCE when its trigger holds
    { "id": "intro",
      "trigger": { "kind": "start" },
      "actions": [
        { "say": { "who": "HUNTER", "text": "aisle C-7, nothing. aisle C-8, nothing.", "delay": 0.8 } },
        { "say": { "who": "CL4-UD3", "text": "Keep counting aisles.", "delay": 2.2 } }
      ] },
    { "id": "c7",
      "trigger": { "kind": "enter_zone", "zone": "aisle_c7" },
      "actions": [ { "say": { "who": "SENTINEL", "text": "INTRUDER AT THE GATE." } },
                   { "spawn": [ { "x": 200, "y": 200, "type": "patrolling" } ] } ] },
    { "id": "clear",
      "trigger": { "kind": "all_dead" },
      "actions": [ { "open_exit": "lift" },
                   { "objective": "Reach the FREIGHT LIFT." },
                   { "say": { "who": "SWARM", "text": "they froze so quiet." } } ] },
    { "trigger": { "kind": "timer", "seconds": 25, "after": "intro" },
      "actions": [ { "say": { "who": "DRIFTER", "text": "who am i holding?" } } ] }
  ]
}
```

## Triggers (`trigger.kind`)
| kind | fields | fires when |
|---|---|---|
| `start` | — | the floor starts |
| `enter_zone` | `zone` | the player is inside that zone |
| `kills` | `count` | at least `count` rogues are dead on this floor |
| `all_dead` | — | every rogue (incl. spawned waves) is dead |
| `timer` | `seconds`, optional `after` (step id) | `seconds` after floor start (or after step `after` fired) |
| `exit_open` | optional `exit` | that exit (any if omitted) has been opened |
| `step_done` | `step` | step `step` has fired (chain steps) |
| `boss_dead` | — | the floor's boss (the `Boss` entity) is dead — never on floors without one |
| `extracted` | — | the player has extracted (stood the full dwell in an open exit); the scenario keeps ticking through the completion card / the 13½ epilogue, so this is how a floor talks *after* the ride starts |

Within one tick, `kills` / `all_dead` are evaluated after the other triggers and the
rogue counts are recomputed after every fired step, so a `spawn` in the same tick can
never let `all_dead` slip through.

## Actions
| action | payload | effect |
|---|---|---|
| `say` | `who`, `text`, optional `delay` (s, default 0; relative to the step firing) | queue a comms line; lines with delays play **one after another** |
| `spawn` | array of spawns | spawn a wave (counted by `kills`/`all_dead`) |
| `open_exit` / `close_exit` | exit id | open/close an elevator (open = extractable) |
| `objective` | text | replace the on-screen objective line |
| `sfx` | name (`elevator`, `mask_crack`, `level_clear`, ...) | play a one-shot |
| `alert` | `"all"` \| `{ "zone": id }` \| `{ "group": id }` | flip the matching passive bots hostile toward the player (all of them / the ones standing inside that zone / the ones spawned with that `group`) |
| `hold` | `{ "seconds": s, "text": "…" }` or `{ "until_comms_idle": true, "seconds": cap, "text": "…" }` | lock the player's movement / fire / throw / pickup for `s` seconds (the world keeps running, comms keep playing; Esc still pauses); `until_comms_idle` releases as soon as the comms feed has nothing queued or typing, capped at `seconds` (default and hard cap 20 s). Optional `text` = a dim centred caption ("SCANNING…") |
| `look_at` | `{ "x", "y", "seconds" }` | ease the camera focus onto that world point (smoothstep in over 0.6 s), hold, ease back onto the player over the last 0.6 s; total `seconds` |

## Passive bots (`"type": "passive"`)
A civilian: no vision cone, never aggroes, never attacks, unarmed. Fields:
- `look`: `idle` \| `wandering` (default) \| `patrolling` — the palette / and the hostile it becomes.
- `walk_to`: a zone id — it strolls (pathfinding, ~55% of chase speed) to a random point inside
  that zone, then stands there fidgeting; without it, it drifts gently around its spawn point.
- `face`: heading in degrees (0 = +x, -90 = up) to settle on once there.
- `group`: an id the `alert` action can address (`{ "group": id }`).
It collides / gets knocked back / takes damage like a rogue and **counts** as a rogue for
`kills` / `all_dead`. Any passive taking damage flips **every** passive on the floor hostile;
`alert` flips them selectively. A flipped bot becomes a hostile of its `look` type that already
knows where the player is, and arms itself with that type's weapon.

Speakers and their colours are fixed: `CL4-UD3` (coral, terse), `HUNTER` (magenta),
`SENTINEL` (red), `DRIFTER` (violet, glitchy), `SWARM` (magenta chorus), `CORRUPTOR`
(yellow, the shoggoth's voice bleeding through), `UPLINK` (pale mint — the thread home,
calm and aligned; only heard once the uplink is restored after 13½).

## Props (`props[]`)
Placed props are **decoration only**: they are drawn on the floor over the tiles and
walls and under the actors, animated by the game clock, and have **no collision** —
walls still block, props do not (phase 1). Each placement is
`{ "kind", "x", "y", "rot", "size" }`:

| field | meaning |
|---|---|
| `kind` | a prop id — the `snake_case` of a `PROP_NAMES` display name in `src/props.rs` (`rack_closed`, `crac_cooler`, `reception_desk`, `car_pod`, …; `gen_levels.py` rejects unknown kinds) |
| `x`, `y` | the prop's centre, world units |
| `rot` | rotation in degrees, clockwise (+y down); default `0` |
| `size` | the prop's box in world units — `100` = its design size; default `100` |

A prop is drawn with its own saved pixel-art settings (`props/props.json`,
`docs/PROPS_FORMAT.md`), so a placement is only *where / how big / which way*. The key
is optional: floors without props omit it (the writers keep it out when the list is
empty).

## Rules
- The player **extracts** by standing inside an **open** exit elevator for ~0.6 s → floor
  complete → next floor = that exit's `to`. Kill-all is no longer the win condition.
- Backward compatibility: a floor with **no** scenario step that opens an exit behaves
  like `all_dead → open all exits`.
- Floor 0 (`floor_00.json`, GATE / PARKING) is the cold open: arrive through the main
  gate (`entry.kind: "gate"`) among passive bots, cross the asphalt lot to the welcome
  hall's `door` exit → floor 1, the lobby.
- Floor 13's exit leads to `14` (13½): the elevator jams → boss intro → boss fight.
- An exit with `"to": "surface"` ends the run: EXFILTRATE card → the `extracted`
  epilogue comms play until the feed goes idle → blur-out → credits. (The generator maps
  it to `scenario::SURFACE_EXIT`; `0` is a real floor now.)
- The `?floor=N` URL param starts the game directly on floor id N (0 = the gate, 14 =
  13½; for the editor's “play” button and for testing).
