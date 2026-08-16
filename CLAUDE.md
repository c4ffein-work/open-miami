## Development Constraints
- NEVER add any additional dependency

## Rendering Architecture
- The Rust/wasm engine owns the simulation only; **all rendering is WebGL in JS**
- Each frame, `Graphics` (src/graphics.rs) records a flat f32 command stream
  (rects, circles, lines, arcs, text, transforms, robots) and hands it to
  `window.frameRender` once per frame — a single zero-copy wasm->JS crossing
- `renderer.js` owns the canvas/GPU: one batched triangle pipeline, VT323 text
  via a lazily-built glyph atlas, robots live-rendered through the
  proto/robot-core.js 3D->2D pipeline into a cached texture atlas (animation
  time is quantized to a few frames per pose)
- The command opcode tables in src/graphics.rs (`mod op`) and renderer.js must
  stay in sync

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
