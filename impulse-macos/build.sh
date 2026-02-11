#!/usr/bin/env bash
set -euo pipefail

# Build script for the Impulse macOS frontend.
#
# This script must be run from the workspace root (the directory containing
# the top-level Cargo.toml and the impulse-macos/ directory).
#
# Steps:
#   1. Build the Rust FFI static library (impulse-ffi).
#   2. Copy vendored Monaco editor assets into the Swift package resources.
#   3. Build the Swift macOS app with SwiftPM.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

cd "${WORKSPACE_ROOT}"

# Verify we're in the right place.
if [[ ! -f Cargo.toml ]]; then
    echo "ERROR: Must be run from the workspace root (expected Cargo.toml)." >&2
    exit 1
fi

if [[ ! -d impulse-macos ]]; then
    echo "ERROR: impulse-macos directory not found." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 1: Build the Rust FFI static library
# ---------------------------------------------------------------------------
echo "==> Building impulse-ffi (Rust static library)..."

cargo build --release -p impulse-ffi

if [[ ! -f target/release/libimpulse_ffi.a ]]; then
    echo "ERROR: target/release/libimpulse_ffi.a not found after build." >&2
    exit 1
fi

echo "    OK: target/release/libimpulse_ffi.a"

# ---------------------------------------------------------------------------
# Step 2: Copy Monaco assets into the Swift package resources
# ---------------------------------------------------------------------------
echo "==> Copying Monaco editor assets..."

MONACO_SRC="impulse-editor/vendor/monaco"
EDITOR_HTML_SRC="impulse-editor/web/editor.html"
MONACO_DST="impulse-macos/Sources/ImpulseApp/Resources/monaco"

if [[ ! -d "${MONACO_SRC}" ]]; then
    echo "ERROR: Monaco vendor directory not found at ${MONACO_SRC}." >&2
    echo "       Run scripts/vendor-monaco.sh first." >&2
    exit 1
fi

if [[ ! -f "${EDITOR_HTML_SRC}" ]]; then
    echo "ERROR: editor.html not found at ${EDITOR_HTML_SRC}." >&2
    exit 1
fi

mkdir -p "${MONACO_DST}"
cp -r "${MONACO_SRC}"/* "${MONACO_DST}/"
cp "${EDITOR_HTML_SRC}" "${MONACO_DST}/"

echo "    OK: Monaco assets copied to ${MONACO_DST}"

# ---------------------------------------------------------------------------
# Step 3: Build the Swift macOS app
# ---------------------------------------------------------------------------
echo "==> Building ImpulseApp (Swift)..."

cd impulse-macos
swift build -c release

echo ""
echo "==> Build complete."
echo "    Binary: impulse-macos/.build/release/ImpulseApp"
