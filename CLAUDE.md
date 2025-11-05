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
- E2E tests (`make check-e2e`) require WASM dependencies (wasm-bindgen, web-sys)
- Due to the "no additional dependencies" constraint, E2E tests cannot currently pass
- E2E tests are excluded from `make verify` but can be run separately with `make verify-all` if dependencies are added
- The CI E2E pipeline is expected to fail until dependencies are resolved
