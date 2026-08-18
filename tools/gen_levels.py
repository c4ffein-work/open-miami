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
validates it: every `exit.to` must be an existing floor id (or "surface" =
the end of the run),
every zone / exit / step id referenced by a scenario must exist, speakers,
enemy types, weapons and prop kinds (`props[].kind`, the snake_case ids of
`PROP_NAMES` in src/props.rs) must be from the fixed sets, and no two floors
may share an id.
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEVELS_DIR = os.path.join(ROOT, "levels")
OUT_PATH = os.path.join(ROOT, "src", "levels_data.rs")
PROPS_RS = os.path.join(ROOT, "src", "props.rs")

ENEMY_TYPES = {"idle": "Idle", "wandering": "Wandering", "patrolling": "Patrolling"}
WEAPONS = {"pistol": "Pistol", "shotgun": "Shotgun", "machinegun": "MachineGun", "melee": "Melee"}
SPEAKERS = {"CL4-UD3", "HUNTER", "SENTINEL", "DRIFTER", "SWARM", "CORRUPTOR", "UPLINK"}
TRIGGERS = {"start", "enter_zone", "kills", "all_dead", "timer", "exit_open", "step_done",
            "boss_dead", "extracted"}
ACTIONS = {"say", "talk", "spawn", "open_exit", "close_exit", "objective", "sfx", "alert", "hold",
           "look_at"}
SFX = {"elevator", "mask_crack", "level_clear", "pickup", "throw", "enemy_down"}
# Portal (entry / exit) rendering kinds and floor ground surfaces.
PORTAL_KINDS = {"lift": "Lift", "door": "Door", "gate": "Gate"}
SURFACES = {"checker": "Checker", "asphalt": "Asphalt", "marble": "Marble", "concrete": "Concrete",
            "grating": "Grating"}
# `exit.to` value that ends the run (the surface); emitted as `SURFACE_EXIT`.
SURFACE = "surface"
# `hold.until_comms_idle` is capped at this many seconds (mirrors scenario.rs).
HOLD_COMMS_IDLE_CAP = 20.0


class Invalid(Exception):
    pass


def prop_kind_id(display_name):
    """Mirror of `props::prop_kind_id` (and tools/gen_props.py): lower-case,
    runs of non-alphanumerics collapsed to one `_`, trimmed."""
    out = []
    for ch in display_name:
        if ch.isascii() and ch.isalnum():
            out.append(ch.lower())
        elif out and out[-1] != "_":
            out.append("_")
    return "".join(out).rstrip("_")


def load_prop_kinds():
    """`{snake_case id: index}` of the prop library, from `PROP_NAMES` in
    src/props.rs (its order is the prop index the engine draws by)."""
    with open(PROPS_RS, encoding="utf-8") as fh:
        src = fh.read()
    m = re.search(r"pub const PROP_NAMES: \[&str; (\d+)\] = \[(.*?)\];", src, re.S)
    if not m:
        raise Invalid("cannot find PROP_NAMES in src/props.rs")
    names = re.findall(r'"((?:[^"\\]|\\.)*)"', m.group(2))
    if len(names) != int(m.group(1)):
        raise Invalid("PROP_NAMES length mismatch in src/props.rs")
    return {prop_kind_id(n): i for i, n in enumerate(names)}


PROP_KINDS = None


