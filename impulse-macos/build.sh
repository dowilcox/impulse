#!/usr/bin/env bash
set -euo pipefail

# Build script for the Impulse macOS frontend.
#
# This script must be run on macOS (on Linux it builds the Rust app
# instead; see below) from the workspace root (the directory
# containing the top-level Cargo.toml and the impulse-macos/ directory).
#
# Steps:
#   1. Build the Rust FFI static library (impulse-ffi).
#   2. Copy vendored Monaco editor assets into the Swift package resources.
#   3. Build the Swift macOS app with SwiftPM.
#   4. Create a proper .app bundle.
#   4b. Optionally codesign with Developer ID (with --sign flag).
#   4c. Optionally notarize and staple the app (with --notarize flag).
#   5. Optionally create a .dmg disk image (with --dmg flag), notarized and
#      stapled too with --notarize.
#
# Usage:
#   ./impulse-macos/build.sh                           # build .app bundle
#   ./impulse-macos/build.sh --dmg                     # build .app + .dmg
#   ./impulse-macos/build.sh --sign                    # build + codesign
#   ./impulse-macos/build.sh --sign --notarize --dmg   # build + sign + notarize + .dmg
#   ./impulse-macos/build.sh --dev                     # build dev variant (separate bundle ID)
#   ./impulse-macos/build.sh --release                 # same as default (release build)
#
# On Linux (rust branch):
#   ./impulse-macos/build.sh --dev                     # debug build: target/debug/impulse-app
#   ./impulse-macos/build.sh                           # release packages (scripts/package-linux.sh)
# (The Rust app's snapshot build on the Mac, for capture.py and run.py with
# IMPULSE_SNAPSHOT_APP, isn't made here:
#   cargo build -p impulse-app -p impulse-cli --features impulse-app/snapshot)
#
# Environment variables for signing:
#   IMPULSE_SIGN_IDENTITY  — codesign identity, e.g. "Developer ID Application: Name (TEAM_ID)"
#                            Auto-detected if not set.
#   IMPULSE_NOTARY_KEY     — path to App Store Connect API key .p8 file
#   IMPULSE_NOTARY_KEY_ID  — API key ID
#   IMPULSE_NOTARY_ISSUER  — API key issuer ID

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=../scripts/lib/macos-bundle.sh
source "${WORKSPACE_ROOT}/scripts/lib/macos-bundle.sh"

CREATE_DMG=false
SIGN=false
NOTARIZE=false
DEV_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --dmg) CREATE_DMG=true ;;
        --sign) SIGN=true ;;
        --notarize) NOTARIZE=true; SIGN=true ;;
        --dev) DEV_BUILD=true ;;
    esac
done

cd "${WORKSPACE_ROOT}"

# ── Linux: the Rust app ───────────────────────────────────────────────
#
# On Linux there is no Swift app; the GPUI app (rust branch) is built
# instead. --dev is the debug build that scripts/docs/capture.py and
# scripts/smoke/run.py run by default (target/debug/impulse-app); a plain
# build is the release packages from scripts/package-linux.sh.

if [[ "$(uname)" == "Linux" ]]; then
    if [[ ! -d impulse-app ]]; then
        echo "ERROR: impulse-app not found. The Linux app is on the rust branch." >&2
        exit 1
    fi
    if [[ "${CREATE_DMG}" == true || "${SIGN}" == true ]]; then
        echo "ERROR: --dmg, --sign and --notarize are macOS only." >&2
        exit 1
    fi
    if [[ "${DEV_BUILD}" == true ]]; then
        cargo build -p impulse-app -p impulse-cli
        echo ""
        echo "Run it with:"
        echo "  ${WORKSPACE_ROOT}/target/debug/impulse-app [--cwd <folder>] [--open <file>]"
        exit 0
    fi
    exec "${WORKSPACE_ROOT}/scripts/package-linux.sh"
fi

# ── Preflight ─────────────────────────────────────────────────────────

if [[ "$(uname)" != "Darwin" ]]; then
    echo "ERROR: This script must be run on macOS or Linux." >&2
    exit 1
