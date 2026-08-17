// Headless acceptance test for the ?viz PROPS page pixel-art stability
// (not part of the Playwright spec run — a standalone script):
//
//   cd tests/e2e && bun props-stability.js [baseURL=http://localhost:8098] [shotsDir]
//
// The engine clock (performance.now) is frozen so frames are deterministic.
// 1) For CRAC COOLER, RACK/CLOSED, EXHAUST FAN, SECURITY CAM (DATACENTER)
//    and MAIN GATE, TURNSTILES, WALL CLOCK (OUTDOOR / LOBBY pages) at px 4:
//    two frames with the SAME clock must be pixel-identical, and between two
//    different clocks the ONLY differing pixels of the preview panel must lie
//    inside the rotating layer's group box (blinking LED layers are hidden
//    with the eye toggles first; each case picks clocks at which its part is
//    actually moving).
// 2) UPLINK OBELISK escort motes at px 4 / 6: soloed, over a frame sequence
//    the number of lit texels must stay constant (constant stamp shape).
// Layout numbers mirror src/lib.rs draw_viz_props at a 1280x800 viewport.
const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const BASE = process.argv[2] || 'http://localhost:8098';
const SHOTS = process.argv[3] || '';
const W = 1280, H = 800;
// One family page = up to 24 tiles in a 4 x 6 grid (src/props.rs largest_family()).
const COLS = 4, ROWS = 6, X0 = 40, Y0 = 152, TILE_W = 150;
const TILE_H = Math.min(110, Math.max(64, (H - Y0 - 16) / ROWS));
const PANEL_X = X0 + COLS * TILE_W + 20, PANEL_W = W - PANEL_X - 40, PANEL_H = ROWS * TILE_H - 6;

// The rotating layers under test (pivot, bounds from src/props.rs PROP_LAYERS)
// and the blinking layers to hide (indices in the prop's layer list). `fam` =
// the family page (0 DATACENTER, 1 OUTDOOR, 2 LOBBY), `idx` = the tile slot on
// that page, `times` = the two frozen clocks (ms) to compare.
const CASES = [
  { name: 'RACK / CLOSED', idx: 0, layers: 4, hide: [3], rot: [[0, -15, -14, -14, 28, 28], [0, 15, -14, -14, 28, 28]] },
  { name: 'CRAC COOLER', idx: 9, layers: 3, hide: [2], rot: [[0, 10, -27, -27, 54, 54]] },
  { name: 'EXHAUST FAN', idx: 11, layers: 3, hide: [], rot: [[0, 0, -33, -33, 66, 66]] },
  { name: 'SECURITY CAM', idx: 20, layers: 3, hide: [], rot: [[0, -30, -20, 0, 40, 65], [0, -30, -8, -7, 16, 31]] },
  // OUTDOOR: the gate's swing arm (+ its shadow layer) mid-swing; the scan
  // beam / LED layer is hidden.
  { name: 'MAIN GATE', fam: 1, idx: 6, layers: 5, hide: [2], times: [6500, 6810], rot: [[-32, 16, -1, -3, 64, 6], [-36, 12, -6, -6, 70, 12]] },
  // LOBBY: the turnstile's free arm mid-swing (LEDs hidden); the clock's hands.
  { name: 'TURNSTILES', fam: 2, idx: 1, layers: 5, hide: [4], times: [3100, 3400], rot: [[34, 0, -30, -4, 34, 8]] },
  { name: 'WALL CLOCK', fam: 2, idx: 16, layers: 4, hide: [], times: [5000, 5310], rot: [[0, 0, -3, -20, 6, 24], [0, 0, -2, -28, 4, 32], [0, 0, -1, -30, 2, 36]] },
];

