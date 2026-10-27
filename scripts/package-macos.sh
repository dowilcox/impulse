#!/usr/bin/env bash
# package-macos.sh: the Rust app (rust branch) as a macOS app.
#
# Builds impulse-app and the impulse CLI in release mode and bundles them
# into dist/Impulse.app: the app in Contents/MacOS, the CLI in
# Contents/Resources/bin (where `impulse` reads the app's version from and
# control_server::cli_path finds it), and the icons
# and fonts the app reads at run time in Contents/Resources
# (impulse-app/src/resources.rs finds them there). The Info.plist is
# assets/macos/Info.plist.in.
#
# Usage:
#   ./scripts/package-macos.sh                          # dist/Impulse.app, unsigned
#   ./scripts/package-macos.sh --sign --notarize --dmg  # what release.sh runs
#   ./scripts/package-macos.sh --no-build ...           # bundle the last release build
#
# --dmg writes dist/Impulse-<version>.dmg. Signing and notarizing take:
#   IMPULSE_SIGN_IDENTITY  — "Developer ID Application: Name (TEAM_ID)";
#                            auto-detected from the keychain if not set
#   IMPULSE_NOTARY_KEY     — path to the App Store Connect API key (.p8)
#   IMPULSE_NOTARY_KEY_ID  — its key ID
#   IMPULSE_NOTARY_ISSUER  — its issuer ID
#
# Setting up signing: download the Developer ID Application certificate from
# developer.apple.com and double-click it to install it in the keychain (or
# `security import DeveloperIDApplication.p12 -k login.keychain`).
# Notarizing: in App Store Connect, Users and Access > Integrations > App
# Store Connect API, generate a key with Developer access, download its .p8
# (offered once) and note its key ID and issuer ID.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# render_info_plist <out> <app name> <bundle id> <version>
# Writes assets/macos/Info.plist.in with its @TOKENS@ filled in.
render_info_plist() {
    local out="$1" name="$2" id="$3" version="$4"
    local plist
    plist="$(<"${ROOT}/assets/macos/Info.plist.in")"
    # bash 5.2 reads & in a replacement as the match; these are literal.
    shopt -u patsub_replacement 2>/dev/null || true
    plist="${plist//@APP_NAME@/${name}}"
    plist="${plist//@BUNDLE_ID@/${id}}"
    plist="${plist//@VERSION@/${version}}"
    printf '%s\n' "${plist}" > "${out}"
}

# make_icns <svg> <out.icns>
# An .icns from an SVG, through rsvg-convert and iconutil. Says so and
# returns 1 when it can't (the bundle then has no icon).
make_icns() {
    local svg="$1" out="$2"
    local tmp
    tmp="$(mktemp -d)"
    local iconset="${tmp}/$(basename "${out}" .icns).iconset"
    mkdir -p "${iconset}"
    if command -v rsvg-convert >/dev/null 2>&1; then
        local size
        for size in 16 32 64 128 256 512 1024; do
            rsvg-convert -w ${size} -h ${size} "${svg}" \
                -o "${iconset}/icon_${size}x${size}.png" 2>/dev/null || true
        done
        # @2x variants
        for size in 16 32 128 256 512; do
            local double=$((size * 2))
            if [[ -f "${iconset}/icon_${double}x${double}.png" ]]; then
                cp "${iconset}/icon_${double}x${double}.png" \
                   "${iconset}/icon_${size}x${size}@2x.png"
            fi
        done
        if command -v iconutil >/dev/null 2>&1; then
            iconutil -c icns "${iconset}" -o "${out}" 2>/dev/null || true
        fi
    fi
    rm -rf "${tmp}"
    if [[ ! -f "${out}" ]]; then
        echo "    Note: Could not generate $(basename "${out}") (install rsvg-convert for the icon)"
        return 1
    fi
    echo "    OK: $(basename "${out}")"
}


BUILD=true
SIGN=false
NOTARIZE=false
CREATE_DMG=false
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=false ;;
        --sign) SIGN=true ;;
        --notarize) NOTARIZE=true; SIGN=true ;;
        --dmg) CREATE_DMG=true ;;
        *) echo "Error: unknown flag '$arg'" >&2; exit 1 ;;
    esac
