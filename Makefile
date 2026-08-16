.PHONY: verify check-test check-clippy check-fmt check-build check-wasm-build check-e2e check-coverage build-wasm gen-levels check-levels help

# Colors for output
RED=\033[0;31m
GREEN=\033[0;32m
YELLOW=\033[1;33m
NC=\033[0m # No Color

help:
	@echo "Available targets:"
	@echo "  make verify          - Run core checks (fmt, clippy, test, build)"
	@echo "  make verify-all      - Run all checks including E2E tests"
	@echo "  make check-test      - Run test suite"
	@echo "  make check-clippy    - Run clippy linting"
	@echo "  make check-fmt       - Check code formatting"
	@echo "  make check-build     - Build release binary"
	@echo "  make check-wasm-build - Build for wasm32 target (compilation check only)"
	@echo "  make build-wasm      - Build WASM and generate JavaScript glue (for local testing)"
	@echo "  make check-e2e       - Run end-to-end tests (requires WASM dependencies)"
	@echo "  make check-coverage  - Generate code coverage report (requires cargo-tarpaulin)"
	@echo "  make gen-levels      - Regenerate src/levels_data.rs from levels/*.json"
	@echo "  make check-levels    - Validate levels/*.json and check levels_data.rs is up to date"

# Run all verification checks (E2E tests excluded by default due to dependency constraints)
verify: check-fmt check-clippy check-test check-build check-wasm-build
	@echo "$(GREEN)✓ All core checks passed!$(NC)"
	@echo "$(YELLOW)Note: E2E tests skipped (run 'make check-e2e' separately if WASM dependencies are available)$(NC)"

# Run all checks including E2E tests (requires wasm-bindgen and web-sys dependencies)
verify-all: check-fmt check-clippy check-test check-build check-e2e
	@echo "$(GREEN)✓ All checks passed!$(NC)"

# Test Suite - runs all tests including doc tests
check-test:
	@echo "$(YELLOW)Running test suite...$(NC)"
	cargo test --verbose --all-features
	cargo test --doc --verbose
	@echo "$(GREEN)✓ Test suite passed$(NC)"

# Clippy - linting with warnings as errors
check-clippy:
	@echo "$(YELLOW)Running clippy (linting)...$(NC)"
	cargo clippy --all-targets --all-features -- -D warnings
	@echo "$(GREEN)✓ Clippy passed$(NC)"

# Rustfmt - code formatting check
check-fmt:
	@echo "$(YELLOW)Checking code formatting...$(NC)"
	cargo fmt --all -- --check
	@echo "$(GREEN)✓ Formatting check passed$(NC)"

# Build - release build
check-build:
	@echo "$(YELLOW)Building release library...$(NC)"
	cargo build --release --lib --verbose
	@echo "$(GREEN)✓ Build passed$(NC)"

# WASM Build Check - verify wasm32 target compiles (catches wasm-specific issues)
check-wasm-build:
	@echo "$(YELLOW)Checking WASM compilation...$(NC)"
	@rustup target list --installed | grep -q wasm32-unknown-unknown || (echo "Installing wasm32-unknown-unknown target..." && rustup target add wasm32-unknown-unknown)
	cargo build --lib --target wasm32-unknown-unknown
	@echo "$(GREEN)✓ WASM build check passed$(NC)"

# Build WASM - build WASM and generate JavaScript glue for local testing
build-wasm:
	@echo "$(YELLOW)Building WASM for local testing...$(NC)"
	@rustup target list --installed | grep -q wasm32-unknown-unknown || (echo "Installing wasm32-unknown-unknown target..." && rustup target add wasm32-unknown-unknown)
	cargo build --release --target wasm32-unknown-unknown
	@echo "$(YELLOW)Generating wasm-bindgen JavaScript glue...$(NC)"
	@if ! which wasm-bindgen > /dev/null; then \
		WASM_BINDGEN_VERSION=$$(grep -A 2 'name = "wasm-bindgen"' Cargo.lock | grep '^version = ' | head -1 | sed 's/version = "\(.*\)"/\1/'); \
		echo "Installing wasm-bindgen-cli v$$WASM_BINDGEN_VERSION to match Cargo.lock..."; \
		cargo install wasm-bindgen-cli --version $$WASM_BINDGEN_VERSION; \
	fi
	wasm-bindgen target/wasm32-unknown-unknown/release/open_miami.wasm --out-dir . --target web --no-typescript
	@echo "$(GREEN)✓ WASM build complete! Files generated:$(NC)"
	@echo "  - open_miami.js"
	@echo "  - open_miami_bg.wasm"
	@echo "$(YELLOW)You can now open index.html in a web browser (via a local web server)$(NC)"

# E2E Tests - end-to-end tests with Playwright
# IMPORTANT: Always run via 'make check-e2e' to ensure proper timeout enforcement
# Running tests directly without timeout can cause Claude Code instances to hang
check-e2e:
	@echo "$(YELLOW)Running end-to-end tests...$(NC)"
	@echo "Building WASM..."
	cargo build --release --target wasm32-unknown-unknown
	@echo "Generating wasm-bindgen JavaScript glue..."
	@if ! which wasm-bindgen > /dev/null; then \
		WASM_BINDGEN_VERSION=$$(grep -A 2 'name = "wasm-bindgen"' Cargo.lock | grep '^version = ' | head -1 | sed 's/version = "\(.*\)"/\1/'); \
		echo "Installing wasm-bindgen-cli v$$WASM_BINDGEN_VERSION to match Cargo.lock..."; \
		cargo install wasm-bindgen-cli --version $$WASM_BINDGEN_VERSION; \
	fi
	wasm-bindgen target/wasm32-unknown-unknown/release/open_miami.wasm --out-dir . --target web --no-typescript
	@echo "Installing E2E test dependencies (via Bun)..."
	cd tests/e2e && bun install
	@echo "Installing the Chromium browser (with system deps when possible)..."
	cd tests/e2e && (bunx playwright install --with-deps chromium || bunx playwright install chromium)
	@echo "Ensuring browser system libraries are present (rootless fallback)..."
	cd tests/e2e && ./setup-browser-deps.sh
	@echo "Running E2E tests with 60-second timeout..."
	cd tests/e2e && mkdir -p test-results && \
		LD_LIBRARY_PATH="$$(find "$$PWD/playwright-deps/libs" -name '*.so*' -printf '%h\n' 2>/dev/null | sort -u | paste -sd:):$${LD_LIBRARY_PATH:-}" \
		timeout 60 bunx playwright test
	@echo "$(GREEN)✓ E2E tests passed$(NC)"

# Levels - compile the floor/scenario JSON (levels/*.json, written by the
# level editor) into static Rust data. Python 3 stdlib only.
gen-levels:
	@echo "$(YELLOW)Generating src/levels_data.rs from levels/*.json...$(NC)"
	python3 tools/gen_levels.py
	@echo "$(GREEN)✓ Levels generated$(NC)"

# Levels check - validate the JSON and make sure the generated file is current
check-levels:
	@echo "$(YELLOW)Validating levels/*.json...$(NC)"
	python3 tools/gen_levels.py --check
	@echo "$(GREEN)✓ Levels valid and up to date$(NC)"

# Code Coverage - requires cargo-tarpaulin (optional check)
check-coverage:
	@echo "$(YELLOW)Generating code coverage...$(NC)"
	@which cargo-tarpaulin > /dev/null || (echo "$(RED)cargo-tarpaulin not installed. Run: cargo install cargo-tarpaulin$(NC)" && exit 1)
	cargo tarpaulin --verbose --all-features --workspace --timeout 120 --out xml
	@echo "$(GREEN)✓ Coverage report generated$(NC)"