function snapBox(x, y, w, h, px) {
  const gx = Math.floor(x / px) * px, gy = Math.floor(y / px) * px;
  return [gx, gy, Math.ceil((x + w) / px) * px - gx, Math.ceil((y + h) / px) * px - gy];
}
function rotBox(x, y, w, h, px) {
  const r = Math.max(...[[x, y], [x + w, y], [x, y + h], [x + w, y + h]].map(([a, b]) => Math.hypot(a, b)));
  return snapBox(-r, -r, 2 * r, 2 * r, px);
}
// src/props.rs snap_size: integer texel -> device pixel magnification.
function snapSize(sizePx, px) {
  if (px <= 1) return sizePx;
  const texels = 100 / px, k = Math.floor(sizePx / texels);
  return k >= 1 ? texels * k : sizePx;
}
function previewGeom(nLayers, px) {
  const listH = 30 + nLayers * 24 + 8;
  const areaTop = Y0 + 56, areaH = Math.max(60, PANEL_H - 56 - listH);
  const size = snapSize(Math.max(40, Math.min(PANEL_W, areaH) * 0.8), px);
  return { cx: PANEL_X + PANEL_W / 2, cy: areaTop + areaH / 2, size, s: size / 100, listY: Y0 + PANEL_H - listH };
}
const tileCenter = (i) => [X0 + (i % COLS) * TILE_W + (TILE_W - 6) / 2, Y0 + Math.floor(i / COLS) * TILE_H + (TILE_H - 6) / 2 - 6];
const PLUS = [583, 95], PROPS_BTN = [287, 95];
// The family page buttons, right of SAVE (src/lib.rs: cx + 414 + f * 120, 112 wide).
const FAMILY_BTN = (f) => [416 + 414 + f * 120 + 56, 95];
const eyeBtn = (listY, i) => [PANEL_X + 12 + 13, listY + 30 + i * 24 + 10];
const soloBtn = (listY, i) => [PANEL_X + 12 + 32 + 13, listY + 30 + i * 24 + 10];

