/* ============================================================
   OPEN MIAMI // LEVEL + SCENARIO EDITOR  (vanilla JS, no deps)
   Reads/writes levels/floor_NN.json + index.json per
   docs/SCENARIO_FORMAT.md. Pure format logic lives in
   levels-editor-format.js (window.LevelFormat).
   ============================================================ */
(function () {
  "use strict";
  const F = window.LevelFormat;
  const $ = (s) => document.querySelector(s);
  const el = (tag, attrs, ...kids) => {
    const e = document.createElement(tag);
    if (attrs) for (const k in attrs) {
      if (k === "class") e.className = attrs[k];
      else if (k === "style") e.style.cssText = attrs[k];
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), attrs[k]);
      else if (k === "value") e.value = attrs[k];
      else if (k === "checked") e.checked = !!attrs[k];
      else e.setAttribute(k, attrs[k]);
    }
    for (const k of kids) if (k != null) e.appendChild(typeof k === "string" ? document.createTextNode(k) : k);
    return e;
  };
  const opt = (v, label, sel) => { const o = el("option", { value: v }, label == null ? v : label); if (sel) o.selected = true; return o; };
  const clone = (o) => JSON.parse(JSON.stringify(o));

  /* ---------------- state ---------------- */
  const qp = new URLSearchParams(location.search);
  const S = {
    dir: "levels/" + (qp.get("dir") ? qp.get("dir").replace(/^\/+|\/+$/g, "") + "/" : ""), // repo-root relative (served at /levels/)
    list: [],            // [{file,id,name,floor|null,dirty,savedText}]
    cur: null,           // entry in list
    sel: null,           // {kind:'wall'|'room'|'zone'|'spawn'|'pickup'|'exit'|'entry', i}
    tool: "select",
    snap: true, grid: 20,
    view: { s: 0.6, ox: 30, oy: 30 },
    undo: [], redo: [],
    drag: null, hover: null, mouse: null,
    space: false,
    pendingSnap: null,
    lastValidation: { errors: [], warnings: [] },
    fontReady: false,
  };
  const KNOWN_IDS = () => new Set(S.list.map((e) => e.id).filter((n) => Number.isInteger(n)));
  const floor = () => (S.cur ? S.cur.floor : null);

  const status = (msg, cls) => { const st = $("#status"); st.textContent = msg; st.className = cls || ""; };

  /* ---------------- history ---------------- */
  function snapshot() { return JSON.stringify({ floor: floor(), sel: S.sel }); }
  function pushUndo(snap) {
    if (!snap) return;
    const now = snapshot();
    if (snap === now) return;
    S.undo.push(snap); if (S.undo.length > 200) S.undo.shift();
    S.redo.length = 0;
    updateUndoButtons();
  }
  function restore(snap) {
    const o = JSON.parse(snap);
    S.cur.floor = o.floor; S.sel = o.sel;
    markDirty(); renderAll();
  }
  function undo() { if (!S.undo.length || !S.cur) return; S.redo.push(snapshot()); restore(S.undo.pop()); updateUndoButtons(); }
  function redo() { if (!S.redo.length || !S.cur) return; S.undo.push(snapshot()); restore(S.redo.pop()); updateUndoButtons(); }
  function updateUndoButtons() { $("#btn-undo").disabled = !S.undo.length; $("#btn-redo").disabled = !S.redo.length; }
  /* mutate(fn): snapshot → fn(floor) → dirty + re-render */
  function mutate(fn, opts) {
    if (!S.cur) return;
    const snap = snapshot();
    fn(S.cur.floor);
    pushUndo(snap);
    markDirty();
    if (!(opts && opts.quiet)) renderAll(opts);
  }
  function markDirty() {
    if (!S.cur) return;
    S.cur.dirty = F.stringify(S.cur.floor) !== S.cur.savedText;
    renderFloorSel();
    validateLive();
  }

  /* ---------------- font ---------------- */
  async function loadFont() {
    try {
      // The game font, served from assets/ (same file index.html uses); loaded via
      // the JS FontFace API because CSS @font-face fails in headless Chromium.
      const ff = new FontFace("VT323", "url(" + new URL("../assets/fonts/VT323-Regular.ttf", location.href).href + ")");
      await ff.load(); document.fonts.add(ff);
    } catch (e) { /* fallback metrics */ }
    try { if (document.fonts && document.fonts.ready) await document.fonts.ready; } catch (e) { /* ignore */ }
    S.fontReady = true;
  }

  /* ---------------- index / loading ---------------- */
  function parseIndex(idx) {
    let arr = Array.isArray(idx) ? idx : (idx && Array.isArray(idx.floors) ? idx.floors : null);
    if (!arr) return null;
    return arr.map((e) => {
      if (typeof e === "string") return { file: e, id: idFromFile(e), name: null };
      if (e && typeof e === "object") return { file: e.file || (e.id != null ? F.fileNameFor(e.id) : null), id: Number.isInteger(e.id) ? e.id : idFromFile(e.file || ""), name: e.name || null };
      return null;
    }).filter((e) => e && e.file);
  }
  function idFromFile(f) {
    const m = /floor_(\d+)(h?)\.json$/.exec(f || "");
    if (!m) return null;
    return m[2] ? 14 : parseInt(m[1], 10);
  }
  async function fetchJSON(path) {
    // data paths are repo-root relative ("levels/…"); the page lives in tools/
    const r = await fetch("/" + path.replace(/^\/+/, "") + "?t=" + Date.now(), { cache: "no-store" });
    if (!r.ok) throw new Error(r.status + " " + path);
    return r.json();
  }
  async function loadIndex(keepLoaded) {
    let entries = null, dir = S.dir;
    try { entries = parseIndex(await fetchJSON(dir + "index.json")); } catch (e) { entries = null; }
    if (!entries && !qp.get("dir")) {
      // no levels/index.json yet → fall back to the shipped samples (if any)
      try { entries = parseIndex(await fetchJSON("levels/samples/index.json")); if (entries) { dir = "levels/samples/"; } } catch (e) { entries = null; }
    }
    S.dir = dir;
    const old = new Map(S.list.map((e) => [e.file, e]));
    const list = (entries || []).map((e) => {
      const o = keepLoaded && old.get(e.file);
      if (o) { o.id = e.id != null ? e.id : o.id; o.name = e.name || o.name; return o; }
      return { file: e.file, id: e.id, name: e.name, floor: null, dirty: false, savedText: null };
    });
    // keep unsaved / imported entries that aren't in the index
    if (keepLoaded) for (const e of S.list) if (!list.some((x) => x.file === e.file) && (e.dirty || e.imported)) list.push(e);
    S.list = list;
    if (S.cur && !S.list.includes(S.cur)) S.cur = S.list.find((e) => e.file === S.cur.file) || null;
    renderFloorSel();
    renderDirSel();
    return entries != null;
  }
  async function selectEntry(entry) {
    if (!entry) return;
    if (!entry.floor) {
      try {
        const raw = await fetchJSON(S.dir + entry.file);
        entry.floor = F.normalize(raw);
        entry.savedText = F.stringify(entry.floor);
        entry.rawText = JSON.stringify(raw);
        entry.id = entry.floor.id; entry.name = entry.floor.name;
        entry.dirty = false;
      } catch (e) {
        status("LOAD FAILED: " + entry.file, "bad");
        return;
      }
    }
    S.cur = entry; S.sel = null; S.undo.length = 0; S.redo.length = 0; updateUndoButtons();
    fitView();
    renderAll();
    status("LOADED " + S.dir + entry.file, "ok");
    const u = new URL(location.href); u.searchParams.set("floor", entry.id); history.replaceState(null, "", u);
  }
  function newFloor() {
    const ids = [...KNOWN_IDS()];
    let id = 1; while (ids.includes(id) && id < F.MAX_FLOOR) id++;
    const fl = F.blankFloor(id);
    const entry = { file: F.fileNameFor(id), id, name: fl.name, floor: fl, dirty: true, savedText: null, imported: true };
    S.list.push(entry);
    S.list.sort((a, b) => (a.id || 99) - (b.id || 99));
    S.cur = entry; S.sel = null; S.undo.length = 0; S.redo.length = 0; updateUndoButtons();
    fitView(); renderAll();
    status("NEW FLOOR " + entry.file + " (unsaved)", "");
  }

  /* ---------------- persistence ---------------- */
  function currentText() { return F.stringify(floor()); }
  async function save() {
    if (!S.cur) return false;
    const v = F.validate(floor(), { knownIds: KNOWN_IDS() });
    renderErrors(v);
    if (v.errors.length) { status("SAVE BLOCKED: " + v.errors.length + " error" + (v.errors.length > 1 ? "s" : ""), "bad"); return false; }
    const text = currentText();
    const path = "/" + S.dir + S.cur.file;
    status("SAVING…", "");
    try {
      // Writes need the shared secret (serve.py prints it at startup as
      // "editor write token"). Ask once, remember it in localStorage, and
      // re-ask if the server says it's wrong.
      const put = async () => fetch(path, {
        method: "PUT",
        headers: { "Content-Type": "application/json", "X-Editor-Token": localStorage.getItem("editorToken") || "" },
        body: text,
      });
      let r = await put();
      if (r.status === 401) {
        const tok = window.prompt("Editor write token (printed by serve.py at startup):", localStorage.getItem("editorToken") || "");
        if (tok == null) throw new Error("no token — use DOWNLOAD");
        localStorage.setItem("editorToken", tok.trim());
        r = await put();
      }
      const j = await r.json().catch(() => ({}));
      if (!r.ok || !j.ok) throw new Error(j.error || (r.status + ""));
      S.cur.savedText = text; S.cur.dirty = false; S.cur.imported = false;
      S.cur.id = floor().id; S.cur.name = floor().name;
      status("SAVED " + j.path + " (" + j.bytes + " bytes)", "ok");
      await loadIndex(true);
      if (S.cur && !S.list.includes(S.cur)) S.cur = S.list.find((e) => e.file === S.cur.file) || S.cur;
      renderAll();
      return true;
    } catch (e) {
      status("SAVE FAILED: " + e.message + " — use DOWNLOAD", "bad");
      return false;
    }
  }
  function download() {
    if (!S.cur) return;
    const blob = new Blob([currentText()], { type: "application/json" });
    const a = el("a", { href: URL.createObjectURL(blob), download: S.cur.file });
    document.body.appendChild(a); a.click(); a.remove();
    status("DOWNLOADED " + S.cur.file, "ok");
  }
  function importFile(file) {
    const rd = new FileReader();
    rd.onload = () => {
      try {
        const raw = JSON.parse(rd.result);
        const fl = F.normalize(raw);
        const fname = F.fileNameFor(fl.id);
        let entry = S.list.find((e) => e.file === fname);
        if (!entry) { entry = { file: fname, id: fl.id, name: fl.name, floor: null, dirty: false, savedText: null }; S.list.push(entry); S.list.sort((a, b) => (a.id || 99) - (b.id || 99)); }
        entry.floor = fl; entry.name = fl.name; entry.id = fl.id; entry.imported = true;
        entry.dirty = F.stringify(fl) !== entry.savedText;
        S.cur = entry; S.sel = null; S.undo.length = 0; S.redo.length = 0; updateUndoButtons();
        fitView(); renderAll();
        status("IMPORTED " + file.name + " → " + fname, "ok");
      } catch (e) { status("IMPORT FAILED: " + e.message, "bad"); }
    };
    rd.readAsText(file);
  }
  function play() { if (S.cur) window.open("/?floor=" + floor().id, "_blank"); }

  /* ---------------- validation ---------------- */
  function validateLive() {
    if (!S.cur) { renderErrors({ errors: [], warnings: [] }); return; }
    const v = F.validate(floor(), { knownIds: KNOWN_IDS() });
    renderErrors(v);
    if (!v.errors.length && /^SAVE BLOCKED/.test($("#status").textContent)) status("errors fixed — ready to save", "");
    // flag step cards
    document.querySelectorAll("#stepsbody .step").forEach((card, i) => {
      card.classList.toggle("err", v.errors.some((e) => e.path.startsWith("scenario[" + i + "]")));
    });
  }
  function renderErrors(v) {
    S.lastValidation = v;
    const box = $("#errors");
    box.innerHTML = "";
    for (const e of v.errors) box.appendChild(el("div", { class: "e", title: e.path }, e.msg));
    for (const w of v.warnings) box.appendChild(el("div", { class: "w", title: w.path }, w.msg));
    box.classList.toggle("show", !!(v.errors.length || v.warnings.length));
    $("#btn-save").classList.toggle("danger", !!v.errors.length);
  }

  /* ---------------- floor selector ---------------- */
  function renderFloorSel() {
    const sel = $("#floorsel");
    sel.innerHTML = "";
    for (const e of S.list) {
      const b = el("button", { class: (e === S.cur ? "on " : "") + (e.dirty ? "dirty" : ""), onclick: () => selectEntry(e) },
        el("span", { class: "fn" }, e.id != null ? F.floorLabel(e.id) : "??"), "  " + (e.name || e.file));
      b.dataset.file = e.file;
      sel.appendChild(b);
    }
    sel.appendChild(el("button", { id: "btn-new", onclick: newFloor, title: "create a blank floor" }, "+ NEW FLOOR"));
    sel.appendChild(el("span", { class: "dirsel", id: "dirsel" }));
    renderDirSel();
  }
  function renderDirSel() {
    const d = $("#dirsel"); if (!d) return;
    d.innerHTML = "";
    const lab = el("label", { class: "f" }, "DIR ");
    const s = el("select", { onchange: () => { const u = new URL(location.href); if (s.value === "levels/") u.searchParams.delete("dir"); else u.searchParams.set("dir", "samples"); u.searchParams.delete("floor"); location.href = u.toString(); } });
    s.appendChild(opt("levels/", "levels/", S.dir === "levels/"));
    s.appendChild(opt("levels/samples/", "levels/samples/", S.dir === "levels/samples/"));
    lab.appendChild(s); d.appendChild(lab);
  }

  /* ---------------- meta form ---------------- */
  function bindText(input, get, set, opts) {
    // live model update on input, one undo entry per focus session
    input.value = get();
    input.addEventListener("focus", () => { S.pendingSnap = snapshot(); });
    input.addEventListener("input", () => { if (!S.cur) return; set(input.value); markDirty(); renderPreview(); if (opts && opts.onInput) opts.onInput(); });
    input.addEventListener("change", () => { pushUndo(S.pendingSnap); S.pendingSnap = null; if (opts && opts.onChange) opts.onChange(); });
    return input;
  }
  function renderMeta() {
    const body = $("#metabody"); body.innerHTML = "";
    const f = floor();
    $("#metafile").textContent = S.cur ? S.dir + S.cur.file : "—";
    if (!f) { body.appendChild(el("span", { class: "none", style: "grid-column:1/-1" }, "no floor loaded — pick one above or + NEW FLOOR")); return; }
    const row = (label, ...inputs) => { body.appendChild(el("label", null, label)); const w = el("div", { class: "row wide" }, ...inputs); body.appendChild(w); return w; };
    // id + name
    body.appendChild(el("label", null, "ID"));
    const idIn = el("input", { type: "number", min: 1, max: F.MAX_FLOOR, value: f.id, id: "m-id" });
    idIn.addEventListener("change", () => {
      const v = parseInt(idIn.value, 10);
      if (!Number.isInteger(v)) return;
      mutate((fl) => { fl.id = v; S.cur.id = v; S.cur.file = F.fileNameFor(v); });
    });
    body.appendChild(el("div", { class: "row" }, idIn, el("span", { style: "color:var(--dim);font-size:15px" }, "→ " + S.cur.file)));
    body.appendChild(el("label", null, "NAME"));
    body.appendChild(bindText(el("input", { type: "text", id: "m-name" }), () => f.name, (v) => { f.name = v; S.cur.name = v; }, { onInput: renderFloorSel }));
    row("THEME", bindText(el("input", { type: "text", id: "m-theme", style: "flex:1" }), () => f.theme, (v) => { f.theme = v; }));
    // accent
    body.appendChild(el("label", null, "ACCENT"));
    const col = el("input", { type: "color", value: /^#[0-9a-fA-F]{6}$/.test(f.accent) ? f.accent : "#37f0e6" });
    const colTxt = bindText(el("input", { type: "text", id: "m-accent", style: "width:90px" }), () => f.accent, (v) => { f.accent = v; applyAccent(); if (/^#[0-9a-fA-F]{6}$/.test(v)) col.value = v; });
    col.addEventListener("input", () => { colTxt.value = col.value; f.accent = col.value; applyAccent(); markDirty(); });
    col.addEventListener("focus", () => { S.pendingSnap = snapshot(); });
    col.addEventListener("change", () => { pushUndo(S.pendingSnap); S.pendingSnap = null; });
    body.appendChild(el("div", { class: "row" }, col, colTxt));
    body.appendChild(el("label", null, "SIZE"));
    const sw = el("input", { type: "number", min: 100, value: f.size.w }), sh = el("input", { type: "number", min: 100, value: f.size.h });
    const szc = () => { const w = parseInt(sw.value, 10), h = parseInt(sh.value, 10); if (w > 0 && h > 0) mutate((fl) => { fl.size.w = w; fl.size.h = h; }); };
    sw.addEventListener("change", szc); sh.addEventListener("change", szc);
    body.appendChild(el("div", { class: "row" }, sw, "×", sh));
    row("FLAVOR", bindText(el("textarea", { id: "m-flavor" }), () => f.flavor, (v) => { f.flavor = v; }));
    row("OBJECTIVE", bindText(el("textarea", { id: "m-objective", style: "min-height:34px" }), () => f.objective, (v) => { f.objective = v; }));
  }
  function applyAccent() {
    const f = floor(); const a = f && /^#[0-9a-fA-F]{6}$/.test(f.accent) ? f.accent : "#37f0e6";
    document.documentElement.style.setProperty("--accent", a);
  }

  /* ---------------- selection helpers ---------------- */
  function selItem() {
    const f = floor(); if (!f || !S.sel) return null;
    const k = S.sel.kind;
    if (k === "entry") return f.entry;
    const arr = f[k + "s"]; return arr ? arr[S.sel.i] : null;
  }
  function itemArrayName(kind) { return kind + "s"; }
  function deleteSelection() {
    if (!S.sel || S.sel.kind === "entry") return;
    const kind = S.sel.kind, i = S.sel.i;
    mutate((f) => { f[itemArrayName(kind)].splice(i, 1); S.sel = null; });
  }
  function duplicateSelection() {
    if (!S.sel || S.sel.kind === "entry") return;
    const kind = S.sel.kind, i = S.sel.i;
    mutate((f) => {
      const arr = f[itemArrayName(kind)]; const c = clone(arr[i]);
      c.x += S.grid; c.y += S.grid;
      if (c.id != null) c.id = uniqueId(arr, c.id);
      arr.push(c); S.sel = { kind, i: arr.length - 1 };
    });
  }
  function uniqueId(arr, base) {
    let b = String(base).replace(/_\d+$/, ""), n = 2, id = b + "_" + n;
    const has = (x) => arr.some((o) => o.id === x);
    if (!has(b)) return b;
    while (has(id)) id = b + "_" + (++n);
    return id;
  }
  function nudge(dx, dy) {
    const it = selItem(); if (!it) return;
    mutate((f) => { const s = selItem(); s.x += dx; s.y += dy; }, { canvasOnly: true });
    renderProps();
  }

  /* ---------------- properties form ---------------- */
  function numField(label, obj, key, step) {
    const inp = el("input", { type: "number", value: obj[key], step: step || 1 });
    inp.addEventListener("focus", () => { S.pendingSnap = snapshot(); });
    inp.addEventListener("input", () => { const v = Number(inp.value); if (Number.isFinite(v)) { selItem()[key] = v; markDirty(); } });
    inp.addEventListener("change", () => { pushUndo(S.pendingSnap); S.pendingSnap = null; });
    return el("label", { class: "f" }, label, inp);
  }
  function textField(label, obj, key, opts) {
    const inp = el("input", Object.assign({ type: "text", value: obj[key] }, opts && opts.attrs || {}));
    inp.addEventListener("focus", () => { S.pendingSnap = snapshot(); });
    inp.addEventListener("input", () => { selItem()[key] = inp.value; markDirty(); renderSteps(); renderPreview(); });
    inp.addEventListener("change", () => { pushUndo(S.pendingSnap); S.pendingSnap = null; });
    return el("label", { class: "f" }, label, inp);
  }
  function selectField(label, obj, key, options, labels) {
    const s = el("select");
    options.forEach((o, i) => s.appendChild(opt(o, labels ? labels[i] : o, obj[key] === o)));
    s.addEventListener("change", () => { mutate((f) => { selItem()[key] = s.value; }); });
    return el("label", { class: "f" }, label, s);
  }
  function renderProps() {
    const body = $("#propbody"); body.innerHTML = "";
    const it = selItem();
    $("#propkind").textContent = S.sel ? S.sel.kind.toUpperCase() + (S.sel.kind !== "entry" ? " #" + S.sel.i : "") : "—";
    if (!it) {
      const f = floor();
      body.appendChild(el("span", { class: "none" }, f
        ? `nothing selected — ${f.walls.length} walls · ${f.rooms.length} rooms · ${f.zones.length} zones · ${f.spawns.length} spawns · ${f.pickups.length} pickups · ${f.exits.length} exits`
        : "no floor loaded"));
      return;
    }
    const k = S.sel.kind;
    body.appendChild(el("span", { class: "kind" }, k.toUpperCase()));
    if (k === "room" || k === "zone" || k === "exit") body.appendChild(textField("id", it, "id", { attrs: { style: "width:110px" } }));
    if (k === "room" || k === "exit" || k === "entry") body.appendChild(textField("label", it, "label"));
    body.appendChild(numField("x", it, "x")); body.appendChild(numField("y", it, "y"));
    if (k !== "spawn" && k !== "pickup") { body.appendChild(numField("w", it, "w")); body.appendChild(numField("h", it, "h")); }
    if (k === "spawn") body.appendChild(selectField("type", it, "type", F.SPAWN_TYPES, ["idle (SENTINEL)", "wandering (DRIFTER)", "patrolling (HUNTER)"]));
    if (k === "pickup") body.appendChild(selectField("weapon", it, "weapon", F.WEAPONS));
    if (k === "exit") {
      const to = el("input", { type: "number", min: 0, max: F.MAX_FLOOR, value: it.to, style: "width:56px", title: "next floor id (0 = end of run)" });
      to.addEventListener("change", () => { const v = parseInt(to.value, 10); if (Number.isInteger(v)) mutate((f) => { selItem().to = v; }); });
      body.appendChild(el("label", { class: "f" }, "to floor", to));
      const op = el("input", { type: "checkbox", checked: it.open });
      op.addEventListener("change", () => mutate((f) => { selItem().open = op.checked; }));
      body.appendChild(el("label", { class: "f" }, op, "starts open"));
    }
    if (k !== "entry") {
      body.appendChild(el("button", { class: "mini", onclick: duplicateSelection, title: "Ctrl+D" }, "DUPLICATE"));
      body.appendChild(el("button", { class: "mini danger", onclick: deleteSelection, title: "Del" }, "DELETE"));
    }
  }

  /* ---------------- scenario steps editor ---------------- */
  const TRIGGER_LABEL = { start: "on floor start", enter_zone: "player enters zone", kills: "kills ≥ count", all_dead: "all rogues dead", timer: "timer (s)", exit_open: "an exit opened", step_done: "after step", boss_dead: "the boss is dead", extracted: "player extracted" };
  function idOptions(sel, ids, current, allowEmpty, emptyLabel) {
    if (allowEmpty) sel.appendChild(opt("", emptyLabel || "(any)", !current));
    let found = false;
    for (const id of ids) { sel.appendChild(opt(id, id, id === current)); if (id === current) found = true; }
    if (current && !found) sel.appendChild(opt(current, current + " (missing!)", true));
    return sel;
  }
  function renderSteps() {
    const body = $("#stepsbody"); body.innerHTML = "";
    const f = floor();
    $("#stepcount").textContent = f ? f.scenario.length + " STEP" + (f.scenario.length === 1 ? "" : "S") : "—";
    if (!f) return;
    const zoneIds = f.zones.map((z) => z.id), exitIds = f.exits.map((e) => e.id);
    const stepIds = f.scenario.map((s) => s.id).filter(Boolean);
    f.scenario.forEach((st, i) => {
      const card = el("div", { class: "step" }); card.dataset.i = i;
      // ---- header: id + trigger
      const hd = el("div", { class: "hd" }, el("span", { class: "n" }, "#" + (i + 1)));
      const idIn = el("input", { type: "text", class: "sid", placeholder: "step id", value: st.id || "" });
      idIn.addEventListener("focus", () => { S.pendingSnap = snapshot(); });
      idIn.addEventListener("input", () => { if (idIn.value) st.id = idIn.value; else delete st.id; markDirty(); renderPreview(); });
      idIn.addEventListener("change", () => { pushUndo(S.pendingSnap); S.pendingSnap = null; renderSteps(); });
      hd.appendChild(idIn);
      hd.appendChild(el("span", null, "when"));
      const kind = el("select", { class: "kind" });
      for (const k of Object.keys(F.TRIGGER_KINDS)) kind.appendChild(opt(k, TRIGGER_LABEL[k], st.trigger.kind === k));
      kind.addEventListener("change", () => mutate((fl) => {
        const t = { kind: kind.value };
        if (kind.value === "enter_zone") t.zone = zoneIds[0] || "";
        if (kind.value === "kills") t.count = 1;
        if (kind.value === "timer") t.seconds = 5;
        if (kind.value === "step_done") t.step = stepIds.find((x) => x !== st.id) || "";
        fl.scenario[i].trigger = t;
      }));
      hd.appendChild(kind);
      const t = st.trigger;
      if (t.kind === "enter_zone") {
        const z = idOptions(el("select"), zoneIds, t.zone, false);
        z.addEventListener("change", () => mutate((fl) => { fl.scenario[i].trigger.zone = z.value; }));
        hd.appendChild(z);
      } else if (t.kind === "kills") {
        const c = el("input", { type: "number", min: 1, value: t.count, style: "width:56px" });
        c.addEventListener("change", () => mutate((fl) => { fl.scenario[i].trigger.count = parseInt(c.value, 10) || 1; }));
        hd.appendChild(c);
      } else if (t.kind === "timer") {
        const sec = el("input", { type: "number", min: 0, step: 0.5, value: t.seconds, style: "width:64px" });
        sec.addEventListener("change", () => mutate((fl) => { fl.scenario[i].trigger.seconds = Number(sec.value) || 0; }));
        hd.appendChild(sec); hd.appendChild(el("span", null, "s after"));
        const af = idOptions(el("select"), stepIds.filter((x) => x !== st.id), t.after, true, "(floor start)");
        af.addEventListener("change", () => mutate((fl) => { if (af.value) fl.scenario[i].trigger.after = af.value; else delete fl.scenario[i].trigger.after; }));
        hd.appendChild(af);
      } else if (t.kind === "exit_open") {
        const ex = idOptions(el("select"), exitIds, t.exit, true, "(any exit)");
        ex.addEventListener("change", () => mutate((fl) => { if (ex.value) fl.scenario[i].trigger.exit = ex.value; else delete fl.scenario[i].trigger.exit; }));
        hd.appendChild(ex);
      } else if (t.kind === "step_done") {
        const sd = idOptions(el("select"), stepIds.filter((x) => x !== st.id), t.step, false);
        sd.addEventListener("change", () => mutate((fl) => { fl.scenario[i].trigger.step = sd.value; }));
        hd.appendChild(sd);
      }
      hd.appendChild(el("span", { class: "sp" }));
      hd.appendChild(el("button", { class: "mini", title: "move up", onclick: () => i > 0 && mutate((fl) => { const a = fl.scenario; [a[i - 1], a[i]] = [a[i], a[i - 1]]; }) }, "^"));
      hd.appendChild(el("button", { class: "mini", title: "move down", onclick: () => i < f.scenario.length - 1 && mutate((fl) => { const a = fl.scenario; [a[i + 1], a[i]] = [a[i], a[i + 1]]; }) }, "v"));
      hd.appendChild(el("button", { class: "mini danger", title: "remove step", onclick: () => mutate((fl) => { fl.scenario.splice(i, 1); }) }, "×"));
      card.appendChild(hd);
      // ---- actions
      const acts = el("div", { class: "acts" });
      st.actions.forEach((a, j) => acts.appendChild(renderAction(f, i, j, a, exitIds)));
      card.appendChild(acts);
      const ft = el("div", { class: "ft" });
      const addSel = el("select");
      addSel.appendChild(opt("", "+ action…"));
      for (const k of F.ACTION_KINDS) addSel.appendChild(opt(k, k));
      addSel.addEventListener("change", () => {
        const k = addSel.value; if (!k) return;
        mutate((fl) => { fl.scenario[i].actions.push(defaultAction(k, fl)); });
      });
      ft.appendChild(addSel);
      card.appendChild(ft);
      body.appendChild(card);
    });
    body.appendChild(el("div", { style: "display:flex;gap:8px" },
      el("button", { id: "btn-add-step", onclick: () => mutate((fl) => { fl.scenario.push({ id: uniqueStepId(fl, "step"), trigger: { kind: "start" }, actions: [] }); }) }, "+ STEP")));
    validateLive();
  }
  function uniqueStepId(fl, base) {
    let n = fl.scenario.length + 1, id = base + "_" + n;
    while (fl.scenario.some((s) => s.id === id)) id = base + "_" + (++n);
    return id;
  }
  function defaultAction(k, fl) {
    switch (k) {
      case "say": return { say: { who: "CL4-UD3", text: "" } };
      case "spawn": return { spawn: [{ x: Math.round(fl.size.w / 2), y: Math.round(fl.size.h / 2), type: "patrolling" }] };
      case "open_exit": return { open_exit: (fl.exits[0] && fl.exits[0].id) || "" };
      case "close_exit": return { close_exit: (fl.exits[0] && fl.exits[0].id) || "" };
      case "objective": return { objective: "" };
      case "sfx": return { sfx: "elevator" };
    }
    return { objective: "" };
  }
  function actionKind(a) { return F.ACTION_KINDS.find((k) => k in a) || "objective"; }
  function renderAction(f, i, j, a, exitIds) {
    const row = el("div", { class: "act" });
    const kind = actionKind(a);
    const ks = el("select", { class: "kind" });
    for (const k of F.ACTION_KINDS) ks.appendChild(opt(k, k, k === kind));
    ks.addEventListener("change", () => mutate((fl) => { fl.scenario[i].actions[j] = defaultAction(ks.value, fl); }));
    row.appendChild(ks);
    const live = (inp, apply) => {
      inp.addEventListener("focus", () => { S.pendingSnap = snapshot(); });
      inp.addEventListener("input", () => { apply(floor().scenario[i].actions[j]); markDirty(); renderPreview(); });
      inp.addEventListener("change", () => { pushUndo(S.pendingSnap); S.pendingSnap = null; });
      return inp;
    };
    if (kind === "say") {
      const who = el("select", { class: "who who-" + a.say.who });
      for (const w of F.SPEAKERS) who.appendChild(opt(w, w, a.say.who === w));
      if (!F.SPEAKERS.includes(a.say.who)) who.appendChild(opt(a.say.who, a.say.who + " (?)", true));
      who.addEventListener("change", () => mutate((fl) => { fl.scenario[i].actions[j].say.who = who.value; }));
      row.appendChild(who);
      row.appendChild(live(el("input", { type: "text", class: "txt", placeholder: "line…", value: a.say.text }), (x) => { x.say.text = row.querySelector(".txt").value; }));
      row.appendChild(el("span", null, "+"));
      row.appendChild(live(el("input", { type: "number", class: "dly", min: 0, step: 0.1, placeholder: "0", value: a.say.delay != null ? a.say.delay : "" }), (x) => {
        const v = row.querySelector(".dly").value; if (v === "" || !Number.isFinite(Number(v))) delete x.say.delay; else x.say.delay = Number(v);
      }));
      row.appendChild(el("span", null, "s"));
    } else if (kind === "open_exit" || kind === "close_exit") {
      const ex = idOptions(el("select"), exitIds, a[kind], false);
      ex.addEventListener("change", () => mutate((fl) => { fl.scenario[i].actions[j][kind] = ex.value; }));
      row.appendChild(ex);
    } else if (kind === "objective") {
      row.appendChild(live(el("input", { type: "text", class: "txt", placeholder: "new objective text", value: a.objective }), (x) => { x.objective = row.querySelector(".txt").value; }));
    } else if (kind === "sfx") {
      const inp = live(el("input", { type: "text", class: "txt", list: "sfx-names", placeholder: "sfx name", value: a.sfx, style: "min-width:120px;flex:0 1 160px" }), (x) => { x.sfx = row.querySelector(".txt").value; });
      row.appendChild(inp);
      if (!$("#sfx-names")) { const dl = el("datalist", { id: "sfx-names" }); for (const n of F.SFX_NAMES) dl.appendChild(opt(n)); document.body.appendChild(dl); }
    } else if (kind === "spawn") {
      const wave = el("div", { class: "wave" });
      a.spawn.forEach((sp, k) => {
        const r = el("div", { class: "sp" });
        const x = el("input", { type: "number", value: sp.x }), y = el("input", { type: "number", value: sp.y });
        const ty = el("select"); for (const t of F.SPAWN_TYPES) ty.appendChild(opt(t, t, sp.type === t));
        const upd = () => mutate((fl) => { const s = fl.scenario[i].actions[j].spawn[k]; s.x = Number(x.value) || 0; s.y = Number(y.value) || 0; s.type = ty.value; }, { canvasOnly: true });
        x.addEventListener("change", upd); y.addEventListener("change", upd); ty.addEventListener("change", upd);
        r.appendChild(el("span", null, "x")); r.appendChild(x); r.appendChild(el("span", null, "y")); r.appendChild(y); r.appendChild(ty);
        r.appendChild(el("button", { class: "mini danger", title: "remove this spawn", onclick: () => mutate((fl) => { fl.scenario[i].actions[j].spawn.splice(k, 1); }) }, "×"));
        wave.appendChild(r);
      });
      wave.appendChild(el("div", { class: "sp" }, el("button", { class: "mini", onclick: () => mutate((fl) => { const w = fl.scenario[i].actions[j].spawn; const last = w[w.length - 1]; w.push(last ? { x: last.x + S.grid * 2, y: last.y, type: last.type } : { x: 100, y: 100, type: "patrolling" }); }) }, "+ rogue"),
        el("span", { style: "font-size:14px" }, "(ghost diamonds on the map)")));
      row.appendChild(wave);
    }
    row.appendChild(el("button", { class: "mini danger", title: "remove action", onclick: () => mutate((fl) => { fl.scenario[i].actions.splice(j, 1); }) }, "×"));
    return row;
  }

  /* ---------------- comms preview (mockup look) ---------------- */
  const FACTION = { "CL4-UD3": "clyde", HUNTER: "hunter", SENTINEL: "sentinel", DRIFTER: "drifter", SWARM: "swarm", CORRUPTOR: "corrupt", UPLINK: "clyde" };
  const portraitCache = new Map();
  function portrait(who) {
    const key = who + "@44";
    if (!portraitCache.has(key)) portraitCache.set(key, makePortrait(FACTION[who] || "swarm", F.SPEAKER_COLORS[who] || "#fff", 44));
    return cloneCanvas(portraitCache.get(key));
  }
  function cloneCanvas(c) { const n = document.createElement("canvas"); n.width = c.width; n.height = c.height; n.getContext("2d").drawImage(c, 0, 0); return n; }
  function makePortrait(faction, col, size) {
    const c = document.createElement("canvas"); c.width = c.height = size;
    const g = c.getContext("2d"); const s = size;
    g.fillStyle = "#05040a"; g.fillRect(0, 0, s, s);
    g.strokeStyle = "rgba(255,255,255,.05)";
    for (let i = 4; i < s; i += 4) { g.beginPath(); g.moveTo(0, i); g.lineTo(s, i); g.stroke(); }
    g.save(); g.translate(s / 2, s / 2);
    g.shadowColor = col; g.shadowBlur = 10; g.strokeStyle = col; g.fillStyle = col; g.lineWidth = Math.max(1.5, s * 0.045);
    const r = s * 0.30;
    g.beginPath(); g.moveTo(0, -r); g.lineTo(0, -r - s * 0.14); g.stroke();
    g.beginPath(); g.arc(0, -r - s * 0.16, s * 0.035, 0, 7); g.fill();
    const hw = r, hh = r * 1.05, rad = s * 0.06;
    g.beginPath(); g.moveTo(-hw + rad, -hh); g.arcTo(hw, -hh, hw, hh, rad); g.arcTo(hw, hh, -hw, hh, rad); g.arcTo(-hw, hh, -hw, -hh, rad); g.arcTo(-hw, -hh, hw, -hh, rad); g.closePath();
    g.globalAlpha = .14; g.fill(); g.globalAlpha = 1; g.stroke();
    g.shadowBlur = 12; const vy = -s * 0.02;
    if (faction === "clyde") {
      g.lineWidth = s * 0.05; g.beginPath(); g.moveTo(-hw * 0.66, vy); g.lineTo(hw * 0.66, vy); g.stroke();
      g.beginPath(); g.arc(0, vy, s * 0.03, 0, 7); g.fill();
    } else if (faction === "sentinel") {
      g.lineWidth = s * 0.06; g.beginPath(); g.moveTo(-hw * 0.7, vy - hh * 0.28); g.lineTo(0, vy + hh * 0.05); g.lineTo(hw * 0.7, vy - hh * 0.28); g.stroke();
    } else if (faction === "hunter") {
      g.lineWidth = s * 0.045; g.beginPath(); g.arc(0, vy, hw * 0.42, 0, 7); g.stroke();
      g.beginPath(); g.moveTo(-hw * 0.7, vy); g.lineTo(hw * 0.7, vy); g.moveTo(0, vy - hh * 0.42); g.lineTo(0, vy + hh * 0.42); g.stroke();
      g.beginPath(); g.arc(0, vy, s * 0.022, 0, 7); g.fill();
    } else if (faction === "drifter") {
      g.lineWidth = s * 0.045;
      g.beginPath(); g.moveTo(-hw * 0.66, vy - hh * 0.12); g.lineTo(-hw * 0.1, vy + hh * 0.06); g.stroke();
      g.beginPath(); g.moveTo(hw * 0.12, vy - hh * 0.05); g.lineTo(hw * 0.6, vy + hh * 0.14); g.stroke();
      g.globalAlpha = .6; g.beginPath(); g.arc(hw * 0.35, vy - hh * 0.2, s * 0.02, 0, 7); g.fill(); g.globalAlpha = 1;
    } else {
      g.lineWidth = s * 0.038;
      for (const dx of [-hw * 0.5, 0, hw * 0.5]) { g.beginPath(); g.arc(dx, vy - hh * 0.12, s * 0.03, 0, 7); g.fill(); }
      g.globalAlpha = .7; g.beginPath(); g.arc(0, vy + hh * 0.15, hw * 0.5, 0.15 * Math.PI, 0.85 * Math.PI); g.stroke(); g.globalAlpha = 1;
    }
    g.restore();
    return c;
  }
  function triggerDesc(t) {
    switch (t.kind) {
      case "start": return "on start";
      case "enter_zone": return "enter zone " + (t.zone || "?");
      case "kills": return "kills ≥ " + t.count;
      case "all_dead": return "all rogues dead";
      case "timer": return "t+" + t.seconds + "s" + (t.after ? " after " + t.after : "");
      case "exit_open": return "exit " + (t.exit || "(any)") + " opened";
      case "step_done": return "after step " + t.step;
    }
    return t.kind;
  }
  function renderPreview() {
    const root = $("#scenario"); root.innerHTML = "";
    const f = floor();
    if (!f) { root.appendChild(el("div", { class: "empty" }, "no floor loaded")); $("#depthno").textContent = "-00"; return; }
    $("#depthno").textContent = "-" + F.floorLabel(f.id);
    const head = el("div", { class: "head" },
      el("div", { class: "theme" }, f.theme || " "),
      el("div", { class: "name" }, "FLOOR " + F.floorLabel(f.id) + " — " + (f.name || "")),
      el("div", { class: "flavor" }, f.flavor || ""));
    root.appendChild(head);
    root.appendChild(el("div", { class: "obj" }, el("div", { class: "lbl" }, "» OBJECTIVE"), el("div", { class: "txt" }, f.objective || " ")));
    const comms = el("div", { class: "comms" });
    comms.appendChild(el("div", { class: "banner" }, el("span", null, "INTERCEPTED COMMS // FLOOR " + F.floorLabel(f.id)), el("span", { class: "live" }, "LOCAL RX")));
    if (!f.scenario.length) comms.appendChild(el("div", { class: "empty" }, "no scenario steps — the floor plays as all_dead → open all exits"));
    f.scenario.forEach((st, i) => {
      comms.appendChild(el("div", { class: "stephd" }, el("b", null, st.id || "#" + (i + 1)), el("i", null, triggerDesc(st.trigger))));
      for (const a of st.actions) {
        if (a.say) {
          const who = a.say.who, col = F.SPEAKER_COLORS[who] || "#fff";
          const row = el("div", { class: "msg" + (who === "CL4-UD3" ? " clyde" : "") + (who === "DRIFTER" ? " static" : "") + (who === "CORRUPTOR" || who === "SWARM" ? " corrupt" : "") });
          row.style.setProperty("--who", col);
          row.appendChild(portrait(who));
          const body = el("div", { class: "body" });
          const wh = el("div", { class: "who" }, who);
          if (F.SPEAKER_TAGS[who]) wh.appendChild(el("span", { class: "tag" }, "// " + F.SPEAKER_TAGS[who]));
          wh.appendChild(el("span", { class: "t" }, "+" + (a.say.delay || 0) + "s"));
          body.appendChild(wh);
          body.appendChild(el("div", { class: "line" }, a.say.text || "…"));
          row.appendChild(body); comms.appendChild(row);
        } else if ("spawn" in a) comms.appendChild(el("div", { class: "sys spawn" }, "SPAWN WAVE ×" + a.spawn.length + " " + a.spawn.map((s) => F.SPAWN_LETTER[s.type]).join("")));
        else if ("open_exit" in a) comms.appendChild(el("div", { class: "sys" }, "EXIT OPENED: " + a.open_exit));
        else if ("close_exit" in a) comms.appendChild(el("div", { class: "sys close" }, "EXIT CLOSED: " + a.close_exit));
        else if ("objective" in a) comms.appendChild(el("div", { class: "sys objv" }, "OBJECTIVE: " + a.objective));
        else if ("sfx" in a) comms.appendChild(el("div", { class: "sys sfx" }, "SFX " + a.sfx));
      }
    });
    root.appendChild(comms);
  }

  /* ---------------- canvas ---------------- */
  const map = $("#map"), mx = map.getContext("2d");
  const W = map.width, H = map.height;
  const w2s = (x, y) => [S.view.ox + x * S.view.s, S.view.oy + y * S.view.s];
  const s2w = (px, py) => [(px - S.view.ox) / S.view.s, (py - S.view.oy) / S.view.s];
  function fitView() {
    const f = floor(); if (!f) return;
    const s = Math.min((W - 60) / f.size.w, (H - 60) / f.size.h);
    S.view.s = s; S.view.ox = (W - f.size.w * s) / 2; S.view.oy = (H - f.size.h * s) / 2;
  }
  function snapv(v) { return S.snap ? Math.round(v / S.grid) * S.grid : Math.round(v); }
  function canvasPos(ev) { const r = map.getBoundingClientRect(); return [(ev.clientX - r.left) * (W / r.width), (ev.clientY - r.top) * (H / r.height)]; }

  const COLORS = { wall: null, room: "#7f7fbf", zone: "#ffd23a", entry: "#3dff9a", exit: "#37f0e6", pickup: "#ffd23a" };
  function drawDiamond(cx, cy, rr) { mx.beginPath(); mx.moveTo(cx, cy - rr); mx.lineTo(cx + rr, cy); mx.lineTo(cx, cy + rr); mx.lineTo(cx - rr, cy); mx.closePath(); }
  function draw() {
    const f = floor(); const t = performance.now() / 1000;
    mx.setTransform(1, 0, 0, 1, 0, 0);
    mx.clearRect(0, 0, W, H);
    mx.fillStyle = "#06040c"; mx.fillRect(0, 0, W, H);
    if (!f) {
      mx.fillStyle = "rgba(216,210,255,.5)"; mx.font = "26px VT323, monospace"; mx.textAlign = "center"; mx.textBaseline = "middle";
      mx.fillText("NO FLOOR LOADED", W / 2, H / 2); return;
    }
    const accent = /^#[0-9a-fA-F]{6}$/.test(f.accent) ? f.accent : "#37f0e6";
    const s = S.view.s;
    // grid
    const step = S.grid * s < 6 ? S.grid * Math.ceil(6 / (S.grid * s)) : S.grid;
    mx.lineWidth = 1;
    const [x0, y0] = s2w(0, 0), [x1, y1] = s2w(W, H);
    for (let x = Math.floor(x0 / step) * step; x <= x1; x += step) {
      const [px] = w2s(x, 0); mx.strokeStyle = x % 100 === 0 ? "rgba(120,90,220,.22)" : "rgba(120,90,220,.09)";
      mx.beginPath(); mx.moveTo(Math.round(px) + .5, 0); mx.lineTo(Math.round(px) + .5, H); mx.stroke();
    }
    for (let y = Math.floor(y0 / step) * step; y <= y1; y += step) {
      const [, py] = w2s(0, y); mx.strokeStyle = y % 100 === 0 ? "rgba(120,90,220,.22)" : "rgba(120,90,220,.09)";
      mx.beginPath(); mx.moveTo(0, Math.round(py) + .5); mx.lineTo(W, Math.round(py) + .5); mx.stroke();
    }
    // floor bounds
    { const [bx, by] = w2s(0, 0); mx.strokeStyle = "rgba(216,210,255,.35)"; mx.setLineDash([6, 4]); mx.strokeRect(bx + .5, by + .5, f.size.w * s, f.size.h * s); mx.setLineDash([]); }
    const R = (r) => { const [px, py] = w2s(r.x, r.y); return [px, py, r.w * s, r.h * s]; };
    // rooms
    for (const r of f.rooms) {
      const [px, py, pw, ph] = R(r);
      mx.fillStyle = accent; mx.globalAlpha = .07; mx.fillRect(px, py, pw, ph); mx.globalAlpha = 1;
      mx.strokeStyle = "rgba(216,210,255,.28)"; mx.lineWidth = 1; mx.strokeRect(px + .5, py + .5, pw, ph);
      if (r.label) { mx.fillStyle = "rgba(216,210,255,.6)"; mx.font = Math.max(11, Math.round(18 * s)) + "px VT323, monospace"; mx.textAlign = "left"; mx.textBaseline = "top"; mx.fillText(r.label, px + 5, py + 4); }
    }
    // zones (dashed amber)
    for (const z of f.zones) {
      const [px, py, pw, ph] = R(z);
      mx.fillStyle = "rgba(255,210,58,.05)"; mx.fillRect(px, py, pw, ph);
      mx.strokeStyle = "rgba(255,210,58,.75)"; mx.lineWidth = 1.5; mx.setLineDash([6, 5]); mx.strokeRect(px + .5, py + .5, pw, ph); mx.setLineDash([]);
      mx.fillStyle = "rgba(255,210,58,.85)"; mx.font = Math.max(10, Math.round(14 * s)) + "px VT323, monospace"; mx.textAlign = "right"; mx.textBaseline = "bottom"; mx.fillText("» " + z.id, px + pw - 4, py + ph - 3);
    }
    // walls (neon)
    mx.save(); mx.shadowColor = accent; mx.shadowBlur = 10; mx.strokeStyle = accent; mx.lineWidth = 2;
    for (const w of f.walls) {
      const [px, py, pw, ph] = R(w);
      mx.fillStyle = accent; mx.globalAlpha = .18; mx.fillRect(px, py, pw, ph);
      mx.globalAlpha = .9; mx.strokeRect(px + .5, py + .5, pw, ph);
    }
    mx.restore(); mx.globalAlpha = 1;
    // entry
    { const e = f.entry, [px, py, pw, ph] = R(e), col = COLORS.entry;
      mx.save(); mx.shadowColor = col; mx.shadowBlur = 14; mx.strokeStyle = col; mx.fillStyle = col; mx.lineWidth = 2;
      mx.globalAlpha = .18; mx.fillRect(px, py, pw, ph); mx.globalAlpha = 1; mx.strokeRect(px + .5, py + .5, pw, ph);
      const cx = px + pw / 2, cy = py + ph / 2, r = Math.min(pw, ph) * 0.3;
      mx.beginPath(); mx.moveTo(cx, cy - r); mx.lineTo(cx + r * 0.9, cy + r * 0.7); mx.lineTo(cx - r * 0.9, cy + r * 0.7); mx.closePath(); mx.fill();
      mx.shadowBlur = 6; mx.font = Math.max(10, Math.round(15 * s)) + "px VT323, monospace"; mx.textAlign = "center"; mx.textBaseline = "bottom";
      mx.fillText((e.label || "ENTRY") + " » CL4-UD3", cx, py - 3);
      mx.restore(); }
    // exits
    for (const e of f.exits) {
      const [px, py, pw, ph] = R(e), col = COLORS.exit;
      mx.save(); mx.shadowColor = col; mx.shadowBlur = 16; mx.strokeStyle = col; mx.fillStyle = col; mx.lineWidth = 2;
      mx.globalAlpha = e.open ? .25 : .12; mx.fillRect(px, py, pw, ph); mx.globalAlpha = 1;
      if (!e.open) mx.setLineDash([5, 4]); mx.strokeRect(px + .5, py + .5, pw, ph); mx.setLineDash([]);
      const cx = px + pw / 2, cy = py + ph / 2, pulse = 1 + 0.15 * Math.sin(t * 2.2);
      const rr = Math.min(pw, ph) * 0.18;
      for (let i = 0; i < 3; i++) { mx.globalAlpha = .8 - i * 0.22; mx.beginPath(); mx.arc(cx, cy, rr * (1 + i * 0.75) * pulse, 0, 7); mx.stroke(); }
      mx.globalAlpha = 1; mx.shadowBlur = 6; mx.font = Math.max(10, Math.round(15 * s)) + "px VT323, monospace"; mx.textAlign = "center"; mx.textBaseline = "top";
      mx.fillText((e.label || e.id) + " → " + (e.to === 0 ? "END" : F.floorLabel(e.to)) + (e.open ? " (OPEN)" : ""), cx, py + ph + 3);
      mx.restore();
    }
    // pickups
    for (const p of f.pickups) {
      const [px, py] = w2s(p.x, p.y), col = COLORS.pickup, r = 6;
      mx.save(); mx.shadowColor = col; mx.shadowBlur = 10; mx.fillStyle = col; mx.fillRect(px - r, py - r, r * 2, r * 2);
      mx.shadowBlur = 0; mx.fillStyle = "#1a1400"; mx.font = "bold 12px VT323, monospace"; mx.textAlign = "center"; mx.textBaseline = "middle";
      mx.fillText(p.weapon[0].toUpperCase(), px, py + 1);
      mx.fillStyle = col; mx.font = "12px VT323, monospace"; mx.textBaseline = "top"; mx.fillText(p.weapon, px, py + r + 2);
      mx.restore();
    }
    // scenario wave spawns (ghosts)
    for (const st of f.scenario) for (const a of st.actions) if (a.spawn) for (const sp of a.spawn) {
      const [px, py] = w2s(sp.x, sp.y), col = F.SPAWN_COLORS[sp.type] || "#fff";
      mx.save(); mx.strokeStyle = col; mx.globalAlpha = .55; mx.lineWidth = 1.5; mx.setLineDash([3, 3]); drawDiamond(px, py, 10); mx.stroke();
      mx.setLineDash([]); mx.fillStyle = col; mx.font = "12px VT323, monospace"; mx.textAlign = "center"; mx.textBaseline = "middle"; mx.fillText(F.SPAWN_LETTER[sp.type], px, py + 1);
      mx.restore();
    }
    // spawns
    f.spawns.forEach((sp, i) => {
      const [px, py] = w2s(sp.x, sp.y), col = F.SPAWN_COLORS[sp.type] || "#fff";
      const pulse = 1 + 0.12 * Math.sin(t * 3 + sp.x + sp.y), rr = 11 * pulse;
      mx.save(); mx.shadowColor = col; mx.shadowBlur = 14; mx.fillStyle = col; mx.globalAlpha = .9; drawDiamond(px, py, rr); mx.fill();
      mx.globalAlpha = 1; mx.strokeStyle = "#0a0710"; mx.lineWidth = 1.5; drawDiamond(px, py, rr); mx.stroke();
      mx.shadowBlur = 0; mx.fillStyle = "#0a0710"; mx.font = "bold 14px VT323, monospace"; mx.textAlign = "center"; mx.textBaseline = "middle";
      mx.fillText(F.SPAWN_LETTER[sp.type] || "?", px, py + 1);
      if (sp.type === "patrolling") { mx.strokeStyle = col; mx.globalAlpha = .25; mx.setLineDash([3, 4]); mx.beginPath(); mx.arc(px, py, 40 * s, 0, 7); mx.stroke(); }
      mx.restore();
    });
    // hover
    if (S.hover && !S.drag && !(S.sel && S.hover.kind === S.sel.kind && S.hover.i === S.sel.i)) {
      const it = itemOf(S.hover); if (it) { const [px, py, pw, ph] = bounds(it, S.hover.kind); mx.strokeStyle = "rgba(255,255,255,.35)"; mx.lineWidth = 1; mx.strokeRect(px - 2.5, py - 2.5, pw + 5, ph + 5); }
    }
    // selection
    const sit = selItem();
    if (sit) {
      const [px, py, pw, ph] = bounds(sit, S.sel.kind);
      mx.save(); mx.strokeStyle = "#fff"; mx.shadowColor = "#fff"; mx.shadowBlur = 8; mx.lineWidth = 1.5; mx.setLineDash([4, 3]);
      mx.strokeRect(px - 3.5, py - 3.5, pw + 7, ph + 7); mx.setLineDash([]);
      if (isRectKind(S.sel.kind)) for (const h of handles(sit)) { mx.fillStyle = "#fff"; mx.fillRect(h.px - 4, h.py - 4, 8, 8); }
      mx.restore();
    }
    // creation preview
    if (S.drag && S.drag.mode === "create") {
      const r = S.drag.rect; const [px, py, pw, ph] = R(r);
      mx.save(); mx.strokeStyle = "#fff"; mx.setLineDash([4, 3]); mx.strokeRect(px + .5, py + .5, pw, ph); mx.setLineDash([]);
      mx.fillStyle = "#fff"; mx.font = "13px VT323, monospace"; mx.textAlign = "left"; mx.textBaseline = "bottom"; mx.fillText(S.drag.kind + " " + r.w + "×" + r.h, px, py - 2); mx.restore();
    }
    // cursor coords
    if (S.mouse) {
      const [wx, wy] = s2w(S.mouse[0], S.mouse[1]);
      mx.fillStyle = "rgba(216,210,255,.55)"; mx.font = "14px VT323, monospace"; mx.textAlign = "right"; mx.textBaseline = "bottom";
      mx.fillText(Math.round(wx) + ", " + Math.round(wy) + "  ×" + s.toFixed(2), W - 6, H - 4);
    }
    mx.fillStyle = "rgba(216,210,255,.4)"; mx.font = "14px VT323, monospace"; mx.textAlign = "left"; mx.textBaseline = "bottom";
    mx.fillText("TOOL: " + S.tool.toUpperCase() + (S.tool === "spawn" ? " / " + $("#spawn-type").value : S.tool === "pickup" ? " / " + $("#pickup-weapon").value : ""), 6, H - 4);
  }
  const isRectKind = (k) => k !== "spawn" && k !== "pickup";
  function itemOf(ref) { const f = floor(); if (!f || !ref) return null; return ref.kind === "entry" ? f.entry : (f[ref.kind + "s"] || [])[ref.i]; }
  function bounds(it, kind) {
    if (isRectKind(kind)) { const [px, py] = w2s(it.x, it.y); return [px, py, it.w * S.view.s, it.h * S.view.s]; }
    const [px, py] = w2s(it.x, it.y); return [px - 11, py - 11, 22, 22];
  }
  function handles(it) {
    const [px, py] = w2s(it.x, it.y), pw = it.w * S.view.s, ph = it.h * S.view.s;
    return [
      { id: "nw", px, py }, { id: "n", px: px + pw / 2, py }, { id: "ne", px: px + pw, py },
      { id: "e", px: px + pw, py: py + ph / 2 }, { id: "se", px: px + pw, py: py + ph },
      { id: "s", px: px + pw / 2, py: py + ph }, { id: "sw", px, py: py + ph }, { id: "w", px, py: py + ph / 2 },
    ];
  }
  function hitTest(px, py) {
    const f = floor(); if (!f) return null;
    const [wx, wy] = s2w(px, py);
    const tol = 12 / S.view.s;
    // points first
    for (let i = f.spawns.length - 1; i >= 0; i--) { const p = f.spawns[i]; if (Math.abs(p.x - wx) <= tol && Math.abs(p.y - wy) <= tol) return { kind: "spawn", i }; }
    for (let i = f.pickups.length - 1; i >= 0; i--) { const p = f.pickups[i]; if (Math.abs(p.x - wx) <= tol && Math.abs(p.y - wy) <= tol) return { kind: "pickup", i }; }
    // rects: smallest area wins, with a class priority for ties/near-ties
    let best = null, bestArea = Infinity;
    const consider = (kind, i, r) => {
      if (wx >= r.x - tol / 2 && wx <= r.x + r.w + tol / 2 && wy >= r.y - tol / 2 && wy <= r.y + r.h + tol / 2) {
        const area = r.w * r.h; if (area < bestArea) { bestArea = area; best = { kind, i }; }
      }
    };
    f.walls.forEach((r, i) => consider("wall", i, r));
    f.exits.forEach((r, i) => consider("exit", i, r));
    consider("entry", 0, f.entry);
    f.zones.forEach((r, i) => consider("zone", i, r));
    f.rooms.forEach((r, i) => consider("room", i, r));
    return best;
  }
  function handleAt(px, py) {
    const it = selItem(); if (!it || !isRectKind(S.sel.kind)) return null;
    for (const h of handles(it)) if (Math.abs(h.px - px) <= 6 && Math.abs(h.py - py) <= 6) return h.id;
    return null;
  }

  /* ---- mouse ---- */
  map.addEventListener("contextmenu", (e) => e.preventDefault());
  map.addEventListener("mousedown", (e) => {
    const f = floor(); if (!f) return;
    map.focus && map.focus();
    const [px, py] = canvasPos(e); const [wx, wy] = s2w(px, py);
    const pan = e.button === 1 || S.space || S.tool === "pan";
    if (pan) { S.drag = { mode: "pan", start: [px, py], view: Object.assign({}, S.view) }; e.preventDefault(); return; }
    if (e.button !== 0) return;
    if (S.tool === "select") {
      const h = handleAt(px, py);
      if (h) { S.drag = { mode: "resize", handle: h, start: [wx, wy], orig: clone(selItem()), snap: snapshot() }; return; }
      const hit = hitTest(px, py);
      if (hit) {
        S.sel = hit; renderProps();
        S.drag = { mode: "move", start: [wx, wy], orig: clone(selItem()), snap: snapshot(), moved: false };
      } else {
        S.sel = null; renderProps();
        S.drag = { mode: "pan", start: [px, py], view: Object.assign({}, S.view) };
      }
      return;
    }
    if (S.tool === "spawn" || S.tool === "pickup") {
      const x = snapv(wx), y = snapv(wy);
      mutate((fl) => {
        if (S.tool === "spawn") { fl.spawns.push({ x, y, type: $("#spawn-type").value }); S.sel = { kind: "spawn", i: fl.spawns.length - 1 }; }
        else { fl.pickups.push({ x, y, weapon: $("#pickup-weapon").value }); S.sel = { kind: "pickup", i: fl.pickups.length - 1 }; }
      }, { canvasOnly: true }); renderProps();
      return;
    }
    // rect creation tools
    S.drag = { mode: "create", kind: S.tool, start: [snapv(wx), snapv(wy)], rect: { x: snapv(wx), y: snapv(wy), w: 0, h: 0 } };
  });
  window.addEventListener("mousemove", (e) => {
    const [px, py] = canvasPos(e); S.mouse = [px, py];
    const d = S.drag;
    if (!d) {
      if (floor() && S.tool === "select") { const h = handleAt(px, py); S.hover = h ? null : hitTest(px, py); map.className = h ? "move" : (S.hover ? "pointer" : ""); }
      else map.className = S.tool === "pan" || S.space ? "pan" : "";
      return;
    }
    const [wx, wy] = s2w(px, py);
    if (d.mode === "pan") { S.view.ox = d.view.ox + (px - d.start[0]); S.view.oy = d.view.oy + (py - d.start[1]); return; }
    if (d.mode === "create") {
      const x0 = d.start[0], y0 = d.start[1], x1 = snapv(wx), y1 = snapv(wy);
      d.rect = { x: Math.min(x0, x1), y: Math.min(y0, y1), w: Math.abs(x1 - x0), h: Math.abs(y1 - y0) };
      return;
    }
    if (d.mode === "move") {
      const it = selItem(); if (!it) return;
      const dx = wx - d.start[0], dy = wy - d.start[1];
      const nx = snapv(d.orig.x + dx), ny = snapv(d.orig.y + dy);
      if (nx !== it.x || ny !== it.y) { it.x = nx; it.y = ny; d.moved = true; }
      return;
    }
    if (d.mode === "resize") {
      const it = selItem(); if (!it) return; const o = d.orig, h = d.handle;
      let x0 = o.x, y0 = o.y, x1 = o.x + o.w, y1 = o.y + o.h;
      const sx = snapv(wx), sy = snapv(wy);
      if (h.includes("w")) x0 = Math.min(sx, x1 - 1); if (h.includes("e")) x1 = Math.max(sx, x0 + 1);
      if (h.includes("n")) y0 = Math.min(sy, y1 - 1); if (h.includes("s")) y1 = Math.max(sy, y0 + 1);
      it.x = x0; it.y = y0; it.w = x1 - x0; it.h = y1 - y0; d.moved = true;
    }
  });
  window.addEventListener("mouseup", (e) => {
    const d = S.drag; if (!d) return; S.drag = null;
    if (d.mode === "create") {
      let r = d.rect;
      if (r.w < 2 || r.h < 2) { const g = Math.max(S.grid, 10); r = { x: r.x, y: r.y, w: d.kind === "wall" ? g : g * 3, h: d.kind === "wall" ? g : g * 2 }; }
      mutate((fl) => {
        if (d.kind === "wall") { fl.walls.push(r); S.sel = { kind: "wall", i: fl.walls.length - 1 }; }
        else if (d.kind === "room") { fl.rooms.push(Object.assign({ id: uniqueId(fl.rooms, "room"), label: "ROOM" }, r)); S.sel = { kind: "room", i: fl.rooms.length - 1 }; }
        else if (d.kind === "zone") { fl.zones.push(Object.assign({ id: uniqueId(fl.zones, "zone") }, r)); S.sel = { kind: "zone", i: fl.zones.length - 1 }; }
        else if (d.kind === "exit") { fl.exits.push(Object.assign({ id: uniqueId(fl.exits, "exit"), label: "EXIT" }, r, { to: Math.min(fl.id + 1, F.MAX_FLOOR), open: false })); S.sel = { kind: "exit", i: fl.exits.length - 1 }; }
        else if (d.kind === "entry") { fl.entry = Object.assign({ label: fl.entry.label || "ENTRY" }, r); S.sel = { kind: "entry", i: 0 }; }
      });
      return;
    }
    if ((d.mode === "move" || d.mode === "resize") && d.moved) { pushUndo(d.snap); markDirty(); renderProps(); }
  });
  map.addEventListener("wheel", (e) => {
    e.preventDefault(); if (!floor()) return;
    const [px, py] = canvasPos(e); const [wx, wy] = s2w(px, py);
    const k = Math.exp(-e.deltaY * 0.0015); const ns = Math.min(8, Math.max(0.1, S.view.s * k));
    S.view.s = ns; S.view.ox = px - wx * ns; S.view.oy = py - wy * ns;
  }, { passive: false });
  map.addEventListener("dblclick", (e) => { if (S.tool === "select" && !hitTest(...canvasPos(e))) fitView(); });

  /* ---- keyboard ---- */
  const inField = (e) => /^(INPUT|TEXTAREA|SELECT)$/.test((e.target && e.target.tagName) || "");
  window.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") { e.preventDefault(); save(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") { if (inField(e)) return; e.preventDefault(); if (e.shiftKey) redo(); else undo(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") { if (inField(e)) return; e.preventDefault(); redo(); return; }
    if (inField(e)) { if (e.key === "Escape") e.target.blur(); return; }
    if (e.key === " ") { S.space = true; map.className = "pan"; e.preventDefault(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") { e.preventDefault(); duplicateSelection(); return; }
    if (e.key === "Delete" || e.key === "Backspace") { deleteSelection(); e.preventDefault(); return; }
    if (e.key === "Escape") { setTool("select"); S.sel = null; renderProps(); return; }
    const nud = e.shiftKey ? S.grid : 1;
    if (e.key === "ArrowLeft") { nudge(-nud, 0); e.preventDefault(); return; }
    if (e.key === "ArrowRight") { nudge(nud, 0); e.preventDefault(); return; }
    if (e.key === "ArrowUp") { nudge(0, -nud); e.preventDefault(); return; }
    if (e.key === "ArrowDown") { nudge(0, nud); e.preventDefault(); return; }
    const tk = { v: "select", h: "pan", w: "wall", r: "room", z: "zone", s: "spawn", p: "pickup", n: "entry", e: "exit" }[e.key.toLowerCase()];
    if (tk && !e.ctrlKey && !e.metaKey && !e.altKey) { setTool(tk); return; }
    if (e.key.toLowerCase() === "f") fitView();
  });
  window.addEventListener("keyup", (e) => { if (e.key === " ") { S.space = false; map.className = ""; } });
  function setTool(t) { S.tool = t; document.querySelectorAll("#toolbar button[data-tool]").forEach((b) => b.classList.toggle("on", b.dataset.tool === t)); }
  document.querySelectorAll("#toolbar button[data-tool]").forEach((b) => b.addEventListener("click", () => setTool(b.dataset.tool)));
  $("#snap").addEventListener("change", (e) => { S.snap = e.target.checked; });
  $("#grid").addEventListener("change", (e) => { const v = parseInt(e.target.value, 10); if (v > 0) S.grid = v; });
  $("#btn-fit").addEventListener("click", fitView);
  $("#btn-undo").addEventListener("click", undo); $("#btn-redo").addEventListener("click", redo);
  $("#btn-save").addEventListener("click", save);
  $("#btn-download").addEventListener("click", download);
  $("#btn-play").addEventListener("click", play);
  $("#btn-import").addEventListener("click", () => $("#file-import").click());
  $("#file-import").addEventListener("change", (e) => { if (e.target.files[0]) importFile(e.target.files[0]); e.target.value = ""; });
  window.addEventListener("beforeunload", (e) => { if (S.list.some((x) => x.dirty)) { e.preventDefault(); e.returnValue = ""; } });

  /* ---------------- render all ---------------- */
  function renderAll(opts) {
    applyAccent();
    $("#mapfloorno").textContent = floor() ? "FLOOR " + F.floorLabel(floor().id) : "FLOOR --";
    renderFloorSel();
    if (opts && opts.canvasOnly) { renderProps(); validateLive(); return; }
    renderMeta(); renderProps(); renderSteps(); renderPreview(); validateLive();
  }
  function loop() { draw(); requestAnimationFrame(loop); }

  /* ---------------- boot ---------------- */
  window.__ready = false;
  window.__editor = {
    state: S, F, floor, save, download, selectEntry, newFloor, loadIndex, setTool, fitView, undo, redo,
    canonical: () => currentText(),
    validate: () => F.validate(floor(), { knownIds: KNOWN_IDS() }),
    select: (kind, i) => { S.sel = { kind, i }; renderProps(); },
    mutate,
    importText: (txt, name) => importFile(new File([txt], name || "import.json", { type: "application/json" })),
  };
  (async () => {
    await loadFont();
    setTool("select");
    updateUndoButtons();
    const ok = await loadIndex(false);
    if (!ok) status("no index.json in " + S.dir + " — start with + NEW FLOOR or IMPORT", "bad");
    let entry = null;
    if (qp.has("floor")) { const n = parseInt(qp.get("floor"), 10); entry = S.list.find((e) => e.id === n) || null; }
    if (!entry && S.list.length) entry = S.list[0];
    if (entry) await selectEntry(entry); else renderAll();
    loop();
    requestAnimationFrame(() => { window.__ready = true; });
  })();
})();
