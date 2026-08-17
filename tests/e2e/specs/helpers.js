// Shared helpers for the Playwright specs.
//
// The whole game is drawn on <canvas#glcanvas> through WebGL, so there is no
// DOM to assert on. Instead we tap the single wasm->JS crossing the engine
// makes each frame: `window.frameRender(cmds, textArena)` (see src/graphics.rs
// and renderer.js). The text arena is every string drawn that frame, joined by
// U+001F in draw order — HUD labels, the objective line, the comms feed, ... —
// which is enough to know which screen / floor we are on.
//
// Conventions:
//   - `/?floor=N&debug`: start straight on floor N (14 = 13½) with the debug
//     keys enabled. With the debug overlay on (I), K purges every rogue and B
//     cracks the boss's mask.
//   - Keys must be HELD (keyboard.down / wait / keyboard.up): the wasm input
//     layer samples key state per frame, a zero-length press can be missed.
const { expect } = require('@playwright/test');

const TEXT_SEP = '\u001f';

/**
 * Hold `key` for `ms` milliseconds — and for at least 2 rendered frames, so the
 * engine's per-frame edge detection (`is_key_pressed`: down now, up in the
 * previous frame) sees the press even when the headless frame rate is low.
 */
async function hold(page, key, ms) {
  const start = await frameCount(page);
  await page.keyboard.down(key);
  await page.waitForTimeout(ms);
  await waitForFrames(page, 2, 15000, start);
  await page.keyboard.up(key);
  await waitForFrames(page, 1);
}

/** A single key press, frame-aware (see `hold`). */
async function tap(page, key) {
  await hold(page, key, 30);
}

/**
 * Collect page errors + console errors. WebGL is REQUIRED (all rendering is
 * WebGL), so a WebGL error is a real failure — nothing is filtered here.
 */
function collectErrors(page) {
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(`console.error: ${m.text()}`);
  });
  return errors;
}

// Per-opcode argument counts of the frame command stream (must mirror
// `mod op` in src/graphics.rs and OP_ARGS in renderer.js).
const OP_ARGS = [4, 8, 9, 7, 9, 9, 8, 0, 0, 2, 1, 8, 2, 6, 5];
const OP_ROBOT = 11; // colorIdx poseIdx weaponIdx x y angle sizePx time
const ROBOT_COLOR_PLAYER = 0; // CL4-UD3, coral (src/lib.rs ROBOT_COLOR_CORAL)

/**
 * Must be called BEFORE page.goto(): wraps the renderer's `window.frameRender`
 * as soon as index.html installs it, recording on `window.__om`:
 *   frames  - a frame counter
 *   texts   - the last frame's text arena (U+001F-joined strings)
 *   player  - {x, y} world position of the player robot in the last frame
 *             (the ROBOT command with the player's colour; null if not drawn)
 *   robots  - number of ROBOT commands in the last frame
 */
async function installFrameProbe(page) {
  await page.addInitScript(
    ({ OP_ARGS, OP_ROBOT, ROBOT_COLOR_PLAYER }) => {
      const om = { frames: 0, texts: '', player: null, robots: 0 };
      window.__om = om;
      function scan(cmds) {
        let player = null;
        let robots = 0;
        const n = cmds.length;
        let i = 0;
        while (i < n) {
          const op = cmds[i++] | 0;
          const args = OP_ARGS[op];
          if (args === undefined) break; // unknown opcode: stop scanning
          if (op === OP_ROBOT) {
            robots += 1;
            if ((cmds[i] | 0) === ROBOT_COLOR_PLAYER) {
              player = { x: cmds[i + 3], y: cmds[i + 4] };
            }
          }
          i += args;
        }
        om.player = player;
        om.robots = robots;
      }
      let real = undefined;
      Object.defineProperty(window, 'frameRender', {
        configurable: true,
        get() { return real; },
        set(fn) {
          real = typeof fn === 'function'
            ? function (cmds, textArena) {
                om.frames += 1;
                om.texts = textArena;
                scan(cmds);
                return fn.call(this, cmds, textArena);
              }
            : fn;
        },
      });
    },
    { OP_ARGS, OP_ROBOT, ROBOT_COLOR_PLAYER },
  );
}

/** The player's world position {x, y} in the last frame (null if not drawn). */
async function playerPos(page) {
  return page.evaluate(() => (window.__om && window.__om.player) || null);
}

/**
 * Hold `key` until `pred(pos)` is true for the player's world position (polled
 * every frame-ish), then release. Position-driven rather than time-driven so
 * it does not depend on the headless frame rate. Returns the final position.
 */
