## Development Constraints
- NEVER add any additional dependency

## Rendering Architecture
- The Rust/wasm engine owns the simulation only; **all rendering is WebGL in JS**
- Each frame, `Graphics` (src/graphics.rs) records a flat f32 command stream
  (rects, circles, lines, arcs, text, transforms, robots, the shoggoth) and
  hands it to `window.frameRender` once per frame — a single zero-copy
  wasm->JS crossing
- `renderer.js` owns the canvas/GPU: one batched triangle pipeline, VT323 text
  via a lazily-built glyph atlas, robots rendered LIVE every frame through the
  robot-core.js 3D->2D pipeline (`createRobotPipeline(gl)` on the same GL
  context, into a per-frame scratch tile atlas — continuous animation time, no
  cache/quantization); the boss the same way through shoggoth-core.js
  (`createShoggothPipeline(gl)`, a bigger 256px scratch tile, opcode SHOGGOTH
  = 13: `x y sizePx heading reveal time`)
- shoggoth-core.js extends robot-core's exported `SpritePipeline` (shared
  pass-1 target + inked post pass + `M4`); the 2D-primitive
  `Graphics::draw_shoggoth` is only the `?viz` gallery / level-map thumbnail
- The command opcode tables in src/graphics.rs (`mod op`) and renderer.js must
  stay in sync

## Repo layout (post-`proto/`)
- Root: `index.html`, `renderer.js`, `robot-core.js` (the 3D->2D robot pipeline, imported by renderer.js at runtime), `shoggoth-core.js` (the boss pipeline, built on robot-core), `serve.py` (dev server, no-store + level-editor write API)
- `tools/`: the `?viz` panels — `inspector.html` (character inspector: `?kind=robot&color=…` / `?kind=shoggoth&phase=masked|enraged`, `&embed=1` for the SPRITES tab; 3D orbit + 2D top-down views), `levels.html` + `levels-editor*.js` (level + scenario editor, LEVELS tab) — and `gen_levels.py`
- `levels/`: `floor_01..13.json`, `floor_13h.json`, `index.json` — the floors' single source of truth (format: `docs/SCENARIO_FORMAT.md`)
- `src/levels_data.rs` is GENERATED from `levels/*.json` by `make gen-levels`; `make check-levels` validates + checks it is current. Never hand-edit it.

## `?viz` toolbox (one entry point)
- `/?viz` tabs: SPRITES (click a character → 3D/2D inspector iframe), MUSICS (tracker + SFX), LEVELS (the level + scenario editor iframe, full pane), EFFECTS
- `/?floor=N` starts the game directly on floor N (14 = 13½). With debug overlays on (I), **K** purges all rogues (incl. the boss; debug/e2e helper) and **B** cracks the boss's mask (drops it to the enrage threshold so the live mask-off / raw form can be previewed)
- Level editor SAVE = `PUT /levels/<file>.json` to serve.py, guarded by the `X-Editor-Token` header (token from `$EDITOR_TOKEN` or the gitignored `.editor-token`, printed at server start). COPY DIFF gives a `patch -p1` unified diff.

## Verification Requirements
- ALWAYS run `make verify` before declaring any task complete or saying "we're done"
- The `make verify` command runs core CI pipeline checks locally:
  - Code formatting (rustfmt) - `make check-fmt`
  - Linting (clippy) - `make check-clippy`
  - Test suite (all tests including doc tests) - `make check-test`
  - Release build - `make check-build`
- ALL checks must pass before completing a task
- If any check fails, fix the issues and re-run `make verify`

### Note on E2E Tests
- E2E tests (`make check-e2e`) require wasm-bindgen-cli build tool to be installed
- wasm-bindgen and web-sys are already in Cargo.toml as dependencies (no new dependencies needed)
- E2E tests are excluded from `make verify` but can be run separately with `make verify-all`
- The `make check-e2e` target will automatically install wasm-bindgen-cli if not present
- E2E tests require the wasm32-unknown-unknown Rust target and Playwright dependencies

#### **E2E Test Timeout Enforcement**
- Prefer running E2E tests via `make check-e2e` — it wires up the toolchain and the timeout for you
- The e2e toolchain runs on **Bun** (`bun install` / `bunx playwright ...`), not npm/node
- Both the Makefile and the Playwright config enforce a 60-second timeout so a run cannot hang indefinitely

## Debug Mode
- The game has a built-in debug mode that can be toggled by pressing **I** during gameplay
- Debug mode is enabled by default (`debug_enabled: true` in GameState)
- When debug mode is active, pressing **I** toggles the display of debug information

### Debug Visualizations
When debug info is enabled (press I), the following visualizations are shown:

1. **Enemy Vision Cones**: Shows the 90-degree vision cone for each enemy
2. **Inflated Wall Boundaries**: Yellow semi-transparent rectangles showing the 25px padding around walls used for pathfinding
3. **Pathfinding Waypoints**: For enemies in chasing mode (SpottedUnsure or SurePlayerSeen):
   - **Cyan line**: Actual movement trail showing where the enemy has traveled (last 100 positions)
   - **Red semi-transparent line**: Direct line from enemy to final target
   - **Green lines and dots**: Pathfinding waypoints showing the planned path the enemy will follow
   - **Red dot**: Final target position
   - **Green dots**: Individual waypoints along the path

These visualizations help understand and debug:
- Enemy AI behavior and detection
- Pathfinding algorithm results (A* + string pulling + wall-hugging)
- How inflated wall boundaries prevent wall grinding
- The difference between direct movement vs pathfinding
- Compare actual path taken (cyan) vs planned path (green)

## Artifact Server
- An artifact server is available at `$ARTIFACTER_API_URL`
- Use PUT requests to upload files to any route - the files will become available via GET requests
- Authentication requires `$ARTIFACTER_API_KEY` header
- This enables fast iteration by uploading wasm and HTML files for immediate testing
