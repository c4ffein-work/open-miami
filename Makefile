.PHONY: verify check-test check-clippy check-fmt check-build check-e2e check-coverage help

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
	@echo "  make check-e2e       - Run end-to-end tests (requires WASM dependencies)"
	@echo "  make check-coverage  - Generate code coverage report (requires cargo-tarpaulin)"

# Run all verification checks (E2E tests excluded by default due to dependency constraints)
verify: check-fmt check-clippy check-test check-build
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
	@echo "$(YELLOW)Building release binary...$(NC)"
	cargo build --release --verbose
	@echo "$(GREEN)✓ Build passed$(NC)"

# E2E Tests - end-to-end tests with Playwright
check-e2e:
	@echo "$(YELLOW)Running end-to-end tests...$(NC)"
	@echo "Building WASM..."
	cargo build --release --target wasm32-unknown-unknown
	cp target/wasm32-unknown-unknown/release/open_miami.wasm .
	@echo "Installing E2E test dependencies..."
	cd tests/e2e && npm install && npx playwright install --with-deps chromium
	@echo "Running E2E tests..."
	cd tests/e2e && mkdir -p test-results && npm test
	@echo "$(GREEN)✓ E2E tests passed$(NC)"

# Code Coverage - requires cargo-tarpaulin (optional check)
check-coverage:
	@echo "$(YELLOW)Generating code coverage...$(NC)"
	@which cargo-tarpaulin > /dev/null || (echo "$(RED)cargo-tarpaulin not installed. Run: cargo install cargo-tarpaulin$(NC)" && exit 1)
	cargo tarpaulin --verbose --all-features --workspace --timeout 120 --out xml
	@echo "$(GREEN)✓ Coverage report generated$(NC)"
