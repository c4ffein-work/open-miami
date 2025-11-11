## Development Constraints
- NEVER add any additional dependency

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

#### **CRITICAL: E2E Test Timeout Enforcement**
- **ALWAYS** run E2E tests via `make check-e2e` - this is the ONLY acceptable way to run these tests
- **NEVER** run e2e tests directly with `npm test` or `playwright test` commands
- The Makefile enforces a 60-second timeout to prevent tests from hanging indefinitely
- Running tests without timeout can cause Claude Code instances to be terminated (they will be considered stuck)
- Both the Playwright config and Makefile enforce this 60-second timeout for safety

## Artifact Server

An artifact server is available at `$ARTIFACTER_API_URL`. You can PUT to any route and it will become available to the user through GET. Authentication is done using `$ARTIFACTER_API_KEY`.

### Usage
- Upload files via PUT request to `$ARTIFACTER_API_URL/<path>`
- Files become accessible via GET at the same URL
- Use `$ARTIFACTER_API_KEY` for authentication (typically in Authorization header)
- Useful for sharing build artifacts like wasm files and HTML pages for quick iteration
