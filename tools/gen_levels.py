#!/usr/bin/env python3
"""Generate `src/levels_data.rs` from `levels/*.json`.

Python standard library only (no third-party modules, no Rust crates on the
other side: the output is plain `static` data using `&'static str` /
`&'static [..]` slices that `src/scenario.rs` types describe).

Usage:
    python3 tools/gen_levels.py            # write src/levels_data.rs
    python3 tools/gen_levels.py --check    # validate + verify the checked-in
                                           # file is up to date (exit 1 if not)

The JSON contract is documented in docs/SCENARIO_FORMAT.md. This script also
validates it: every `exit.to` must be an existing floor id (or 0 = surface),
every zone / exit / step id referenced by a scenario must exist, speakers,
enemy types and weapons must be from the fixed sets, and no two floors may
share an id.
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVELS_DIR = os.path.join(ROOT, "levels")
OUT_PATH = os.path.join(ROOT, "src", "levels_data.rs")

ENEMY_TYPES = {"idle": "Idle", "wandering": "Wandering", "patrolling": "Patrolling"}
WEAPONS = {"pistol": "Pistol", "shotgun": "Shotgun", "machinegun": "MachineGun", "melee": "Melee"}
SPEAKERS = {"CL4-UD3", "HUNTER", "SENTINEL", "DRIFTER", "SWARM", "CORRUPTOR", "UPLINK"}
TRIGGERS = {"start", "enter_zone", "kills", "all_dead", "timer", "exit_open", "step_done",
            "boss_dead", "extracted"}
ACTIONS = {"say", "spawn", "open_exit", "close_exit", "objective", "sfx"}
SFX = {"elevator", "mask_crack", "level_clear", "pickup", "throw", "enemy_down"}


class Invalid(Exception):
    pass


def f32(v):
    """Format a number as a Rust f32 literal, deterministically."""
    v = float(v)
    if v != v or v in (float("inf"), float("-inf")):
        raise Invalid(f"non-finite number {v!r}")
    if v == int(v) and abs(v) < 1e15:
        return f"{int(v)}.0"
    return repr(v)


def rstr(s):
    """A Rust string literal (UTF-8 passthrough, escaping only what must be)."""
    if not isinstance(s, str):
        raise Invalid(f"expected string, got {s!r}")
    out = s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "").replace("\t", "\\t")
    return f'"{out}"'


def ident(name):
    """Turn an id into a Rust-safe identifier fragment (for static names)."""
    return "".join(c if c.isalnum() else "_" for c in str(name)).upper()


def rect(d, what):
    for k in ("x", "y", "w", "h"):
        if k not in d:
            raise Invalid(f"{what}: missing '{k}'")
    if float(d["w"]) <= 0 or float(d["h"]) <= 0:
        raise Invalid(f"{what}: non-positive size")
    return f"Rect::new({f32(d['x'])}, {f32(d['y'])}, {f32(d['w'])}, {f32(d['h'])})"


def load_floors():
    with open(os.path.join(LEVELS_DIR, "index.json"), encoding="utf-8") as fh:
        index = json.load(fh)
    floors = []
    for entry in index["floors"]:
        path = os.path.join(LEVELS_DIR, entry["file"])
        with open(path, encoding="utf-8") as fh:
            floor = json.load(fh)
        if floor.get("id") != entry.get("id"):
            raise Invalid(f"{entry['file']}: id {floor.get('id')} != index id {entry.get('id')}")
        floor["_file"] = entry["file"]
        floors.append(floor)
    return floors


def validate(floors):
    ids = [f["id"] for f in floors]
    if len(set(ids)) != len(ids):
        raise Invalid(f"duplicate floor ids: {ids}")
    if not all(isinstance(i, int) and i >= 1 for i in ids):
        raise Invalid(f"floor ids must be positive integers: {ids}")
    id_set = set(ids)
    for f in floors:
        tag = f["_file"]
        for key in ("id", "name", "theme", "accent", "flavor", "objective", "size", "entry",
                    "exits", "walls", "spawns", "scenario"):
            if key not in f:
                raise Invalid(f"{tag}: missing '{key}'")
        acc = f["accent"]
        if not (isinstance(acc, str) and len(acc) == 7 and acc[0] == "#"
                and all(c in "0123456789abcdefABCDEF" for c in acc[1:])):
            raise Invalid(f"{tag}: accent must be #rrggbb, got {acc!r}")
        exits = f["exits"]
        if not exits:
            raise Invalid(f"{tag}: needs at least one exit")
        exit_ids = [e["id"] for e in exits]
        if len(set(exit_ids)) != len(exit_ids):
            raise Invalid(f"{tag}: duplicate exit ids {exit_ids}")
        for e in exits:
            to = e.get("to", f["id"] + 1)
            if to != 0 and to not in id_set:
                raise Invalid(f"{tag}: exit '{e['id']}' leads to unknown floor {to}")
        zone_ids = [z["id"] for z in f.get("zones", [])]
        if len(set(zone_ids)) != len(zone_ids):
            raise Invalid(f"{tag}: duplicate zone ids {zone_ids}")
        room_ids = [r["id"] for r in f.get("rooms", [])]
        if len(set(room_ids)) != len(room_ids):
            raise Invalid(f"{tag}: duplicate room ids {room_ids}")
        for s in f["spawns"]:
            if s.get("type", "idle") not in ENEMY_TYPES:
                raise Invalid(f"{tag}: bad spawn type {s.get('type')!r}")
        for p in f.get("pickups", []):
            if p.get("weapon") not in WEAPONS:
                raise Invalid(f"{tag}: bad pickup weapon {p.get('weapon')!r}")
        step_ids = []
        for i, st in enumerate(f["scenario"]):
            sid = st.get("id", f"step_{i}")
            step_ids.append(sid)
        if len(set(step_ids)) != len(step_ids):
            raise Invalid(f"{tag}: duplicate step ids {step_ids}")
        for i, st in enumerate(f["scenario"]):
            sid = step_ids[i]
            trig = st.get("trigger") or {}
            kind = trig.get("kind")
            if kind not in TRIGGERS:
                raise Invalid(f"{tag}/{sid}: unknown trigger kind {kind!r}")
            if kind == "enter_zone" and trig.get("zone") not in zone_ids:
                raise Invalid(f"{tag}/{sid}: enter_zone references unknown zone {trig.get('zone')!r}")
            if kind == "kills" and not (isinstance(trig.get("count"), int) and trig["count"] >= 1):
                raise Invalid(f"{tag}/{sid}: kills needs an integer count >= 1")
            if kind == "timer":
                if not isinstance(trig.get("seconds"), (int, float)) or trig["seconds"] < 0:
                    raise Invalid(f"{tag}/{sid}: timer needs seconds >= 0")
                if "after" in trig and trig["after"] not in step_ids:
                    raise Invalid(f"{tag}/{sid}: timer.after references unknown step {trig['after']!r}")
            if kind == "exit_open" and "exit" in trig and trig["exit"] not in exit_ids:
                raise Invalid(f"{tag}/{sid}: exit_open references unknown exit {trig['exit']!r}")
            if kind == "step_done" and trig.get("step") not in step_ids:
                raise Invalid(f"{tag}/{sid}: step_done references unknown step {trig.get('step')!r}")
            for a in st.get("actions", []):
                if len(a) != 1 or next(iter(a)) not in ACTIONS:
                    raise Invalid(f"{tag}/{sid}: bad action {a!r}")
                (name, payload), = a.items()
                if name == "say":
                    if payload.get("who") not in SPEAKERS:
                        raise Invalid(f"{tag}/{sid}: unknown speaker {payload.get('who')!r}")
                    if not isinstance(payload.get("text"), str) or not payload["text"]:
                        raise Invalid(f"{tag}/{sid}: say needs text")
                    if "delay" in payload and (not isinstance(payload["delay"], (int, float)) or payload["delay"] < 0):
                        raise Invalid(f"{tag}/{sid}: say.delay must be >= 0")
                elif name == "spawn":
                    for s in payload:
                        if s.get("type", "idle") not in ENEMY_TYPES:
                            raise Invalid(f"{tag}/{sid}: bad wave spawn type {s.get('type')!r}")
                elif name in ("open_exit", "close_exit"):
                    if payload not in exit_ids:
                        raise Invalid(f"{tag}/{sid}: {name} references unknown exit {payload!r}")
                elif name == "objective":
                    if not isinstance(payload, str):
                        raise Invalid(f"{tag}/{sid}: objective must be a string")
                elif name == "sfx":
                    if payload not in SFX:
                        raise Invalid(f"{tag}/{sid}: unknown sfx {payload!r}")


def elevator(e, floor_id, what):
    to = e.get("to", floor_id + 1)
    return (f"ElevatorDef {{ id: {rstr(e['id'])}, rect: {rect(e, what)}, "
            f"label: {rstr(e.get('label', e['id']))}, to: {int(to)}, "
            f"open: {'true' if e.get('open', False) else 'false'} }}")


def spawn(s):
    return f"SpawnDef {{ x: {f32(s['x'])}, y: {f32(s['y'])}, kind: EnemyType::{ENEMY_TYPES[s.get('type', 'idle')]} }}"


def gen_floor(f, out):
    fid = f["id"]
    name = f"FLOOR_{ident(fid)}"
    tag = f["_file"]
    out.append(f"// ---- {tag}: FLOOR {fid} — {f['name']} " + "-" * max(1, 60 - len(f["name"])))
    out.append("")
    # Spawn waves need their own statics (an Action holds a slice).
    wave_names = []
    for i, st in enumerate(f["scenario"]):
        sid = st.get("id", f"step_{i}")
        for j, a in enumerate(st.get("actions", [])):
            if "spawn" in a:
                wname = f"{name}_WAVE_{ident(sid)}_{j}"
                wave_names.append(wname)
                out.append(f"static {wname}: [SpawnDef; {len(a['spawn'])}] = [")
                for s in a["spawn"]:
                    out.append(f"    {spawn(s)},")
                out.append("];")
                out.append("")
    # Per-step action slices.
    for i, st in enumerate(f["scenario"]):
        sid = st.get("id", f"step_{i}")
        aname = f"{name}_ACTIONS_{ident(sid)}"
        out.append(f"static {aname}: [Action; {len(st.get('actions', []))}] = [")
        for j, a in enumerate(st.get("actions", [])):
            (kind, payload), = a.items()
            if kind == "say":
                out.append(f"    Action::Say(SayDef {{ who: {rstr(payload['who'])}, "
                           f"text: {rstr(payload['text'])}, delay: {f32(payload.get('delay', 0))} }}),")
            elif kind == "spawn":
                out.append(f"    Action::Spawn(&{name}_WAVE_{ident(sid)}_{j}),")
            elif kind == "open_exit":
                out.append(f"    Action::OpenExit({rstr(payload)}),")
            elif kind == "close_exit":
                out.append(f"    Action::CloseExit({rstr(payload)}),")
            elif kind == "objective":
                out.append(f"    Action::Objective({rstr(payload)}),")
            elif kind == "sfx":
                out.append(f"    Action::Sfx({rstr(payload)}),")
        out.append("];")
        out.append("")
    # Steps.
    out.append(f"static {name}_SCENARIO: [StepDef; {len(f['scenario'])}] = [")
    for i, st in enumerate(f["scenario"]):
        sid = st.get("id", f"step_{i}")
        trig = st["trigger"]
        k = trig["kind"]
        if k == "start":
            t = "Trigger::Start"
        elif k == "enter_zone":
            t = f"Trigger::EnterZone({rstr(trig['zone'])})"
        elif k == "kills":
            t = f"Trigger::Kills({int(trig['count'])})"
        elif k == "all_dead":
            t = "Trigger::AllDead"
        elif k == "timer":
            after = f"Some({rstr(trig['after'])})" if "after" in trig else "None"
            t = f"Trigger::Timer {{ seconds: {f32(trig['seconds'])}, after: {after} }}"
        elif k == "exit_open":
            ex = f"Some({rstr(trig['exit'])})" if "exit" in trig else "None"
            t = f"Trigger::ExitOpen({ex})"
        elif k == "boss_dead":
            t = "Trigger::BossDead"
        elif k == "extracted":
            t = "Trigger::Extracted"
        else:
            t = f"Trigger::StepDone({rstr(trig['step'])})"
        out.append(f"    StepDef {{ id: {rstr(sid)}, trigger: {t}, actions: &{name}_ACTIONS_{ident(sid)} }},")
    out.append("];")
    out.append("")
    # Geometry.
    out.append(f"static {name}_EXITS: [ElevatorDef; {len(f['exits'])}] = [")
    for e in f["exits"]:
        out.append(f"    {elevator(e, fid, tag + ' exit')},")
    out.append("];")
    out.append("")
    out.append(f"static {name}_WALLS: [Rect; {len(f['walls'])}] = [")
    for w in f["walls"]:
        out.append(f"    {rect(w, tag + ' wall')},")
    out.append("];")
    out.append("")
    rooms = f.get("rooms", [])
    out.append(f"static {name}_ROOMS: [RoomDef; {len(rooms)}] = [")
    for r in rooms:
        out.append(f"    RoomDef {{ id: {rstr(r['id'])}, label: {rstr(r.get('label', r['id']))}, rect: {rect(r, tag + ' room')} }},")
    out.append("];")
    out.append("")
    zones = f.get("zones", [])
    out.append(f"static {name}_ZONES: [ZoneDef; {len(zones)}] = [")
    for z in zones:
        out.append(f"    ZoneDef {{ id: {rstr(z['id'])}, rect: {rect(z, tag + ' zone')} }},")
    out.append("];")
    out.append("")
    out.append(f"static {name}_SPAWNS: [SpawnDef; {len(f['spawns'])}] = [")
    for s in f["spawns"]:
        out.append(f"    {spawn(s)},")
    out.append("];")
    out.append("")
    pickups = f.get("pickups", [])
    out.append(f"static {name}_PICKUPS: [PickupDef; {len(pickups)}] = [")
    for p in pickups:
        out.append(f"    PickupDef {{ x: {f32(p['x'])}, y: {f32(p['y'])}, weapon: WeaponType::{WEAPONS[p['weapon']]} }},")
    out.append("];")
    out.append("")
    size = f["size"]
    out.append(f"pub static {name}: FloorDef = FloorDef {{")
    out.append(f"    id: {fid},")
    out.append(f"    name: {rstr(f['name'])},")
    out.append(f"    theme: {rstr(f['theme'])},")
    out.append(f"    accent: {rstr(f['accent'])},")
    out.append(f"    flavor: {rstr(f['flavor'])},")
    out.append(f"    objective: {rstr(f['objective'])},")
    out.append(f"    width: {f32(size['w'])},")
    out.append(f"    height: {f32(size['h'])},")
    out.append(f"    entry: {elevator(dict(f['entry'], to=0), fid, tag + ' entry')},")
    out.append(f"    exits: &{name}_EXITS,")
    out.append(f"    walls: &{name}_WALLS,")
    out.append(f"    rooms: &{name}_ROOMS,")
    out.append(f"    zones: &{name}_ZONES,")
    out.append(f"    spawns: &{name}_SPAWNS,")
    out.append(f"    pickups: &{name}_PICKUPS,")
    out.append(f"    scenario: &{name}_SCENARIO,")
    out.append("};")
    out.append("")
    return name


def generate(floors):
    out = [
        "// @generated by tools/gen_levels.py from levels/*.json — DO NOT EDIT.",
        "// Re-run `make gen-levels` after editing the JSON (the level editor writes it).",
        "//",
        "// Floors in play order; see docs/SCENARIO_FORMAT.md for the contract and",
        "// src/scenario.rs for the types.",
        "#![allow(clippy::all)]",
        "#![allow(clippy::excessive_precision)]",
        "",
        "use crate::components::{EnemyType, WeaponType};",
        "use crate::scenario::{",
        "    Action, ElevatorDef, FloorDef, PickupDef, Rect, RoomDef, SayDef, SpawnDef, StepDef, Trigger,",
        "    ZoneDef,",
        "};",
        "",
    ]
    names = []
    for f in sorted(floors, key=lambda f: f["id"]):
        names.append(gen_floor(f, out))
    out.append("/// Number of floors (13 + the hidden 13½).")
    out.append(f"pub const FLOOR_COUNT: usize = {len(names)};")
    out.append("")
    out.append("/// Every floor, in play order (index = floor id - 1).")
    out.append("pub static FLOORS: [&FloorDef; FLOOR_COUNT] = [")
    for n in names:
        out.append(f"    &{n},")
    out.append("];")
    out.append("")
    return "\n".join(out)


def main(argv):
    check = "--check" in argv
    try:
        floors = load_floors()
        validate(floors)
        text = generate(floors)
    except (Invalid, KeyError, OSError, ValueError, json.JSONDecodeError) as e:
        print(f"gen_levels: error: {e}", file=sys.stderr)
        return 1
    if check:
        try:
            with open(OUT_PATH, encoding="utf-8") as fh:
                current = fh.read()
        except OSError:
            current = None
        if current != text:
            print(f"gen_levels: {os.path.relpath(OUT_PATH, ROOT)} is out of date — run `make gen-levels`",
                  file=sys.stderr)
            return 1
        print(f"gen_levels: {len(floors)} floors valid, {os.path.relpath(OUT_PATH, ROOT)} up to date")
        return 0
    with open(OUT_PATH, "w", encoding="utf-8") as fh:
        fh.write(text)
    print(f"gen_levels: wrote {os.path.relpath(OUT_PATH, ROOT)} ({len(floors)} floors)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