done

# ── Preflight ─────────────────────────────────────────────────────────

if [[ "$(uname)" != "Darwin" ]]; then
    echo "Error: macOS apps are built on macOS." >&2
    exit 1
fi
# A shell under Rosetta reports x86_64 and would build for Intel.
if [[ "$(uname -m)" != "arm64" ]]; then
    echo "Error: Impulse builds for Apple silicon only (this shell runs as $(uname -m))." >&2
    exit 1
fi
if [[ "$CREATE_DMG" == true ]] && ! command -v create-dmg >/dev/null 2>&1; then
    echo "Error: create-dmg not found. Install it with: brew install create-dmg" >&2
    exit 1
fi

if [[ "$SIGN" == true && -z "${IMPULSE_SIGN_IDENTITY:-}" ]]; then
    IMPULSE_SIGN_IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed 's/.*"\(.*\)"/\1/' || true)
    if [[ -z "$IMPULSE_SIGN_IDENTITY" ]]; then
        echo "Error: no Developer ID Application certificate in the keychain." >&2
        echo "Install one, or set IMPULSE_SIGN_IDENTITY (see this script's header)." >&2
        exit 1
    fi
    echo "Auto-detected signing identity: $IMPULSE_SIGN_IDENTITY"
fi
if [[ "$NOTARIZE" == true ]]; then
    if [[ -z "${IMPULSE_NOTARY_KEY:-}" || -z "${IMPULSE_NOTARY_KEY_ID:-}" || -z "${IMPULSE_NOTARY_ISSUER:-}" ]]; then
        echo "Error: notarizing needs IMPULSE_NOTARY_KEY, IMPULSE_NOTARY_KEY_ID and IMPULSE_NOTARY_ISSUER" >&2
        echo "(see this script's header)." >&2
        exit 1
    fi
    if [[ ! -f "$IMPULSE_NOTARY_KEY" ]]; then
        echo "Error: notary key file not found: $IMPULSE_NOTARY_KEY" >&2
        exit 1
    fi
fi

VERSION="$(tr -d '[:space:]' < VERSION)"
APP_NAME="Impulse"
BUNDLE_ID="dev.impulse.Impulse"

# ── Build ─────────────────────────────────────────────────────────────

if [[ "$BUILD" == true ]]; then
    echo "==> Building impulse-app and impulse ${VERSION} (release)..."
    # The Info.plist's LSMinimumSystemVersion.
    MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --release -p impulse-app -p impulse-cli
fi
for bin in impulse-app impulse; do
    if [[ ! -x "target/release/$bin" ]]; then
        echo "Error: target/release/$bin is missing (build without --no-build)." >&2
        exit 1
    fi
    if [[ "$(lipo -archs "target/release/$bin")" != "arm64" ]]; then
        echo "Error: target/release/$bin is $(lipo -archs "target/release/$bin"), expected arm64 only." >&2
        exit 1
    fi
done
# The binaries carry their crate's version (CARGO_PKG_VERSION: About, the
# update check); a release must not ship one that disagrees with VERSION.
CRATE_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' impulse-app/Cargo.toml | head -1)"
if [[ "$CRATE_VERSION" != "$VERSION" ]]; then
    echo "Error: impulse-app is $CRATE_VERSION but VERSION is $VERSION." >&2
    exit 1
fi

# ── Bundle ────────────────────────────────────────────────────────────

APP_DIR="dist/${APP_NAME}.app"
CONTENTS="$APP_DIR/Contents"
RESOURCES="$CONTENTS/Resources"
echo "==> Creating ${APP_DIR}..."
rm -rf "$APP_DIR"
mkdir -p "$CONTENTS/MacOS" "$RESOURCES/bin" "$RESOURCES/lucide" "$RESOURCES/material" "$RESOURCES/fonts"