fi

# Apple silicon only. A shell running under Rosetta reports x86_64, and
# would otherwise quietly produce an Intel build.
if [[ "$(uname -m)" != "arm64" ]]; then
    echo "ERROR: Impulse builds for Apple silicon only (this shell runs as $(uname -m))." >&2
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

if [[ "${CREATE_DMG}" == true ]]; then
    if ! command -v create-dmg >/dev/null 2>&1; then
        echo "ERROR: create-dmg not found. Install it with: brew install create-dmg" >&2
        exit 1
    fi
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

# ── Dev mode configuration ────────────────────────────────────────────

if [[ "${DEV_BUILD}" == true ]]; then
    APP_NAME="Impulse Dev"
    BUNDLE_ID="dev.impulse.Impulse.Devel"
    echo "Building in DEV mode (bundle ID: ${BUNDLE_ID})"
else
    APP_NAME="Impulse"
    BUNDLE_ID="dev.impulse.Impulse"
fi

# ── Version detection ─────────────────────────────────────────────────

# Read version from the top-level VERSION file (single source of truth).
VERSION=$(cat VERSION)
echo "Building Impulse v${VERSION} for macOS..."

# ── Step 1: Build Rust FFI static library ─────────────────────────────

# Match the Swift package's deployment target so the Rust-compiled
# objects don't trigger "built for newer macOS version than being
# linked" linker warnings at link time.
export MACOSX_DEPLOYMENT_TARGET=26.0

echo "==> Building vendored libgit2 (cached)..."
./scripts/build-libgit2.sh

echo "==> Building impulse-ffi (Rust static library)..."
cargo build --release -p impulse-ffi

if [[ ! -f target/release/libimpulse_ffi.a ]]; then
    echo "ERROR: target/release/libimpulse_ffi.a not found after build." >&2
    exit 1
fi
echo "    OK: target/release/libimpulse_ffi.a"

# ── Step 2: Copy Monaco assets ────────────────────────────────────────

echo "==> Copying Monaco editor assets..."

MONACO_SRC="vendor/monaco"
EDITOR_HTML_SRC="impulse-macos/web/editor.html"
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

FONTS_SRC="$(dirname "${MONACO_SRC}")/fonts"
HIGHLIGHT_SRC="$(dirname "${MONACO_SRC}")/highlight"
WEB_SRC="$(dirname "${EDITOR_HTML_SRC}")"