async function walkUntil(page, key, pred, { timeout = 15000, what = `walking (${key})` } = {}) {
  const deadline = Date.now() + timeout;
  await page.keyboard.down(key);
  let pos = null;
  try {
    while (Date.now() < deadline) {
      pos = await playerPos(page);
      if (pos && pred(pos)) return pos;
      await page.waitForTimeout(30);
    }
  } finally {
    await page.keyboard.up(key);
  }
  throw new Error(`timed out ${what}; last player position: ${JSON.stringify(pos)}`);
}

/** The strings drawn in the most recent frame, in draw order. */
async function lastFrameTexts(page) {
  const arena = await page.evaluate(() => (window.__om && window.__om.texts) || '');
  return arena.length ? arena.split(TEXT_SEP) : [];
}

/** Frames rendered so far. */
async function frameCount(page) {
  return page.evaluate(() => (window.__om && window.__om.frames) || 0);
}

/** Wait until the engine has rendered at least `n` more frames (from `start`). */
async function waitForFrames(page, n, timeout = 15000, start = undefined) {
  if (start === undefined) start = await frameCount(page);
  await page.waitForFunction(
    ([s, k]) => window.__om && window.__om.frames >= s + k,
    [start, n],
    { timeout },
  );
}

/**
 * Wait until some frame's texts satisfy `pred` (polled). Returns those texts.
 * Fails the test with the last seen texts on timeout.
 */
async function waitForFrameTexts(page, pred, { timeout = 15000, what = 'frame text' } = {}) {
  const deadline = Date.now() + timeout;
  let texts = [];
  while (Date.now() < deadline) {
    texts = await lastFrameTexts(page);
    if (pred(texts)) return texts;
    await page.waitForTimeout(100);
  }
  throw new Error(`timed out waiting for ${what}; last frame texts: ${JSON.stringify(texts)}`);
}

/** The value drawn right after a `label` (e.g. "Rogues:" -> "12"). */
function hudValue(texts, label) {
  const i = texts.indexOf(label);
  return i >= 0 ? texts[i + 1] : undefined;
}

/**
 * Open `/?floor=N&debug`, wait for the game to actually be running (canvas
 * visible, frames flowing, in-game HUD drawn) and focus the canvas for input.
 */
async function loadFloor(page, floor) {
  await installFrameProbe(page);
  await page.goto(`/?floor=${floor}&debug`);
  const canvas = page.locator('canvas#glcanvas');
  await canvas.waitFor({ state: 'visible', timeout: 10000 });
  await waitForFrames(page, 10);
  await waitForFrameTexts(page, (t) => t.includes('Health:') && t.includes('Rogues:'), {
    what: 'the in-game HUD',
  });
  await canvas.focus();
  return canvas;
}

/**
 * Debug overlay on (I) then purge every rogue (K). Waits until the HUD reports
 * 0 rogues (the all-dead scenario step then opens the exit lift).
 */
async function purgeRogues(page) {
  await tap(page, 'i');
  await tap(page, 'k');
  await waitForFrameTexts(page, (t) => hudValue(t, 'Rogues:') === '0', {
    what: 'Rogues: 0 after the debug purge',
  });
}

/**
 * Floor 1 (RECEPTION CACHE): the entry lift is SE (895,750), the SERVICE LIFT
 * exit is the NW car x 60..150, y 20..80. Walk west along the bottom until
 * under the car, then north into it; the caller then waits for the dwell
 * (0.6 s), the extraction card and floor 2 to load.
 */
async function walkFloor1ToServiceLift(page) {
  await page.mouse.move(300, 400);
  await walkUntil(page, 'a', (p) => p.x <= 108, { what: 'walking west to the SERVICE LIFT column' });
  await page.mouse.move(640, 100);
  const pos = await walkUntil(page, 'w', (p) => p.y <= 60, { what: 'walking north into the SERVICE LIFT' });
  expect(pos.x).toBeGreaterThan(60);
  expect(pos.x).toBeLessThan(150);
}

/** Wait for the floor-2 HUD (its objective names the FREIGHT LIFT). */
async function expectFloor2(page) {
  const texts = await waitForFrameTexts(
    page,
    (t) => t.includes('Health:') && t.some((s) => s.includes('FREIGHT LIFT')),
    { timeout: 12000, what: 'floor 2 (COLD STORAGE / FREIGHT LIFT objective)' },
  );
  expect(texts.some((s) => s.includes('SERVICE LIFT'))).toBe(false);
  return texts;
}

module.exports = {
  TEXT_SEP,
  hold,
  tap,
  collectErrors,
  installFrameProbe,
  lastFrameTexts,
  frameCount,
  waitForFrames,
  waitForFrameTexts,
  hudValue,
  playerPos,
  walkUntil,
  loadFloor,
  purgeRogues,
  walkFloor1ToServiceLift,
  expectFloor2,
};
