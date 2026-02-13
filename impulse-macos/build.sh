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
#   4b. Optionally codesign with Developer ID (with --sign flag).
#   5. Optionally create a .dmg disk image (with --dmg flag).
#   6. Optionally notarize with Apple (with --notarize flag).
#
# Usage:
#   ./impulse-macos/build.sh                           # build .app bundle
#   ./impulse-macos/build.sh --dmg                     # build .app + .dmg
#   ./impulse-macos/build.sh --sign                    # build + codesign
#   ./impulse-macos/build.sh --sign --notarize --dmg   # build + sign + notarize + .dmg
#   ./impulse-macos/build.sh --release                 # same as default (release build)
#
# Environment variables for signing:
#   IMPULSE_SIGN_IDENTITY  — codesign identity, e.g. "Developer ID Application: Name (TEAM_ID)"
#                            Auto-detected if not set.
#   IMPULSE_NOTARY_KEY     — path to App Store Connect API key .p8 file
#   IMPULSE_NOTARY_KEY_ID  — API key ID
#   IMPULSE_NOTARY_ISSUER  — API key issuer ID

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CREATE_DMG=false
SIGN=false
NOTARIZE=false
for arg in "$@"; do
    case "$arg" in
        --dmg) CREATE_DMG=true ;;
        --sign) SIGN=true ;;
        --notarize) NOTARIZE=true; SIGN=true ;;
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

# ── Signing preflight ────────────────────────────────────────────────

if [[ "${SIGN}" == true ]]; then
    # Auto-detect signing identity if not set
    if [[ -z "${IMPULSE_SIGN_IDENTITY:-}" ]]; then
        IMPULSE_SIGN_IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed 's/.*"\(.*\)"/\1/' || true)
        if [[ -z "${IMPULSE_SIGN_IDENTITY}" ]]; then
            echo "ERROR: No Developer ID Application certificate found in keychain." >&2
            echo "" >&2
            echo "To set up code signing:" >&2
            echo "  1. Download your Developer ID Application certificate from developer.apple.com" >&2
            echo "  2. Double-click to install in Keychain, or run:" >&2
            echo "     security import DeveloperIDApplication.p12 -k login.keychain" >&2
            echo "  3. Set IMPULSE_SIGN_IDENTITY to your identity, e.g.:" >&2
            echo '     export IMPULSE_SIGN_IDENTITY="Developer ID Application: Your Name (TEAM_ID)"' >&2
            exit 1
        fi
        echo "Auto-detected signing identity: ${IMPULSE_SIGN_IDENTITY}"
    fi

    if [[ "${NOTARIZE}" == true ]]; then
        if [[ -z "${IMPULSE_NOTARY_KEY:-}" || -z "${IMPULSE_NOTARY_KEY_ID:-}" || -z "${IMPULSE_NOTARY_ISSUER:-}" ]]; then
            echo "ERROR: Notarization requires the following environment variables:" >&2
            echo "  IMPULSE_NOTARY_KEY     — path to App Store Connect API key .p8 file" >&2
            echo "  IMPULSE_NOTARY_KEY_ID  — API key ID" >&2
            echo "  IMPULSE_NOTARY_ISSUER  — API key issuer ID" >&2
            echo "" >&2
            echo "To set up notarization:" >&2
            echo "  1. Go to appstoreconnect.apple.com > Users and Access > Integrations > App Store Connect API" >&2
            echo "  2. Generate a key with 'Developer' access" >&2
            echo "  3. Download the .p8 file (only available once)" >&2
            echo "  4. Note the Key ID and Issuer ID" >&2
            echo "  5. Set environment variables, e.g. in ~/.zshrc:" >&2
            echo '     export IMPULSE_NOTARY_KEY=~/private/AuthKey_XXXXXXXXXX.p8' >&2
            echo '     export IMPULSE_NOTARY_KEY_ID=XXXXXXXXXX' >&2
            echo '     export IMPULSE_NOTARY_ISSUER=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx' >&2
            exit 1
        fi
        if [[ ! -f "${IMPULSE_NOTARY_KEY}" ]]; then
            echo "ERROR: Notary key file not found: ${IMPULSE_NOTARY_KEY}" >&2
            exit 1
        fi
    fi
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