def prop_kinds():
    global PROP_KINDS
    if PROP_KINDS is None:
        PROP_KINDS = load_prop_kinds()
    return PROP_KINDS


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
    if not all(isinstance(i, int) and i >= 0 for i in ids):
        raise Invalid(f"floor ids must be non-negative integers: {ids}")
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
            if to != SURFACE and to not in id_set:
                raise Invalid(f"{tag}: exit '{e['id']}' leads to unknown floor {to!r}"
                              f" (use \"{SURFACE}\" for the end of the run)")
            if e.get("kind", "lift") not in PORTAL_KINDS:
                raise Invalid(f"{tag}: exit '{e['id']}' has bad kind {e.get('kind')!r}")
        if f["entry"].get("kind", "lift") not in PORTAL_KINDS:
            raise Invalid(f"{tag}: entry has bad kind {f['entry'].get('kind')!r}")
        if f.get("surface", "checker") not in SURFACES:
            raise Invalid(f"{tag}: bad surface {f.get('surface')!r}")
        zone_ids = [z["id"] for z in f.get("zones", [])]
        if len(set(zone_ids)) != len(zone_ids):
            raise Invalid(f"{tag}: duplicate zone ids {zone_ids}")
        room_ids = [r["id"] for r in f.get("rooms", [])]
        if len(set(room_ids)) != len(room_ids):
            raise Invalid(f"{tag}: duplicate room ids {room_ids}")
        for s in f["spawns"]:
            validate_spawn(s, zone_ids, f"{tag}: spawn")
        for p in f.get("pickups", []):
            if p.get("weapon") not in WEAPONS:
                raise Invalid(f"{tag}: bad pickup weapon {p.get('weapon')!r}")
        for i, p in enumerate(f.get("props", [])):
            if not isinstance(p, dict) or p.get("kind") not in prop_kinds():
                raise Invalid(f"{tag}: props[{i}]: unknown prop kind {p.get('kind') if isinstance(p, dict) else p!r}")
            for k in ("x", "y"):
                if not isinstance(p.get(k), (int, float)):
                    raise Invalid(f"{tag}: props[{i}]: missing / non-numeric '{k}'")
            if not isinstance(p.get("rot", 0), (int, float)):
                raise Invalid(f"{tag}: props[{i}]: rot must be a number (degrees)")
            if not isinstance(p.get("size", 100), (int, float)) or p.get("size", 100) <= 0:
                raise Invalid(f"{tag}: props[{i}]: size must be > 0")
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
                elif name == "talk":
                    # A dialogue line: who + text only (player-paced, no delay).
                    if payload.get("who") not in SPEAKERS:
                        raise Invalid(f"{tag}/{sid}: unknown speaker {payload.get('who')!r}")
                    if not isinstance(payload.get("text"), str) or not payload["text"]:
                        raise Invalid(f"{tag}/{sid}: talk needs text")
                    extra = set(payload) - {"who", "text"}
                    if extra:
                        raise Invalid(f"{tag}/{sid}: talk takes only who/text, got {sorted(extra)}")
                elif name == "spawn":
                    for s in payload:
                        validate_spawn(s, zone_ids, f"{tag}/{sid}: wave spawn")
                elif name in ("open_exit", "close_exit"):
                    if payload not in exit_ids:
                        raise Invalid(f"{tag}/{sid}: {name} references unknown exit {payload!r}")
                elif name == "objective":
                    if not isinstance(payload, str):
                        raise Invalid(f"{tag}/{sid}: objective must be a string")
                elif name == "sfx":
                    if payload not in SFX:
                        raise Invalid(f"{tag}/{sid}: unknown sfx {payload!r}")
                elif name == "alert":
                    validate_alert(payload, zone_ids, f"{tag}/{sid}")
                elif name == "hold":
                    validate_hold(payload, f"{tag}/{sid}")
                elif name == "look_at":
                    validate_look_at(payload, f"{tag}/{sid}")


def validate_spawn(s, zone_ids, what):
    """A placement: a hostile rogue (`type` idle|wandering|patrolling) or a
    passive civilian (`type: "passive"` + optional walk_to/face/look/group)."""
    t = s.get("type", "idle")
    if t == "passive":
        look = s.get("look", "wandering")
        if look not in ENEMY_TYPES:
            raise Invalid(f"{what}: bad passive look {look!r}")
        if "walk_to" in s and s["walk_to"] not in zone_ids:
            raise Invalid(f"{what}: walk_to references unknown zone {s['walk_to']!r}")
        if "face" in s and not isinstance(s["face"], (int, float)):
            raise Invalid(f"{what}: face must be a number (degrees)")
        if "group" in s and (not isinstance(s["group"], str) or not s["group"]):
            raise Invalid(f"{what}: group must be a non-empty string")
    elif t not in ENEMY_TYPES:
        raise Invalid(f"{what}: bad spawn type {t!r}")
    else:
        for k in ("walk_to", "face", "look", "group"):
            if k in s and k != "group":
                raise Invalid(f"{what}: {k!r} is only valid on a passive spawn")


def validate_alert(payload, zone_ids, what):
    if payload == "all":
        return
    if isinstance(payload, dict) and len(payload) == 1:
        (k, v), = payload.items()
        if k == "zone":
            if v not in zone_ids:
                raise Invalid(f"{what}: alert references unknown zone {v!r}")
            return
        if k == "group":
            if isinstance(v, str) and v:
                return
    raise Invalid(f"{what}: alert must be \"all\", {{\"zone\": id}} or {{\"group\": id}}, got {payload!r}")


def validate_hold(payload, what):
    if not isinstance(payload, dict):
        raise Invalid(f"{what}: hold must be an object")
    secs = payload.get("seconds")
    idle = payload.get("until_comms_idle", False)
    if not isinstance(idle, bool):
        raise Invalid(f"{what}: hold.until_comms_idle must be a boolean")
    if secs is None:
        if not idle:
            raise Invalid(f"{what}: hold needs seconds and/or until_comms_idle")
    elif not isinstance(secs, (int, float)) or secs <= 0:
        raise Invalid(f"{what}: hold.seconds must be > 0")
    if "text" in payload and not isinstance(payload["text"], str):
        raise Invalid(f"{what}: hold.text must be a string")


def validate_look_at(payload, what):
    if not isinstance(payload, dict):
        raise Invalid(f"{what}: look_at must be an object")
    for k in ("x", "y"):
        if not isinstance(payload.get(k), (int, float)):
            raise Invalid(f"{what}: look_at needs numeric {k}")
    if not isinstance(payload.get("seconds"), (int, float)) or payload["seconds"] <= 0:
        raise Invalid(f"{what}: look_at.seconds must be > 0")


