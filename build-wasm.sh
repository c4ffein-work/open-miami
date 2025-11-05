#!/bin/bash

echo "🔥 Building Open Miami for the web... 🔥"

# Check if wasm32 target is installed
if ! rustup target list | grep -q "wasm32-unknown-unknown (installed)"; then
    echo "Installing wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown
fi

# Check if wasm-bindgen-cli is installed
if ! command -v wasm-bindgen &> /dev/null; then
    echo "Installing wasm-bindgen-cli..."
    cargo install wasm-bindgen-cli
fi

# Build the project
echo "Building WASM binary..."
cargo build --release --target wasm32-unknown-unknown

# Generate wasm-bindgen JavaScript glue code
echo "Generating wasm-bindgen JavaScript glue..."
wasm-bindgen target/wasm32-unknown-unknown/release/open_miami.wasm \
    --out-dir . \
    --target web \
    --no-typescript

echo ""
echo "✅ Build complete!"
echo ""
echo "To run the game:"
echo "  1. Start a local server:"
echo "     python3 -m http.server 8000"
echo "  2. Open http://localhost:8000 in your browser"
echo ""
echo "Files ready:"
echo "  - open_miami_bg.wasm (generated WASM binary)"
echo "  - open_miami.js (generated wasm-bindgen glue)"
echo "  - index.html"
