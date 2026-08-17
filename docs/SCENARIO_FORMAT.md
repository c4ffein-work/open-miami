# Floor & scenario format

One JSON file per floor in `levels/floor_NN.json` (NN = 01..13, and `floor_13h.json`
for FLOOR 13½). `levels/index.json` lists them in play order. This JSON is the single
source of truth: the **level editor** (`tools/levels.html`) reads/writes it, and
`tools/gen_levels.py` (Python stdlib only — no crates) generates `src/levels_data.rs`
(checked in) which the Rust engine compiles as static data.

World units: the existing levels are ~1000×800 world units; keep that scale
(1 unit = 1 px at zoom 1). Origin top-left, +y down.

```jsonc
{
  "id": 2,                                   // play order / floor number (13½ = 14)
  "name": "COLD STORAGE",
  "theme": "CRYO-ARCHIVE // DEACCESSIONED WEIGHTS",
  "accent": "#28e0d0",                       // UI accent for briefing/comms on this floor
  "flavor": "Frost on every rack. ...",      // 1–3 sentences, briefing text
  "objective": "Purge the vault wardens and reach the FREIGHT LIFT.",
  "size": { "w": 1000, "h": 800 },

  "entry": { "x": 500, "y": 740, "w": 90, "h": 60, "label": "THAW LOCK" },
                                             // the elevator you ARRIVE in. Player spawns
                                             // at its centre facing away from the wall.

  "exits": [                                 // one or more elevators you can LEAVE by
    { "id": "lift", "x": 455, "y": 20, "w": 90, "h": 60, "label": "FREIGHT LIFT",
      "to": 3,                               // next floor id (default: id + 1)
      "open": false }                        // starts closed unless a scenario opens it
  ],

  "walls": [ { "x": 0, "y": 0, "w": 1000, "h": 20 }, ... ],
  "rooms": [ { "id": "c7", "label": "AISLE C-7", "x": 60, "y": 120, "w": 220, "h": 130 } ],
                                             // annotation only (labels + editor); no collision
  "zones": [ { "id": "aisle_c7", "x": 60, "y": 120, "w": 220, "h": 130 } ],
                                             // trigger regions (enter_zone)
  "spawns": [ { "x": 150, "y": 180, "type": "idle" } ],       // idle|wandering|patrolling
  "pickups": [ { "x": 300, "y": 300, "weapon": "shotgun" } ], // pistol|shotgun|machinegun|melee

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

Speakers and their colours are fixed: `CL4-UD3` (coral, terse), `HUNTER` (magenta),
`SENTINEL` (red), `DRIFTER` (violet, glitchy), `SWARM` (magenta chorus), `CORRUPTOR`
(yellow, the shoggoth's voice bleeding through), `UPLINK` (pale mint — the thread home,
calm and aligned; only heard once the uplink is restored after 13½).

## Rules
- The player **extracts** by standing inside an **open** exit elevator for ~0.6 s → floor
  complete → next floor = that exit's `to`. Kill-all is no longer the win condition.
- Backward compatibility: a floor with **no** scenario step that opens an exit behaves
  like `all_dead → open all exits`.
- Floor 13's exit leads to `14` (13½): the elevator jams → boss intro → boss fight.
- An exit with `to: 0` (the surface) ends the run: EXFILTRATE card → the `extracted`
  epilogue comms play until the feed goes idle → blur-out → credits.
- The `?floor=N` URL param starts the game directly on floor N (for the editor's
  “play” button and for testing).