(async () => {
  const browser = await chromium.launch({
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage({ viewport: { width: W, height: H } });
  const errors = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.addInitScript(() => {
    window.__fakeT = 5000;
    const orig = performance.now.bind(performance);
    performance.now = () => (window.__fakeT == null ? orig() : window.__fakeT);
  });
  const held = async (x, y) => {
    await page.mouse.move(x, y); await page.waitForTimeout(50);
    await page.mouse.down(); await page.waitForTimeout(150); await page.mouse.up(); await page.waitForTimeout(100);
  };
  const setTime = async (t) => { await page.evaluate((t) => { window.__fakeT = t; }, t); await page.waitForTimeout(150); };
  // Grab the preview panel as RGBA into window.__F[name] via a 2D canvas copy
  // of the WebGL canvas, right after a fresh engine frame (same task, so the
  // drawing buffer is intact). Diffs / counts are computed in-page.
  const grab = async (name) => page.evaluate(([name, x, y, w, h]) => {
    const gc = document.getElementById('glcanvas');
    const c = document.createElement('canvas'); c.width = w; c.height = h;
    const ctx = c.getContext('2d');
    window.__F = window.__F || {};
    return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => {
      ctx.drawImage(gc, x, y, w, h, 0, 0, w, h);
      window.__F[name] = ctx.getImageData(0, 0, w, h).data;
      resolve(true);
    })));
  }, [name, PANEL_X, Y0, PANEL_W, PANEL_H]);
  const diffPixels = async (na, nb) => page.evaluate(([na, nb, w]) => {
    const a = window.__F[na], b = window.__F[nb], out = [];
    for (let i = 0; i < a.length; i += 4) {
      if (a[i] !== b[i] || a[i + 1] !== b[i + 1] || a[i + 2] !== b[i + 2]) out.push([(i / 4) % w, Math.floor(i / 4 / w)]);
    }
    return out;
  }, [na, nb, PANEL_W]);
  const litCount = async (n) => page.evaluate((n) => {
    const f = window.__F[n]; let lit = 0;
    for (let i = 0; i < f.length; i += 4) if (f[i] + f[i + 1] + f[i + 2] > 90) lit++;
    return lit;
  }, n);
  const shot = async (name) => { if (SHOTS) await page.screenshot({ path: path.join(SHOTS, name) }); };

  let failures = 0;
  const report = (ok, msg) => { console.log((ok ? 'PASS ' : 'FAIL ') + msg); if (!ok) failures++; };

  // ---- 1) rotating layers only ----
  const PX = 4;
  for (const c of CASES) {
    await page.goto(BASE + '/?viz');
    await page.waitForFunction(() => document.getElementById('loading').style.display === 'none', null, { timeout: 30000 });
    await page.waitForTimeout(300);
    await held(...PROPS_BTN);
    if (c.fam) await held(...FAMILY_BTN(c.fam));
    await held(...tileCenter(c.idx));
    for (let i = 1; i < PX; i++) await held(...PLUS);
    const g = previewGeom(c.layers, PX);
    for (const li of c.hide) await held(...eyeBtn(g.listY, li));
    const [t1, t2] = c.times || [5000, 5310];
    const tag = `${c.fam || 0}_${c.idx}`;
    await setTime(t1);
    await grab('a1');
    await grab('a2');
    const same = (await diffPixels('a1', 'a2')).length;
    report(same === 0, `${c.name}: same clock, two frames -> ${same} differing pixels`);
    await shot(`layers_fix_stable_${tag}_t1.png`);
    await setTime(t2);
    await grab('b');
    await shot(`layers_fix_stable_${tag}_t2.png`);
    const d = await diffPixels('a1', 'b');
    // The rotating layers' boxes on screen (+1 px for the origin snap).
    const boxes = c.rot.map(([pvx, pvy, bx, by, bw, bh]) => {
      const [gx, gy, gw, gh] = rotBox(bx, by, bw, bh, PX);
      return [g.cx + (pvx + gx) * g.s - PANEL_X - 1, g.cy + (pvy + gy) * g.s - Y0 - 1, gw * g.s + 2, gh * g.s + 2];
    });
    const inside = (p) => boxes.some(([x, y, w, h]) => p[0] >= x && p[0] <= x + w && p[1] >= y && p[1] <= y + h);
    const outside = d.filter((p) => !inside(p));
    report(outside.length === 0 && d.length > 0,
      `${c.name}: px ${PX}, clock ${t1} vs ${t2} -> ${d.length} differing pixels, ${outside.length} OUTSIDE the rotating box(es)` +
      (outside.length ? ` e.g. ${JSON.stringify(outside.slice(0, 6))}` : ''));
  }

  // ---- 2) obelisk motes: constant stamp ----
  for (const px of [4, 6]) {
    await page.goto(BASE + '/?viz');
    await page.waitForFunction(() => document.getElementById('loading').style.display === 'none', null, { timeout: 30000 });
    await page.waitForTimeout(300);
    await held(...PROPS_BTN);
    await held(...tileCenter(23));
    for (let i = 1; i < px; i++) await held(...PLUS);
    const g = previewGeom(5, px);
    await held(...soloBtn(g.listY, 4)); // solo "escort"
    const texel = px * g.s;
    const counts = [];
    for (let k = 0; k < 8; k++) {
      await setTime(5000 + k * 90);
      await grab('m');
      // lit = anything brighter than the panel background (0.07,0.05,0.10)
      counts.push(await litCount('m'));
      if (k === 0 || k === 3) await shot(`layers_fix_motes_px${px}_f${k}.png`);
    }
    const texels = counts.map((n) => +(n / (texel * texel)).toFixed(2));
    const constant = counts.every((n) => n === counts[0]);
    report(constant, `UPLINK OBELISK motes px ${px} (texel = ${texel.toFixed(2)} dev px): lit pixels per frame ${JSON.stringify(counts)} = ${JSON.stringify(texels)} texels`);
  }

  console.log('console errors:', JSON.stringify(errors));
  await browser.close();
  process.exit(failures ? 1 : 0);
})();