cp target/release/impulse-app "$CONTENTS/MacOS/$APP_NAME"
cp target/release/impulse "$RESOURCES/bin/impulse"
# What scripts/package-linux.sh puts in share/impulse/.
cp vendor/lucide/icons/*.svg "$RESOURCES/lucide/"
cp vendor/lucide/LICENSE "$RESOURCES/lucide/LICENSE"
cp -R assets/icons/material/. "$RESOURCES/material/"
cp -R vendor/fonts/. "$RESOURCES/fonts/"

render_info_plist "$CONTENTS/Info.plist" "$APP_NAME" "$BUNDLE_ID" "$VERSION"
plutil -lint "$CONTENTS/Info.plist" >/dev/null
make_icns assets/impulse-logo.svg "$RESOURCES/AppIcon.icns" || true
make_icns assets/impulse-doc-icon.svg "$RESOURCES/DocumentIcon.icns" || true
echo "    OK: $APP_DIR"

# ── Sign ──────────────────────────────────────────────────────────────

if [[ "$SIGN" == true ]]; then
    echo "==> Signing with $IMPULSE_SIGN_IDENTITY..."
    # Inside out: the CLI, then the app (which seals Resources/).
    codesign --force --options runtime --timestamp \
        --sign "$IMPULSE_SIGN_IDENTITY" "$RESOURCES/bin/impulse"
    codesign --force --options runtime --timestamp \
        --entitlements assets/macos/Impulse.entitlements \
        --sign "$IMPULSE_SIGN_IDENTITY" "$APP_DIR"
    codesign --verify --deep --strict "$APP_DIR"
    echo "    OK: signature verified"
fi

# ── Notarize ──────────────────────────────────────────────────────────
#
# The app is stapled before it goes into the disk image, so the copy users
# drag to /Applications opens offline too; the image is notarized after.

notarize() {
    echo "    Submitting $(basename "$1") for notarization..."
    xcrun notarytool submit "$1" \
        --key "$IMPULSE_NOTARY_KEY" \
        --key-id "$IMPULSE_NOTARY_KEY_ID" \
        --issuer "$IMPULSE_NOTARY_ISSUER" \
        --wait
}

if [[ "$NOTARIZE" == true ]]; then
    echo "==> Notarizing ${APP_DIR}..."
    ZIP="dist/${APP_NAME}-${VERSION}.zip"
    ditto -c -k --keepParent "$APP_DIR" "$ZIP"
    notarize "$ZIP"
    rm -f "$ZIP"
    xcrun stapler staple "$APP_DIR"
    spctl --assess --type exec "$APP_DIR"
    echo "    OK: app notarized"
fi

# ── Disk image ────────────────────────────────────────────────────────

if [[ "$CREATE_DMG" == true ]]; then
    DMG_PATH="dist/${APP_NAME}-${VERSION}.dmg"
    echo "==> Creating ${DMG_PATH}..."
    rm -f "$DMG_PATH"
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
    [[ -f "$RESOURCES/AppIcon.icns" ]] && DMG_ARGS+=(--volicon "$RESOURCES/AppIcon.icns")
    DMG_BG=""
    if command -v rsvg-convert >/dev/null 2>&1; then
        DMG_BG="$(mktemp -d)/dmg-background.png"
        if rsvg-convert -w 660 -h 400 assets/dmg-background.svg -o "$DMG_BG" 2>/dev/null; then
            DMG_ARGS+=(--background "$DMG_BG")
        else
            echo "    Note: couldn't render the DMG background; continuing without it"
        fi
    fi
    create-dmg "${DMG_ARGS[@]}" "$DMG_PATH" "$APP_DIR"
    [[ -n "$DMG_BG" ]] && rm -rf "$(dirname "$DMG_BG")"
    if [[ "$SIGN" == true ]]; then
        codesign --force --sign "$IMPULSE_SIGN_IDENTITY" --timestamp "$DMG_PATH"
    fi
    if [[ "$NOTARIZE" == true ]]; then
        notarize "$DMG_PATH"
        xcrun stapler staple "$DMG_PATH"
    fi
    echo "    OK: $DMG_PATH"
fi

echo ""
echo "==> Done: $APP_DIR"
[[ "$CREATE_DMG" == true ]] && echo "    Disk image: $DMG_PATH"
echo "To run: open $APP_DIR"
