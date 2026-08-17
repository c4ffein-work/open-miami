# End-to-End Tests for Open Miami

Browser-based tests (Playwright + Chromium) that load the wasm build in a real
page and actually play: start on a floor, read the WebGL HUD, move, use the
debug helpers, ride the exit elevator to the next floor.

## Running

Always from the repository root, through the Makefile (it builds the wasm,
generates the wasm-bindgen glue, installs the browser, and wraps the run in a
60-second `timeout` so a hung browser cannot hang the shell):

```bash
make check-e2e
```

That target does, in order:

1. `cargo build --release --target wasm32-unknown-unknown`
2. `wasm-bindgen ... --target web` (installs `wasm-bindgen-cli` pinned to the
   `wasm-bindgen` version in `Cargo.lock` if missing)
3. `cd tests/e2e && bun install` — the toolchain is **Bun** (`bun` / `bunx`),
   not npm/npx
4. `bunx playwright install --with-deps chromium` (falls back to a rootless
   install + `./setup-browser-deps.sh`, which extracts the browser's system
   libraries under the gitignored `tests/e2e/playwright-deps/` and exposes
   them via `LD_LIBRARY_PATH`)
5. `timeout 60 bunx playwright test`

`make verify` does NOT include the e2e tests; `make verify-all` does.

Running Playwright by hand (after `make build-wasm` or a previous
`make check-e2e`), still from the repo root:

```bash
cd tests/e2e
bun install
# rootless box: point the browser at the locally extracted system libs
export LD_LIBRARY_PATH="$(find "$PWD/playwright-deps/libs" -name '*.so*' -printf '%h\n' | sort -u | paste -sd:)"
bunx playwright test                       # everything
bunx playwright test specs/scenario.spec.js  # one spec
bunx playwright test --headed              # watch the browser
bunx playwright test --debug               # Playwright inspector
```

The config's `webServer` starts `python3 -m http.server 8000` at the repo root
and reuses one that is already listening (e.g. `python3 serve.py`).

## Specs

| File | What it checks |
| --- | --- |
| `specs/level-completion.spec.js` | `/?floor=1&debug` loads straight into floor 1 with the in-game HUD drawn and zero page/console errors; the player moves (world position + frame change), **I** then **K** purges every rogue (`Rogues: 0`), the SERVICE LIFT opens, walking into it extracts to floor 2 (COLD STORAGE / FREIGHT LIFT objective). |
| `specs/scenario.spec.js` | The floor/scenario engine: purge on floor 1, objective text, extraction elevator floor 1 -> floor 2, no errors. |
| `specs/helpers.js` | Shared helpers (not a spec): the frame probe, held-key input, floor loading, the floor-1 walk. |

## How the tests see the game

Everything is drawn on `<canvas#glcanvas>` through WebGL, so there is no DOM
to assert on. Each frame the wasm engine makes one call,
`window.frameRender(cmds, textArena)` (see `src/graphics.rs`, `renderer.js`).
`helpers.js` installs an init script that wraps that function and records, on
`window.__om`:

- `frames` — frame counter
- `texts` — every string drawn in the last frame (HUD labels, values, the
  objective line, comms feed, elevator labels), split on U+001F
- `player` — the player's world position, taken from the ROBOT command with
  the player's colour index (opcode table mirrored from `renderer.js`
  `OP_ARGS`; keep in sync if opcodes change)

Assertions read the HUD as text (`hudValue(texts, 'Rogues:')`), and walking is
position-driven (`walkUntil(page, 'a', p => p.x <= 108)`), so the specs do not
depend on the headless frame rate.

**Errors are not filtered.** WebGL is required (all rendering is WebGL), so any
`pageerror` or `console.error` — including WebGL ones — fails the test.

## Conventions

- `/?floor=N&debug` — start directly on floor N (14 = 13½) with the debug keys
  enabled (no title / level-select screen).
- Debug keys (need `&debug`): **I** toggles the debug overlay; with it on,
  **K** purges every rogue (incl. the boss) and **B** cracks the boss's mask.
- Keys must be **held**: `page.keyboard.down`, wait, `page.keyboard.up`. The
  engine samples key state once per frame and edge-detects presses
  (`is_key_pressed`), so a zero-length press between two frames is lost.
  `hold()` / `tap()` in `helpers.js` also wait for at least two rendered
  frames while the key is down.
- Focus the canvas (`canvas.focus()`) rather than clicking it — a click fires
  the weapon.
- Screenshots go to `test-results/*.png` (gitignored); the HTML report to
  `playwright-report/`.

## Timeouts

- 60 s per test (`timeout` in `playwright.config.js`) — the number the
  Makefile / CLAUDE.md promise; the Makefile additionally wraps the whole run
  in `timeout 60`, so keep the specs short (a full floor-1 playthrough is
  ~15 s, the three tests run in parallel locally).
- 10 min `globalTimeout`, 1 retry and a single worker on CI.

## Chromium flags

`playwright.config.js` launches headless Chromium with
`--enable-unsafe-swiftshader --use-gl=angle --use-angle=swiftshader` so WebGL
works without a GPU (software ANGLE/SwiftShader). Needed on a GPU-less local
box; harmless on the CI ubuntu runners.

## CI

`.github/workflows/e2e-tests.yml` runs `bunx playwright test` on every push /
pull request (a failing spec fails the job), uploads `test-results/` +
`playwright-report/` as artifacts and posts the pass/fail verdict on the PR.

## Adding a test

1. Create `specs/<name>.spec.js`.
2. `const { loadFloor, purgeRogues, hudValue, lastFrameTexts, walkUntil, hold, tap, collectErrors } = require('./helpers');`
3. `const errors = collectErrors(page); const canvas = await loadFloor(page, N);`
   then drive the game with held keys and assert on `lastFrameTexts(page)` /
   `playerPos(page)`; finish with `expect(errors).toEqual([]);`.