# ── Step 2b: Copy file icons ─────────────────────────────────────

echo "==> Copying file icons..."

ICONS_SRC="assets/icons"
ICONS_DST="impulse-macos/Sources/ImpulseApp/Resources/icons"

mkdir -p "${ICONS_DST}"
cp -f "${ICONS_SRC}"/*.svg "${ICONS_DST}/"
echo "    OK: File icons copied to ${ICONS_DST}"

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

# ── Step 4b: Code Signing (optional) ─────────────────────────────────

if [[ "${SIGN}" == true ]]; then
    echo "==> Signing Impulse.app with Developer ID..."

    ENTITLEMENTS="impulse-macos/Impulse.entitlements"

    # Sign embedded bundles first (inside-out signing order)
    if [[ -d "${MACOS_DIR}/ImpulseApp_ImpulseApp.bundle" ]]; then
        echo "    Signing resource bundle..."
        codesign --force --options runtime \
            --entitlements "${ENTITLEMENTS}" \
            --sign "${IMPULSE_SIGN_IDENTITY}" \
            --timestamp \
            "${MACOS_DIR}/ImpulseApp_ImpulseApp.bundle"
    fi

    # Sign the main app bundle
    echo "    Signing app bundle..."
    codesign --force --options runtime \
        --entitlements "${ENTITLEMENTS}" \
        --sign "${IMPULSE_SIGN_IDENTITY}" \
        --timestamp \
        "${APP_DIR}"

    # Verify
    echo "    Verifying signature..."
    codesign --verify --deep --strict "${APP_DIR}"
    spctl --assess --type exec "${APP_DIR}"
    echo "    OK: Code signing verified"
fi

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

    # Sign the DMG if signing is enabled
    if [[ "${SIGN}" == true ]]; then
        echo "    Signing DMG..."
        codesign --force --sign "${IMPULSE_SIGN_IDENTITY}" --timestamp "${DMG_PATH}"
    fi

    echo "    OK: ${DMG_PATH}"
fi

# ── Step 6: Notarization (optional) ──────────────────────────────────

if [[ "${NOTARIZE}" == true ]]; then
    echo "==> Notarizing with Apple..."

    # Determine what to submit: prefer DMG, fall back to zipped .app
    if [[ "${CREATE_DMG}" == true && -f "dist/Impulse-${VERSION}.dmg" ]]; then
        NOTARIZE_TARGET="dist/Impulse-${VERSION}.dmg"
    else
        echo "    Creating zip for notarization..."
        NOTARIZE_TARGET="dist/Impulse-${VERSION}.zip"
        ditto -c -k --keepParent "${APP_DIR}" "${NOTARIZE_TARGET}"
    fi

    echo "    Submitting ${NOTARIZE_TARGET} for notarization..."
    xcrun notarytool submit "${NOTARIZE_TARGET}" \
        --key "${IMPULSE_NOTARY_KEY}" \
        --key-id "${IMPULSE_NOTARY_KEY_ID}" \
        --issuer "${IMPULSE_NOTARY_ISSUER}" \
        --wait

    echo "    Stapling notarization ticket..."
    xcrun stapler staple "${APP_DIR}"

    if [[ "${CREATE_DMG}" == true && -f "dist/Impulse-${VERSION}.dmg" ]]; then
        xcrun stapler staple "dist/Impulse-${VERSION}.dmg"
    fi

    # Clean up temporary zip if we created one
    if [[ "${CREATE_DMG}" != true && -f "dist/Impulse-${VERSION}.zip" ]]; then
        rm -f "dist/Impulse-${VERSION}.zip"
    fi

    echo "    OK: Notarization complete"
fi

# ── Summary ───────────────────────────────────────────────────────────

echo ""
echo "==> Build complete."
echo "    App bundle: ${APP_DIR}"
if [[ "${SIGN}" == true ]]; then
    echo "    Signed:     yes (${IMPULSE_SIGN_IDENTITY})"
fi
if [[ "${NOTARIZE}" == true ]]; then
    echo "    Notarized:  yes"
fi
if [[ "${CREATE_DMG}" == true ]]; then
    echo "    Disk image: dist/Impulse-${VERSION}.dmg"
fi
echo ""
echo "To run: open ${APP_DIR}"
