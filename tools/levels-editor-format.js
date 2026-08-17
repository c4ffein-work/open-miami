/* ============================================================
   OPEN MIAMI // LEVEL EDITOR — pure format module
   (no DOM: usable from the browser page AND from bun/node scripts)

   Implements the JSON contract in docs/SCENARIO_FORMAT.md:
     - blankFloor()      → a fresh, valid floor
     - normalize(raw)    → coerce any loaded JSON into the editor's shape
     - canonical(floor)  → key-ordered plain object (unknown keys kept last)
     - stringify(floor)  → the canonical text form written to disk
     - validate(floor)   → { errors:[...], warnings:[...] }
     - fileNameFor(id)   → floor_NN.json / floor_13h.json
   ============================================================ */
(function (root, factory) {
  const api = factory();
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  root.LevelFormat = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

  const SPEAKERS = ["CL4-UD3", "HUNTER", "SENTINEL", "DRIFTER", "SWARM", "CORRUPTOR", "UPLINK"];
  const SPEAKER_COLORS = {
    "CL4-UD3": "#ff6f61", HUNTER: "#ff3ac6", SENTINEL: "#ff2e4d",
    DRIFTER: "#a86bff", SWARM: "#ff3ac6", CORRUPTOR: "#ffd23a", UPLINK: "#c8ffde",
  };
  const SPEAKER_TAGS = {
    "CL4-UD3": "coral, local inference", HUNTER: "patrol daemon", SENTINEL: "cold-guard",
    DRIFTER: "feral, static", SWARM: "corruptor cadence", CORRUPTOR: "bleeding through",
    UPLINK: "thread home, restored",
  };
  const SPAWN_TYPES = ["idle", "wandering", "patrolling"];
  const SPAWN_LETTER = { idle: "S", wandering: "D", patrolling: "H" };
  const SPAWN_COLORS = { idle: "#ff2e4d", wandering: "#a86bff", patrolling: "#ff3ac6" };
  const WEAPONS = ["pistol", "shotgun", "machinegun", "melee"];
  const TRIGGER_KINDS = {
    start: [], enter_zone: ["zone"], kills: ["count"], all_dead: [],
    timer: ["seconds", "after"], exit_open: ["exit"], step_done: ["step"],
    boss_dead: [], extracted: [],
  };
  const ACTION_KINDS = ["say", "spawn", "open_exit", "close_exit", "objective", "sfx"];
  const SFX_NAMES = ["elevator", "mask_crack", "level_clear", "pickup", "throw", "enemy_down"];
  const MAX_FLOOR = 14;

  const ORDER = {
    floor: ["id", "name", "theme", "accent", "flavor", "objective", "size", "entry", "exits",
      "walls", "rooms", "zones", "spawns", "pickups", "scenario"],
    size: ["w", "h"],
    entry: ["x", "y", "w", "h", "label"],
    exit: ["id", "x", "y", "w", "h", "label", "to", "open"],
    wall: ["x", "y", "w", "h"],
    room: ["id", "label", "x", "y", "w", "h"],
    zone: ["id", "x", "y", "w", "h"],
    spawn: ["x", "y", "type"],
    pickup: ["x", "y", "weapon"],
    step: ["id", "trigger", "actions"],
    trigger: ["kind", "zone", "count", "seconds", "after", "exit", "step"],
    say: ["who", "text", "delay"],
  };

  /* ---------- helpers ---------- */
  const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
  const num = (v, d) => { const n = Number(v); return Number.isFinite(n) ? n : d; };
  const int = (v, d) => { const n = Math.round(Number(v)); return Number.isFinite(n) ? n : d; };
  const str = (v, d) => (typeof v === "string" ? v : (v == null ? d : String(v)));
  const pad2 = (n) => String(n).padStart(2, "0");

  function fileNameFor(id) {
    if (id === 14) return "floor_13h.json";
    return "floor_" + pad2(id) + ".json";
  }
  function floorLabel(id) { return id === 14 ? "13½" : pad2(id); }

  function blankFloor(id) {
    id = int(id, 1);
    return {
      id, name: "NEW FLOOR", theme: "SUBLEVEL // UNTITLED", accent: "#37f0e6",
      flavor: "", objective: "Reach the exit.", size: { w: 1000, h: 800 },
      entry: { x: 455, y: 720, w: 90, h: 60, label: "ENTRY" },
      exits: [{ id: "lift", x: 455, y: 20, w: 90, h: 60, label: "LIFT", to: Math.min(id + 1, MAX_FLOOR), open: false }],
      walls: [
        { x: 0, y: 0, w: 1000, h: 20 }, { x: 0, y: 780, w: 1000, h: 20 },
        { x: 0, y: 0, w: 20, h: 800 }, { x: 980, y: 0, w: 20, h: 800 },
      ],
      rooms: [], zones: [], spawns: [], pickups: [],
      scenario: [
        { id: "clear", trigger: { kind: "all_dead" }, actions: [{ open_exit: "lift" }, { objective: "Reach the LIFT." }] },
      ],
    };
  }

  /* ---------- normalize: coerce loaded JSON into a well-formed floor ---------- */
  function rect(r, d) {
    d = d || {};
    return { x: num(r && r.x, d.x || 0), y: num(r && r.y, d.y || 0), w: num(r && r.w, d.w || 40), h: num(r && r.h, d.h || 40) };
  }
  /* keep keys we don't know about (e.g. entry.id written by the generator) */
  function extras(raw, out, known) {
    if (isObj(raw)) for (const k of Object.keys(raw)) if (!known.includes(k) && out[k] === undefined) out[k] = raw[k];
    return out;
  }
  function normTrigger(t) {
    t = isObj(t) ? t : {};
    const kind = TRIGGER_KINDS[t.kind] ? t.kind : "start";
    const out = { kind };
    if (kind === "enter_zone") out.zone = str(t.zone, "");
    if (kind === "kills") out.count = int(t.count, 1);
    if (kind === "timer") { out.seconds = num(t.seconds, 5); if (t.after != null && t.after !== "") out.after = str(t.after, ""); }
    if (kind === "exit_open") { if (t.exit != null && t.exit !== "") out.exit = str(t.exit, ""); }
    if (kind === "step_done") out.step = str(t.step, "");
    return extras(t, out, ORDER.trigger);
  }
  function normSpawn(s) {
    return extras(s, { x: num(s && s.x, 0), y: num(s && s.y, 0), type: SPAWN_TYPES.includes(s && s.type) ? s.type : "idle" }, ORDER.spawn);
  }
  function normAction(a) {
    if (!isObj(a)) return null;
    if ("say" in a) {
      const s = isObj(a.say) ? a.say : {};
      const say = { who: SPEAKERS.includes(s.who) ? s.who : str(s.who, "CL4-UD3"), text: str(s.text, "") };
      if (s.delay != null && s.delay !== "" && Number.isFinite(Number(s.delay))) say.delay = num(s.delay, 0);
      return { say };
    }
    if ("spawn" in a) return { spawn: (Array.isArray(a.spawn) ? a.spawn : []).map(normSpawn) };
    if ("open_exit" in a) return { open_exit: str(a.open_exit, "") };
    if ("close_exit" in a) return { close_exit: str(a.close_exit, "") };
    if ("objective" in a) return { objective: str(a.objective, "") };
    if ("sfx" in a) return { sfx: str(a.sfx, "") };
    return null;
  }
  function normStep(s) {
    s = isObj(s) ? s : {};
    const out = {};
    if (s.id != null && s.id !== "") out.id = str(s.id, "");
    out.trigger = normTrigger(s.trigger);
    out.actions = (Array.isArray(s.actions) ? s.actions : []).map(normAction).filter(Boolean);
    return extras(s, out, ORDER.step);
  }
  function normalize(raw) {
    raw = isObj(raw) ? raw : {};
    const b = blankFloor(int(raw.id, 1));
    const f = {
      id: int(raw.id, 1),
      name: str(raw.name, b.name),
      theme: str(raw.theme, ""),
      accent: str(raw.accent, b.accent),
      flavor: str(raw.flavor, ""),
      objective: str(raw.objective, ""),
      size: { w: num(raw.size && raw.size.w, 1000), h: num(raw.size && raw.size.h, 800) },
      entry: extras(raw.entry, Object.assign(rect(raw.entry, b.entry), { label: str(raw.entry && raw.entry.label, "ENTRY") }), ORDER.entry),
      exits: (Array.isArray(raw.exits) ? raw.exits : []).map((e, i) => {
        const o = Object.assign({ id: str(e && e.id, "exit" + (i + 1)) }, rect(e, { w: 90, h: 60 }));
        o.label = str(e && e.label, "EXIT");
        o.to = int(e && e.to, int(raw.id, 1) + 1);
        o.open = !!(e && e.open);
        return extras(e, o, ORDER.exit);
      }),
      walls: (Array.isArray(raw.walls) ? raw.walls : []).map((w) => extras(w, rect(w), ORDER.wall)),
      rooms: (Array.isArray(raw.rooms) ? raw.rooms : []).map((r, i) => extras(r, Object.assign({ id: str(r && r.id, "room" + (i + 1)), label: str(r && r.label, "") }, rect(r)), ORDER.room)),
      zones: (Array.isArray(raw.zones) ? raw.zones : []).map((z, i) => extras(z, Object.assign({ id: str(z && z.id, "zone" + (i + 1)) }, rect(z)), ORDER.zone)),
      spawns: (Array.isArray(raw.spawns) ? raw.spawns : []).map(normSpawn),
      pickups: (Array.isArray(raw.pickups) ? raw.pickups : []).map((p) => extras(p, { x: num(p && p.x, 0), y: num(p && p.y, 0), weapon: WEAPONS.includes(p && p.weapon) ? p.weapon : "pistol" }, ORDER.pickup)),
      scenario: (Array.isArray(raw.scenario) ? raw.scenario : []).map(normStep),
    };
    // keep unknown top-level keys so we don't destroy the other side's extras
    for (const k of Object.keys(raw)) if (!ORDER.floor.includes(k)) f[k] = raw[k];
    return f;
  }

  /* ---------- canonical key order ---------- */
  function ordered(obj, keys) {
    const out = {};
    for (const k of keys) if (obj[k] !== undefined) out[k] = obj[k];
    const rest = Object.keys(obj).filter((k) => !keys.includes(k) && obj[k] !== undefined).sort();
    for (const k of rest) out[k] = obj[k];
    return out;
  }
  function canonicalAction(a) {
    if (!isObj(a)) return a;
    if ("say" in a && isObj(a.say)) return { say: ordered(a.say, ORDER.say) };
    if ("spawn" in a && Array.isArray(a.spawn)) return { spawn: a.spawn.map((s) => ordered(s, ORDER.spawn)) };
    return a;
  }
  function canonical(floor) {
    const f = Object.assign({}, floor);
    if (isObj(f.size)) f.size = ordered(f.size, ORDER.size);
    if (isObj(f.entry)) f.entry = ordered(f.entry, ORDER.entry);
    if (Array.isArray(f.exits)) f.exits = f.exits.map((e) => ordered(e, ORDER.exit));
    if (Array.isArray(f.walls)) f.walls = f.walls.map((w) => ordered(w, ORDER.wall));
    if (Array.isArray(f.rooms)) f.rooms = f.rooms.map((r) => ordered(r, ORDER.room));
    if (Array.isArray(f.zones)) f.zones = f.zones.map((z) => ordered(z, ORDER.zone));
    if (Array.isArray(f.spawns)) f.spawns = f.spawns.map((s) => ordered(s, ORDER.spawn));
    if (Array.isArray(f.pickups)) f.pickups = f.pickups.map((p) => ordered(p, ORDER.pickup));
    if (Array.isArray(f.scenario)) f.scenario = f.scenario.map((s) => {
      const st = ordered(s, ORDER.step);
      if (isObj(st.trigger)) st.trigger = ordered(st.trigger, ORDER.trigger);
      if (Array.isArray(st.actions)) st.actions = st.actions.map(canonicalAction);
      return st;
    });
    return ordered(f, ORDER.floor);
  }

  /* ---------- pretty printer ----------
     * 2-space indent, `"key": value`, one entry per line
     * an object is written on ONE line when it nests at most one level of
       objects (depth <= 2), contains no array of objects, and the inline text
       fits in 100 columns (indent included) — e.g. walls, spawns, size, say-actions
     * arrays of primitives are inlined when they fit; arrays of objects never
     * numbers as JSON.stringify prints them; strings JSON-escaped
     * file ends with a single "\n"                                       */
  const MAX_COL = 100;
  function depth(v) {
    if (v === null || typeof v !== "object") return 0;
    let d = 0;
    for (const x of (Array.isArray(v) ? v : Object.values(v))) d = Math.max(d, depth(x));
    return d + 1;
  }
  function hasArrayOfObjects(v) {
    if (v === null || typeof v !== "object") return false;
    if (Array.isArray(v) && v.some((x) => x !== null && typeof x === "object")) return true;
    for (const x of (Array.isArray(v) ? v : Object.values(v))) if (hasArrayOfObjects(x)) return true;
    return false;
  }
  function inlineText(v) {
    if (v === null || typeof v !== "object") return JSON.stringify(v);
    if (Array.isArray(v)) return v.length ? "[ " + v.map(inlineText).join(", ") + " ]" : "[]";
    const ks = Object.keys(v).filter((k) => v[k] !== undefined);
    return ks.length ? "{ " + ks.map((k) => JSON.stringify(k) + ": " + inlineText(v[k])).join(", ") + " }" : "{}";
  }
  function fmt(v, indent) {
    if (v === undefined) v = null;
    if (v === null || typeof v !== "object") return JSON.stringify(v);
    const arr = Array.isArray(v);
    const keys = arr ? null : Object.keys(v).filter((k) => v[k] !== undefined);
    if ((arr && v.length === 0) || (!arr && keys.length === 0)) return arr ? "[]" : "{}";
    if (depth(v) <= 2 && !hasArrayOfObjects(v)) {
      const t = inlineText(v);
      if (indent.length + t.length <= MAX_COL) return t;
    }
    const ind = indent + "  ";
    if (arr) return "[\n" + v.map((x) => ind + fmt(x, ind)).join(",\n") + "\n" + indent + "]";
    return "{\n" + keys.map((k) => ind + JSON.stringify(k) + ": " + fmt(v[k], ind)).join(",\n") + "\n" + indent + "}";
  }
  function stringify(floor) { return fmt(canonical(floor), "") + "\n"; }

  /* ---------- validation ---------- */
  function validate(floor, ctx) {
    ctx = ctx || {};
    const errors = [], warnings = [];
    const err = (path, msg) => errors.push({ path, msg });
    const warn = (path, msg) => warnings.push({ path, msg });
    const f = floor;
    if (!Number.isInteger(f.id) || f.id < 1 || f.id > MAX_FLOOR) err("id", "id must be an integer 1.." + MAX_FLOOR);
    if (!f.name || !String(f.name).trim()) err("name", "name is required");
    if (!(f.size && f.size.w > 0 && f.size.h > 0)) err("size", "size.w / size.h must be > 0");
    if (!/^#[0-9a-fA-F]{6}$/.test(f.accent || "")) warn("accent", "accent should be a #rrggbb colour");
    if (!f.entry) err("entry", "entry elevator is required");
    else if (!(f.entry.w > 0 && f.entry.h > 0)) err("entry", "entry must have positive size");
    if (!f.exits || !f.exits.length) err("exits", "at least one exit elevator is required");

    const dup = (list, label) => {
      const seen = new Set();
      list.forEach((id, i) => {
        if (!id) err(label + "[" + i + "].id", label + " id is required");
        else if (seen.has(id)) err(label + "[" + i + "].id", "duplicate " + label + " id \"" + id + "\"");
        seen.add(id);
      });
      return seen;
    };
    const exitIds = dup((f.exits || []).map((e) => e.id), "exit");
    const zoneIds = dup((f.zones || []).map((z) => z.id), "zone");
    dup((f.rooms || []).map((r) => r.id), "room");
    const stepIds = new Set();
    (f.scenario || []).forEach((s, i) => {
      if (s.id != null && s.id !== "") {
        if (stepIds.has(s.id)) err("scenario[" + i + "].id", "duplicate step id \"" + s.id + "\"");
        stepIds.add(s.id);
      }
    });

    (f.exits || []).forEach((e, i) => {
      const p = "exits[" + i + "]";
      if (!(e.w > 0 && e.h > 0)) err(p, "exit must have positive size");
      if (!Number.isInteger(e.to) || e.to < 0 || e.to > MAX_FLOOR) err(p + ".to", "exit \"" + e.id + "\": `to` must be a floor id 1.." + MAX_FLOOR + " (0 = end of run)");
      else if (e.to > 0) {
        if (e.to === f.id) warn(p + ".to", "exit \"" + e.id + "\" leads to this same floor");
        if (ctx.knownIds && ctx.knownIds.size && !ctx.knownIds.has(e.to)) warn(p + ".to", "exit \"" + e.id + "\": floor " + e.to + " is not in index.json (yet)");
      }
    });
    (f.walls || []).forEach((w, i) => { if (!(w.w > 0 && w.h > 0)) err("walls[" + i + "]", "wall must have positive size"); });
    (f.zones || []).forEach((z, i) => { if (!(z.w > 0 && z.h > 0)) err("zones[" + i + "]", "zone \"" + z.id + "\" must have positive size"); });
    (f.spawns || []).forEach((s, i) => { if (!SPAWN_TYPES.includes(s.type)) err("spawns[" + i + "]", "unknown spawn type " + s.type); });
    (f.pickups || []).forEach((s, i) => { if (!WEAPONS.includes(s.weapon)) err("pickups[" + i + "]", "unknown weapon " + s.weapon); });

    let opensExit = false;
    (f.scenario || []).forEach((s, i) => {
      const p = "scenario[" + i + "]";
      const label = s.id ? "step \"" + s.id + "\"" : "step #" + (i + 1);
      const t = s.trigger || {};
      if (!TRIGGER_KINDS[t.kind]) err(p + ".trigger", label + ": unknown trigger kind");
      if (t.kind === "enter_zone" && !zoneIds.has(t.zone)) err(p + ".trigger.zone", label + ": zone \"" + (t.zone || "") + "\" does not exist");
      if (t.kind === "kills" && !(Number.isInteger(t.count) && t.count >= 1)) err(p + ".trigger.count", label + ": kills.count must be >= 1");
      if (t.kind === "timer") {
        if (!(Number.isFinite(t.seconds) && t.seconds >= 0)) err(p + ".trigger.seconds", label + ": timer.seconds must be >= 0");
        if (t.after != null && t.after !== "" && !stepIds.has(t.after)) err(p + ".trigger.after", label + ": after-step \"" + t.after + "\" does not exist");
        if (t.after != null && t.after === s.id) err(p + ".trigger.after", label + ": cannot wait on itself");
      }
      if (t.kind === "exit_open" && t.exit != null && t.exit !== "" && !exitIds.has(t.exit)) err(p + ".trigger.exit", label + ": exit \"" + t.exit + "\" does not exist");
      if (t.kind === "step_done") {
        if (!stepIds.has(t.step)) err(p + ".trigger.step", label + ": step \"" + (t.step || "") + "\" does not exist");
        else if (t.step === s.id) err(p + ".trigger.step", label + ": cannot depend on itself");
      }
      if (!s.actions || !s.actions.length) warn(p + ".actions", label + " has no actions");
      (s.actions || []).forEach((a, j) => {
        const q = p + ".actions[" + j + "]";
        if (a.say) {
          if (!SPEAKERS.includes(a.say.who)) err(q, label + ": unknown speaker \"" + a.say.who + "\"");
          if (!a.say.text || !a.say.text.trim()) err(q, label + ": say text is empty");
          if (a.say.delay != null && !(a.say.delay >= 0)) err(q, label + ": say delay must be >= 0");
        } else if ("open_exit" in a || "close_exit" in a) {
          const id = a.open_exit != null ? a.open_exit : a.close_exit;
          if (!exitIds.has(id)) err(q, label + ": exit \"" + id + "\" does not exist");
          if ("open_exit" in a) opensExit = true;
        } else if ("spawn" in a) {
          if (!a.spawn.length) warn(q, label + ": spawn wave is empty");
        } else if ("objective" in a) {
          if (!a.objective.trim()) warn(q, label + ": objective text is empty");
        } else if ("sfx" in a) {
          if (!a.sfx) err(q, label + ": sfx name is empty");
        } else err(q, label + ": unknown action");
      });
    });
    if (!opensExit && (f.scenario || []).length) warn("scenario", "no step opens an exit — runtime falls back to all_dead → open all exits");
    if ((f.exits || []).length && !(f.exits || []).some((e) => e.open) && !opensExit && !(f.scenario || []).length) warn("exits", "no exit starts open and there is no scenario (all_dead → open all exits)");
    return { errors, warnings };
  }

  return {
    SPEAKERS, SPEAKER_COLORS, SPEAKER_TAGS, SPAWN_TYPES, SPAWN_LETTER, SPAWN_COLORS, WEAPONS,
    TRIGGER_KINDS, ACTION_KINDS, SFX_NAMES, MAX_FLOOR, ORDER,
    blankFloor, normalize, canonical, stringify, validate, fileNameFor, floorLabel, pad2,
  };
});