# Start clean: chunk names change between Monaco versions, and copying over
# the old folder would ship both.
rm -rf "${MONACO_DST}"
mkdir -p "${MONACO_DST}"
cp -r "${MONACO_SRC}"/* "${MONACO_DST}/"
cp "${WEB_SRC}/editor.html" "${WEB_SRC}/editor.js" "${MONACO_DST}/"

# Fonts (editor @font-face + terminal font installation) and highlight.js
# (markdown preview, review syntax colors) mirror the layout the old Rust
# extraction produced.
mkdir -p "${MONACO_DST}/fonts" "${MONACO_DST}/highlight"
cp -r "${FONTS_SRC}"/* "${MONACO_DST}/fonts/"
cp -r "${HIGHLIGHT_SRC}"/* "${MONACO_DST}/highlight/"
# Vim mode for the editor (optional; vendored by scripts/vendor-monaco-vim.sh).
VIM_SRC="$(dirname "${MONACO_SRC}")/monaco-vim"
if [[ -f "${VIM_SRC}/monaco-vim.umd.js" ]]; then
    mkdir -p "${MONACO_DST}/vim"
    cp "${VIM_SRC}/monaco-vim.umd.js" "${MONACO_DST}/vim/"
fi
echo "    OK: Monaco assets copied to ${MONACO_DST}"

# ── Step 2b: Copy file icons ─────────────────────────────────────

echo "==> Copying file icons..."

ICONS_SRC="assets/icons"
ICONS_DST="impulse-macos/Sources/ImpulseApp/Resources/icons"

mkdir -p "${ICONS_DST}"
cp -f "${ICONS_SRC}"/*.svg "${ICONS_DST}/"

# Copy material file/folder icons
MATERIAL_SRC="assets/icons/material"
MATERIAL_DST="${ICONS_DST}/material"
mkdir -p "${MATERIAL_DST}"
cp -f "${MATERIAL_SRC}"/*.svg "${MATERIAL_DST}/"
cp -f "${MATERIAL_SRC}"/icon-mapping.json "${MATERIAL_DST}/"
cp -f "${MATERIAL_SRC}"/LICENSE "${MATERIAL_DST}/"
echo "    OK: File icons copied to ${ICONS_DST} (+ material/)"

# ── Step 3: Build Swift app ───────────────────────────────────────────

# SwiftPM does not detect changes to linked static libraries (.a files).
# If libimpulse_ffi.a is newer than the last Swift build output, touch a
# Swift source file to force SwiftPM to re-link.
SWIFT_BIN="impulse-macos/.build/release/ImpulseApp"
if [[ -f "${SWIFT_BIN}" && "target/release/libimpulse_ffi.a" -nt "${SWIFT_BIN}" ]]; then
    echo "    Rust FFI is newer than Swift binary; forcing SwiftPM relink..."
    rm -f "${SWIFT_BIN}"
    touch impulse-macos/Sources/ImpulseApp/ImpulseApp.swift
fi

echo "==> Building ImpulseApp (Swift)..."
cd impulse-macos
# Dependencies come exactly from the committed Package.resolved.
./swiftw build -c release --force-resolved-versions
cd "${WORKSPACE_ROOT}"

SWIFT_BIN="impulse-macos/.build/release/ImpulseApp"
if [[ ! -f "${SWIFT_BIN}" ]]; then
    echo "ERROR: Swift binary not found at ${SWIFT_BIN}." >&2
    exit 1
fi
if [[ "$(lipo -archs "${SWIFT_BIN}")" != "arm64" ]]; then
    echo "ERROR: ${SWIFT_BIN} is $(lipo -archs "${SWIFT_BIN}"), expected arm64 only." >&2
    exit 1
fi
echo "    OK: ${SWIFT_BIN}"

# ── Step 4: Create .app bundle ────────────────────────────────────────

echo "==> Creating ${APP_NAME}.app bundle..."

APP_DIR="dist/${APP_NAME}.app"
CONTENTS="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS}/MacOS"
RESOURCES="${CONTENTS}/Resources"

rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES}"

# Copy binary
cp "${SWIFT_BIN}" "${MACOS_DIR}/${APP_NAME}"

# The `impulse` command-line tool; the app puts Contents/Resources/bin on
# PATH inside its terminals.
CLI_BIN="impulse-macos/.build/release/impulse"
if [[ -f "${CLI_BIN}" ]]; then
    mkdir -p "${RESOURCES}/bin"
    cp "${CLI_BIN}" "${RESOURCES}/bin/impulse"
fi

# Copy the SwiftPM resource bundles into Contents/Resources/ — the standard
# macOS location (codesign rejects bundles beside the executable):
#   ImpulseApp_ImpulseApp.bundle — Monaco, web assets, icons
#   ImpulseApp_ImpulseKit.bundle — themes and shell integration scripts
# Both are required: without them the app can't find its themes or shell
# integration and crashes on any Mac other than the one that built it.
#
# SwiftPM's bundles are plain folders. codesign treats a .bundle as a nested
# bundle and needs an Info.plist in it; and one whose files sit in a
# top-level Resources/ folder (ImpulseKit's) reads to it as a malformed
# macOS bundle, so that one gets the real layout: Contents/Info.plist and
# Contents/Resources/. Lookups relative to a bundle's resources (like
# "Resources/Themes") match either way.
RESOURCE_BUNDLES=(ImpulseApp_ImpulseApp ImpulseApp_ImpulseKit)
for bundle in "${RESOURCE_BUNDLES[@]}"; do
    BUNDLE_SRC="impulse-macos/.build/release/${bundle}.bundle"
    if [[ ! -d "${BUNDLE_SRC}" ]]; then
        echo "ERROR: resource bundle ${BUNDLE_SRC} not found." >&2
        exit 1
    fi
    cp -r "${BUNDLE_SRC}" "${RESOURCES}/"
    BUNDLE_DIR="${RESOURCES}/${bundle}.bundle"
    INFO_DIR="${BUNDLE_DIR}"
    if [[ -d "${BUNDLE_DIR}/Resources" ]]; then
        mkdir -p "${BUNDLE_DIR}/Contents/Resources"
        mv "${BUNDLE_DIR}/Resources" "${BUNDLE_DIR}/Contents/Resources/Resources"
        INFO_DIR="${BUNDLE_DIR}/Contents"
    fi
    cat > "${INFO_DIR}/Info.plist" << RPLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>dev.impulse.Impulse.resources.${bundle#ImpulseApp_}</string>
    <key>CFBundleName</key>
    <string>${bundle#ImpulseApp_} Resources</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
</dict>
</plist>
RPLIST
done

# Info.plist, from assets/macos/Info.plist.in. The Swift app also offers
# Finder's "New Impulse Workspace Here" service (AppDelegate's
# openWorkspaceFromService).
SERVICES_KEYS="    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>New Impulse Workspace Here</string>
            </dict>
            <key>NSMessage</key>
            <string>openWorkspace</string>
            <key>NSPortName</key>
            <string>${APP_NAME}</string>
            <key>NSRequiredContext</key>
            <dict/>
            <key>NSSendFileTypes</key>
            <array>
                <string>public.folder</string>
            </array>
        </dict>
    </array>"
render_info_plist "${CONTENTS}/Info.plist" "${APP_NAME}" "${BUNDLE_ID}" "${VERSION}" "${SERVICES_KEYS}"

# The app icon, and the document icon for its file types.
make_icns assets/impulse-logo.svg "${RESOURCES}/AppIcon.icns" || true
make_icns assets/impulse-doc-icon.svg "${RESOURCES}/DocumentIcon.icns" || true

echo "    OK: ${APP_DIR}"

# ── Step 4b: Code Signing (optional) ─────────────────────────────────

if [[ "${SIGN}" == true ]]; then
    echo "==> Signing ${APP_NAME}.app with Developer ID..."

    ENTITLEMENTS="impulse-macos/Impulse.entitlements"

    # Resource bundles first (their Info.plist and layout come from the
    # copy step above).
    for bundle in "${RESOURCE_BUNDLES[@]}"; do
        BUNDLE_DIR="${RESOURCES}/${bundle}.bundle"
        [[ -d "${BUNDLE_DIR}" ]] || continue
        echo "    Signing ${bundle}.bundle..."
        codesign --force \
            --sign "${IMPULSE_SIGN_IDENTITY}" \
            --timestamp \
            "${BUNDLE_DIR}"
    done

    if [[ -f "${RESOURCES}/bin/impulse" ]]; then
        echo "    Signing command-line tool..."
        codesign --force --options runtime \
            --sign "${IMPULSE_SIGN_IDENTITY}" \
            --timestamp \
            "${RESOURCES}/bin/impulse"
    fi

    # Sign the app bundle (inside-out: resource bundles first, then the .app)
    echo "    Signing app bundle..."
    codesign --force --options runtime \
        --entitlements "${ENTITLEMENTS}" \
        --sign "${IMPULSE_SIGN_IDENTITY}" \
        --timestamp \
        "${APP_DIR}"

    # Verify signature (spctl requires notarization, so it runs after step 6)
    echo "    Verifying signature..."
    codesign --verify --deep --strict "${APP_DIR}"
    echo "    OK: Code signing verified"
fi

# ── Step 4c: Notarize the app (optional) ─────────────────────────────
#
# The app is notarized and stapled before it goes into the DMG, so the copy
# users drag to /Applications carries its ticket and opens offline too. The
# DMG is notarized and stapled separately after it's built.

notarize() {
    echo "    Submitting $(basename "$1") for notarization..."
    xcrun notarytool submit "$1" \
        --key "${IMPULSE_NOTARY_KEY}" \
        --key-id "${IMPULSE_NOTARY_KEY_ID}" \
        --issuer "${IMPULSE_NOTARY_ISSUER}" \
        --wait
}

if [[ "${NOTARIZE}" == true ]]; then
    echo "==> Notarizing ${APP_NAME}.app with Apple..."
    NOTARIZE_ZIP="dist/${APP_NAME// /-}-${VERSION}.zip"
    ditto -c -k --keepParent "${APP_DIR}" "${NOTARIZE_ZIP}"
    notarize "${NOTARIZE_ZIP}"
    rm -f "${NOTARIZE_ZIP}"
    echo "    Stapling the app..."
    xcrun stapler staple "${APP_DIR}"
    # Verify Gatekeeper acceptance (requires notarization on modern macOS)
    echo "    Verifying Gatekeeper acceptance..."
    spctl --assess --type exec "${APP_DIR}"
    echo "    OK: app notarized"
fi

# ── Step 5: Create .dmg (optional) ────────────────────────────────────

if [[ "${CREATE_DMG}" == true ]]; then
    DMG_BASE_NAME="${APP_NAME// /-}"
    echo "==> Creating ${DMG_BASE_NAME}-${VERSION}.dmg..."

    DMG_NAME="${DMG_BASE_NAME}-${VERSION}.dmg"
    DMG_PATH="dist/${DMG_NAME}"

    # Convert background SVG to PNG for the DMG window background
    DMG_BG_SVG="assets/dmg-background.svg"
    DMG_BG_PNG=""
    if [[ -f "${DMG_BG_SVG}" ]] && command -v rsvg-convert >/dev/null 2>&1; then
        DMG_BG_PNG=$(mktemp /tmp/dmg-background-XXXXXX.png)
        rsvg-convert -w 660 -h 400 "${DMG_BG_SVG}" -o "${DMG_BG_PNG}" 2>/dev/null || {
            echo "    Note: Could not convert DMG background SVG to PNG, continuing without background"
            DMG_BG_PNG=""
        }
    elif [[ -f "${DMG_BG_SVG}" ]]; then
        echo "    Note: rsvg-convert not found, DMG will have no custom background"
    fi

    # Remove any previous DMG at this path (create-dmg won't overwrite)
    rm -f "${DMG_PATH}"

    # Build create-dmg arguments
    DMG_ARGS=(
        --volname "Impulse"
        --window-size 660 400
        --window-pos 200 120
        --icon-size 100
        --icon "${APP_NAME}.app" 180 200
        --app-drop-link 480 200
        --hide-extension "${APP_NAME}.app"
        --format UDZO
        --no-internet-enable
    )

    # Add volume icon if AppIcon.icns was generated
    if [[ -f "${RESOURCES}/AppIcon.icns" ]]; then
        DMG_ARGS+=(--volicon "${RESOURCES}/AppIcon.icns")
    fi

    # Add background image if available
    if [[ -n "${DMG_BG_PNG}" && -f "${DMG_BG_PNG}" ]]; then
        DMG_ARGS+=(--background "${DMG_BG_PNG}")
    fi

    create-dmg "${DMG_ARGS[@]}" "${DMG_PATH}" "${APP_DIR}"

    # Clean up temp background PNG
    if [[ -n "${DMG_BG_PNG}" ]]; then
        rm -f "${DMG_BG_PNG}"
    fi

    # Sign the DMG if signing is enabled
    if [[ "${SIGN}" == true ]]; then
        echo "    Signing DMG..."
        codesign --force --sign "${IMPULSE_SIGN_IDENTITY}" --timestamp "${DMG_PATH}"
    fi

    if [[ "${NOTARIZE}" == true ]]; then
        echo "    Notarizing the DMG..."
        notarize "${DMG_PATH}"
        xcrun stapler staple "${DMG_PATH}"
    fi

    echo "    OK: ${DMG_PATH}"
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
    echo "    Disk image: ${DMG_PATH}"
fi
echo ""
echo "To run: open ${APP_DIR}"
