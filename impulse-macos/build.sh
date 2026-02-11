#!/usr/bin/env bash
set -euo pipefail

# Build script for the Impulse macOS frontend.
#
# This script must be run on macOS from the workspace root (the directory
# containing the top-level Cargo.toml and the impulse-macos/ directory).
#
# Steps:
#   1. Build the Rust FFI static library (impulse-ffi).
#   2. Copy vendored Monaco editor assets into the Swift package resources.
#   3. Build the Swift macOS app with SwiftPM.
#   4. Create a proper .app bundle.
#   5. Optionally create a .dmg disk image (with --dmg flag).
#
# Usage:
#   ./impulse-macos/build.sh               # build .app bundle
#   ./impulse-macos/build.sh --dmg         # build .app + .dmg
#   ./impulse-macos/build.sh --release     # same as default (release build)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CREATE_DMG=false
for arg in "$@"; do
    case "$arg" in
        --dmg) CREATE_DMG=true ;;
    esac
done

cd "${WORKSPACE_ROOT}"

# ── Preflight ─────────────────────────────────────────────────────────

if [[ "$(uname)" != "Darwin" ]]; then
    echo "ERROR: This script must be run on macOS." >&2
    exit 1
fi

if [[ ! -f Cargo.toml ]]; then
    echo "ERROR: Must be run from the workspace root (expected Cargo.toml)." >&2
    exit 1
fi

if [[ ! -d impulse-macos ]]; then
    echo "ERROR: impulse-macos directory not found." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "ERROR: cargo not found. Install Rust: https://rustup.rs" >&2
    exit 1
fi

if ! command -v swift >/dev/null 2>&1; then
    echo "ERROR: swift not found. Install Xcode command line tools: xcode-select --install" >&2
    exit 1
fi

# ── Version detection ─────────────────────────────────────────────────

# Read version from impulse-core/Cargo.toml (single source of truth).
VERSION=$(grep -m1 '^version' impulse-core/Cargo.toml | sed 's/.*"\(.*\)"/\1/')
echo "Building Impulse v${VERSION} for macOS..."

# ── Step 1: Build Rust FFI static library ─────────────────────────────

echo "==> Building impulse-ffi (Rust static library)..."
cargo build --release -p impulse-ffi

if [[ ! -f target/release/libimpulse_ffi.a ]]; then
    echo "ERROR: target/release/libimpulse_ffi.a not found after build." >&2
    exit 1
fi
echo "    OK: target/release/libimpulse_ffi.a"

# ── Step 2: Copy Monaco assets ────────────────────────────────────────

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

# ── Step 3: Build Swift app ───────────────────────────────────────────

echo "==> Building ImpulseApp (Swift)..."
cd impulse-macos
swift build -c release
cd "${WORKSPACE_ROOT}"

SWIFT_BIN="impulse-macos/.build/release/ImpulseApp"
if [[ ! -f "${SWIFT_BIN}" ]]; then
    echo "ERROR: Swift binary not found at ${SWIFT_BIN}." >&2
    exit 1
fi
echo "    OK: ${SWIFT_BIN}"

# ── Step 4: Create .app bundle ────────────────────────────────────────

echo "==> Creating Impulse.app bundle..."

APP_DIR="dist/Impulse.app"
CONTENTS="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS}/MacOS"
RESOURCES="${CONTENTS}/Resources"

rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES}"

# Copy binary
cp "${SWIFT_BIN}" "${MACOS_DIR}/Impulse"

# Copy SwiftPM bundle resources (Monaco assets, etc.)
BUNDLE_RESOURCES="impulse-macos/.build/release/ImpulseApp_ImpulseApp.bundle"
if [[ -d "${BUNDLE_RESOURCES}" ]]; then
    cp -r "${BUNDLE_RESOURCES}" "${MACOS_DIR}/"
fi

# Generate Info.plist
cat > "${CONTENTS}/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Impulse</string>
    <key>CFBundleDisplayName</key>
    <string>Impulse</string>
    <key>CFBundleIdentifier</key>
    <string>dev.impulse.Impulse</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>Impulse</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticTermination</key>
    <false/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
PLIST

# Generate .icns from the SVG logo if possible, otherwise skip.
if [[ -f assets/impulse-logo.svg ]]; then
    ICONSET_DIR=$(mktemp -d)/AppIcon.iconset
    mkdir -p "${ICONSET_DIR}"

    # Try to convert SVG to PNG at various sizes using sips + rsvg-convert / qlmanage.
    if command -v rsvg-convert >/dev/null 2>&1; then
        for size in 16 32 64 128 256 512 1024; do
            rsvg-convert -w ${size} -h ${size} assets/impulse-logo.svg \
                -o "${ICONSET_DIR}/icon_${size}x${size}.png" 2>/dev/null || true
        done
        # Create @2x variants
        for size in 16 32 128 256 512; do
            double=$((size * 2))
            if [[ -f "${ICONSET_DIR}/icon_${double}x${double}.png" ]]; then
                cp "${ICONSET_DIR}/icon_${double}x${double}.png" \
                   "${ICONSET_DIR}/icon_${size}x${size}@2x.png"
            fi
        done
        if command -v iconutil >/dev/null 2>&1; then
            iconutil -c icns "${ICONSET_DIR}" -o "${RESOURCES}/AppIcon.icns" 2>/dev/null || true
        fi
    fi

    if [[ ! -f "${RESOURCES}/AppIcon.icns" ]]; then
        echo "    Note: Could not generate .icns (install rsvg-convert for app icon)"
    else
        echo "    OK: AppIcon.icns"
    fi

    rm -rf "$(dirname "${ICONSET_DIR}")"
fi

echo "    OK: ${APP_DIR}"

# ── Step 5: Create .dmg (optional) ────────────────────────────────────

if [[ "${CREATE_DMG}" == true ]]; then
    echo "==> Creating Impulse-${VERSION}.dmg..."

    DMG_NAME="Impulse-${VERSION}.dmg"
    DMG_PATH="dist/${DMG_NAME}"
    DMG_STAGING=$(mktemp -d)

    cp -r "${APP_DIR}" "${DMG_STAGING}/"

    # Create a symlink to /Applications for drag-to-install.
    ln -s /Applications "${DMG_STAGING}/Applications"

    # Create the DMG.
    hdiutil create -volname "Impulse" \
        -srcfolder "${DMG_STAGING}" \
        -ov -format UDZO \
        "${DMG_PATH}"

    rm -rf "${DMG_STAGING}"
    echo "    OK: ${DMG_PATH}"
fi

# ── Summary ───────────────────────────────────────────────────────────

echo ""
echo "==> Build complete."
echo "    App bundle: ${APP_DIR}"
if [[ "${CREATE_DMG}" == true ]]; then
    echo "    Disk image: dist/Impulse-${VERSION}.dmg"
fi
echo ""
echo "To run: open ${APP_DIR}"