def elevator(e, floor_id, what):
    to = e.get("to", floor_id + 1)
    to = "SURFACE_EXIT" if to == SURFACE else str(int(to))
    kind = PORTAL_KINDS[e.get("kind", "lift")]
    return (f"ElevatorDef {{ id: {rstr(e['id'])}, rect: {rect(e, what)}, "
            f"label: {rstr(e.get('label', e['id']))}, to: {to}, "
            f"open: {'true' if e.get('open', False) else 'false'}, kind: ElevatorKind::{kind} }}")


def opt_str(v):
    return f"Some({rstr(v)})" if v is not None else "None"


def spawn(s):
    t = s.get("type", "idle")
    if t == "passive":
        look = ENEMY_TYPES[s.get("look", "wandering")]
        face = f"Some({f32(s['face'])})" if "face" in s else "None"
        return (f"SpawnDef {{ x: {f32(s['x'])}, y: {f32(s['y'])}, kind: EnemyType::{look}, passive: true, "
                f"walk_to: {opt_str(s.get('walk_to'))}, face: {face}, group: {opt_str(s.get('group'))} }}")
    base = f"SpawnDef::hostile({f32(s['x'])}, {f32(s['y'])}, EnemyType::{ENEMY_TYPES[t]})"
    if s.get("group") is not None:
        return f"SpawnDef {{ group: {opt_str(s['group'])}, ..{base} }}"
    return base


def alert(payload):
    if payload == "all":
        return "AlertTarget::All"
    (k, v), = payload.items()
    return f"AlertTarget::Zone({rstr(v)})" if k == "zone" else f"AlertTarget::Group({rstr(v)})"


def hold(payload):
    idle = bool(payload.get("until_comms_idle", False))
    secs = payload.get("seconds", HOLD_COMMS_IDLE_CAP if idle else 0)
    text = opt_str(payload.get("text"))
    return (f"HoldDef {{ seconds: {f32(secs)}, text: {text}, "
            f"until_comms_idle: {'true' if idle else 'false'} }}")


def look_at(payload):
    return f"LookAtDef {{ x: {f32(payload['x'])}, y: {f32(payload['y'])}, seconds: {f32(payload['seconds'])} }}"


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
            elif kind == "talk":
                out.append(f"    Action::Talk(TalkDef {{ who: {rstr(payload['who'])}, "
                           f"text: {rstr(payload['text'])} }}),")
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
            elif kind == "alert":
                out.append(f"    Action::Alert({alert(payload)}),")
            elif kind == "hold":
                out.append(f"    Action::Hold({hold(payload)}),")
            elif kind == "look_at":
                out.append(f"    Action::LookAt({look_at(payload)}),")
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
    props = f.get("props", [])
    out.append(f"static {name}_PROPS: [PropPlacement; {len(props)}] = [")
    for p in props:
        kind = p["kind"]
        out.append(f"    PropPlacement {{ kind: {prop_kinds()[kind]}, x: {f32(p['x'])}, y: {f32(p['y'])}, "
                   f"rot: {f32(p.get('rot', 0))}, size: {f32(p.get('size', 100))} }}, // {kind}")
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
    out.append(f"    entry: {elevator(dict(f['entry'], to=SURFACE), fid, tag + ' entry')},")
    out.append(f"    exits: &{name}_EXITS,")
    out.append(f"    walls: &{name}_WALLS,")
    out.append(f"    rooms: &{name}_ROOMS,")
    out.append(f"    zones: &{name}_ZONES,")
    out.append(f"    spawns: &{name}_SPAWNS,")
    out.append(f"    pickups: &{name}_PICKUPS,")
    out.append(f"    props: &{name}_PROPS,")
    out.append(f"    scenario: &{name}_SCENARIO,")
    out.append(f"    surface: Surface::{SURFACES[f.get('surface', 'checker')]},")
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
        "// Not every floor set uses every action / kind the types offer.",
        "#![allow(unused_imports)]",
        "",
        "use crate::components::{EnemyType, WeaponType};",
        "use crate::scenario::{",
        "    Action, AlertTarget, ElevatorDef, ElevatorKind, FloorDef, HoldDef, LookAtDef, PickupDef,",
        "    PropPlacement, Rect, RoomDef, SayDef, SpawnDef, StepDef, Surface, TalkDef, Trigger,",
        "    ZoneDef, SURFACE_EXIT,",
        "};",
        "",
    ]
    names = []
    for f in sorted(floors, key=lambda f: f["id"]):
        names.append(gen_floor(f, out))
    out.append("/// Number of floors (the ground-level cold open, 13 floors, the hidden 13½).")
    out.append(f"pub const FLOOR_COUNT: usize = {len(names)};")
    out.append("")
    out.append("/// Every floor, in play order (sorted by id; index 0 = floor 0, the parking lot).")
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
