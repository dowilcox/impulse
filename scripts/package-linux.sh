#!/usr/bin/env bash
# package-linux.sh: the Rust app (rust branch) as a Linux package.
#
# Builds impulse-app and the impulse CLI in release mode and lays them out
# as an installed app: the binaries in bin/, and the icons and fonts the app
# reads at run time in share/impulse/ (impulse-app/src/resources.rs finds
# them there, beside its executable), with a desktop entry and the icon.
# Writes into dist/:
#   impulse-<version>-linux-<arch>.tar.gz   that layout under a folder
#   impulse_<version>_<debarch>.deb         the same under /usr (dpkg-deb)
#
# Usage: ./scripts/package-linux.sh [--no-build]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUILD=true
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=false ;;
        *) echo "Error: unknown flag '$arg'" >&2; exit 1 ;;
    esac
done

if [[ "$(uname)" != "Linux" ]]; then
    echo "Error: Linux packages are built on Linux." >&2
    exit 1
fi

VERSION="$(tr -d '[:space:]' < VERSION)"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) DEB_ARCH=amd64 ;;
    aarch64) DEB_ARCH=arm64 ;;
    *) DEB_ARCH="$ARCH" ;;
esac

if [[ "$BUILD" == true ]]; then
    cargo build --release -p impulse-app -p impulse-cli
fi
for bin in impulse-app impulse; do
    if [[ ! -x "target/release/$bin" ]]; then
        echo "Error: target/release/$bin is missing (build without --no-build)." >&2
        exit 1
    fi
done

STAGE="$ROOT/target/package"
DIST="$ROOT/dist"
rm -rf "$STAGE"
mkdir -p "$DIST"

# One installed tree under $1 (a prefix such as usr/).
lay_out() {
    local prefix="$1"
    install -Dm755 target/release/impulse-app "$prefix/bin/impulse-app"
    install -Dm755 target/release/impulse "$prefix/bin/impulse"
    local share="$prefix/share/impulse"
    mkdir -p "$share/lucide" "$share/material" "$share/fonts"
    cp vendor/lucide/icons/*.svg "$share/lucide/"
    cp vendor/lucide/LICENSE "$share/lucide/LICENSE"
    cp -R assets/icons/material/. "$share/material/"
    cp -R vendor/fonts/. "$share/fonts/"
    install -Dm644 assets/impulse-logo.svg "$prefix/share/icons/hicolor/scalable/apps/impulse.svg"
    install -Dm644 /dev/stdin "$prefix/share/applications/impulse.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Impulse
GenericName=Terminal IDE
Comment=A terminal with command blocks and a code editor
Exec=impulse-app
Icon=impulse
Terminal=false
Categories=Development;IDE;TerminalEmulator;
StartupWMClass=impulse-app
DESKTOP
}

# The tarball: a folder holding bin/ and share/.
NAME="impulse-$VERSION-linux-$ARCH"
lay_out "$STAGE/tar/$NAME"
tar -C "$STAGE/tar" -czf "$DIST/$NAME.tar.gz" "$NAME"
echo "Wrote dist/$NAME.tar.gz"

# The .deb: the same tree under /usr.
if command -v dpkg-deb >/dev/null 2>&1; then
    DEB="$STAGE/deb"
    lay_out "$DEB/usr"
    SIZE="$(du -sk "$DEB/usr" | cut -f1)"
    mkdir -p "$DEB/DEBIAN"
    cat > "$DEB/DEBIAN/control" <<CONTROL
Package: impulse
Version: $VERSION
Section: devel
Priority: optional
Architecture: $DEB_ARCH
Installed-Size: $SIZE
Depends: libc6, libgcc-s1, zlib1g, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1, git
Recommends: libwayland-client0, mesa-vulkan-drivers
Maintainer: Impulse <noreply@impulse-terminal.app>
Homepage: https://impulse-terminal.app
Description: Terminal IDE: a terminal with command blocks and a code editor
 Impulse is a terminal with command blocks, a code editor with language
 servers, git review and history, and tasks for coding agents.
CONTROL
    dpkg-deb --root-owner-group --build "$DEB" "$DIST/impulse_${VERSION}_${DEB_ARCH}.deb" >/dev/null
    echo "Wrote dist/impulse_${VERSION}_${DEB_ARCH}.deb"
else
    echo "dpkg-deb not found: skipped the .deb"
fi
