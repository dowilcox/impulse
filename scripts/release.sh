#!/usr/bin/env bash
# release.sh — Tag a release, build, and produce distribution packages.
#
# This script handles both Linux and macOS builds depending on which
# platform it's run on. For a full cross-platform release, run it on
# both platforms and then combine the artifacts.
#
# Usage:
#   ./scripts/release.sh 0.3.0              # tag + build + package (current platform)
#   ./scripts/release.sh 0.3.0 --push       # also push tag and create GitHub release
#   ./scripts/release.sh 0.3.0 --macos-only # only build macOS artifacts (skip tagging)
#   ./scripts/release.sh 0.3.0 --linux-only # only build Linux artifacts (skip tagging)
#
# Cross-platform release workflow:
#   1. On Linux:  ./scripts/release.sh 0.3.0          # tags + builds Linux packages
#   2. On macOS:  ./scripts/release.sh 0.3.0 --macos-only  # builds macOS .app/.dmg
#   3. On either: ./scripts/release.sh 0.3.0 --push   # upload all dist/ artifacts
set -euo pipefail

VERSION="${1:-}"
PUSH=false
MACOS_ONLY=false
LINUX_ONLY=false

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [--push] [--macos-only] [--linux-only]"
    echo ""
    echo "  Cross-platform release workflow:"
    echo "    1. On Linux:  $0 0.3.0              # tag + build Linux packages"
    echo "    2. On macOS:  $0 0.3.0 --macos-only # build macOS .app + .dmg"
    echo "    3. On either: $0 0.3.0 --push       # push tag + create GitHub release"
    exit 1
fi

shift
for arg in "$@"; do
    case "$arg" in
        --push) PUSH=true ;;
        --macos-only) MACOS_ONLY=true ;;
        --linux-only) LINUX_ONLY=true ;;
    esac
done

TAG="v${VERSION}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"
PLATFORM="$(uname)"

cd "$PROJECT_ROOT"

# ── Preflight checks ────────────────────────────────────────────────────

echo "Checking required tools..."

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo not found. Install Rust: https://rustup.rs" >&2
    exit 1
fi

if [[ "$PUSH" == true ]] && ! command -v gh >/dev/null 2>&1; then
    echo "Error: gh (GitHub CLI) not found. Install it or omit --push." >&2
    exit 1
fi

# ── Check working tree ──────────────────────────────────────────────────

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "Error: working tree has uncommitted changes. Commit or stash first." >&2
    exit 1
fi

# ── Version bump + tag (skip for --macos-only / --linux-only) ──────────

if [[ "$MACOS_ONLY" == false && "$LINUX_ONLY" == false ]]; then
    echo "Setting version to ${VERSION}..."
    sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-core/Cargo.toml
    sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-editor/Cargo.toml
    sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-linux/Cargo.toml
    sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-ffi/Cargo.toml

    # Update lockfile
    cargo check -p impulse-core --quiet 2>/dev/null || true

    # Commit version bump if anything changed
    if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
        git add -A
        git commit -m "Release ${TAG}"
    fi

    # Tag
    if git rev-parse "$TAG" >/dev/null 2>&1; then
        echo "Tag ${TAG} already exists. Skipping tag creation."
    else
        echo "Creating tag ${TAG}..."
        git tag -a "$TAG" -m "Release ${TAG}"
    fi
fi

mkdir -p "$DIST_DIR"
DIST_FILES=()

# ── Linux build ──────────────────────────────────────────────────────

if [[ "$MACOS_ONLY" == false && "$PLATFORM" == "Linux" ]]; then
    echo ""
    echo "=== Building Linux packages ==="
    echo ""

    # Check Linux-specific tools
    if ! cargo deb --version >/dev/null 2>&1; then
        echo "Installing cargo-deb..."
        cargo install cargo-deb
    fi

    if ! cargo generate-rpm --version >/dev/null 2>&1; then
        echo "Installing cargo-generate-rpm..."
        cargo install cargo-generate-rpm
    fi

    HAS_MAKEPKG=false
    if command -v makepkg >/dev/null 2>&1; then
        HAS_MAKEPKG=true
    fi

    echo "Building release binary..."
    cargo build --release -p impulse-linux

    echo "Building .deb package..."
    DEB_PATH=$(cargo deb -p impulse-linux --no-build --no-strip 2>&1 | tail -1)
    cp "$DEB_PATH" "$DIST_DIR/"
    DEB_NAME=$(basename "$DEB_PATH")
    DIST_FILES+=("$DEB_NAME")

    echo "Building .rpm package..."
    cargo generate-rpm -p impulse-linux
    RPM_PATH=$(find target/generate-rpm -name "*.rpm" -type f -printf '%T@ %p\n' | sort -rn | head -1 | cut -d' ' -f2)
    cp "$RPM_PATH" "$DIST_DIR/"
    RPM_NAME=$(basename "$RPM_PATH")
    DIST_FILES+=("$RPM_NAME")

    if [[ "$HAS_MAKEPKG" == true ]]; then
        echo "Building .pkg.tar.zst package..."
        ARCH_BUILD_DIR=$(mktemp -d)
        trap 'rm -rf "$ARCH_BUILD_DIR"' EXIT

        sed "s/__VERSION__/${VERSION}/" pkg/arch/PKGBUILD > "$ARCH_BUILD_DIR/PKGBUILD"
        cp target/release/impulse "$ARCH_BUILD_DIR/"
        cp assets/dev.impulse.Impulse.desktop "$ARCH_BUILD_DIR/"
        cp assets/impulse-logo.svg "$ARCH_BUILD_DIR/"

        (cd "$ARCH_BUILD_DIR" && makepkg -f --nodeps)

        PKG_PATH=$(find "$ARCH_BUILD_DIR" -name '*.pkg.tar.zst' -type f | head -1)
        cp "$PKG_PATH" "$DIST_DIR/"
        PKG_NAME=$(basename "$PKG_PATH")
        DIST_FILES+=("$PKG_NAME")
    else
        echo "Skipping .pkg.tar.zst (makepkg not found — only available on Arch-based systems)"
    fi

elif [[ "$MACOS_ONLY" == false && "$PLATFORM" == "Darwin" && "$LINUX_ONLY" == false ]]; then
    echo "Note: Skipping Linux packages (not on Linux). Use --linux-only on a Linux machine."
fi

# ── macOS build ──────────────────────────────────────────────────────

if [[ "$LINUX_ONLY" == false && "$PLATFORM" == "Darwin" ]]; then
    echo ""
    echo "=== Building macOS packages ==="
    echo ""

    if ! command -v swift >/dev/null 2>&1; then
        echo "Error: swift not found. Install Xcode command line tools: xcode-select --install" >&2
        exit 1
    fi

    # Use the macOS build script to create .app + .dmg
    bash impulse-macos/build.sh --dmg

    DMG_NAME="Impulse-${VERSION}.dmg"
    if [[ -f "$DIST_DIR/$DMG_NAME" ]]; then
        DIST_FILES+=("$DMG_NAME")
    fi

elif [[ "$LINUX_ONLY" == false && "$PLATFORM" == "Linux" && "$MACOS_ONLY" == false ]]; then
    echo ""
    echo "Note: Skipping macOS build (not on macOS). Use --macos-only on a Mac."
fi

# ── Checksums ───────────────────────────────────────────────────────

if [[ ${#DIST_FILES[@]} -gt 0 ]]; then
    echo ""
    echo "Generating checksums..."
    cd "$DIST_DIR"
    # Use shasum on macOS, sha256sum on Linux
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "${DIST_FILES[@]}" >> "SHA256SUMS"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "${DIST_FILES[@]}" >> "SHA256SUMS"
    fi
    # Deduplicate checksums file (in case of repeated runs)
    sort -u -o "SHA256SUMS" "SHA256SUMS"
    cd "$PROJECT_ROOT"
fi

# ── Summary ─────────────────────────────────────────────────────────

echo ""
echo "Release ${TAG} built successfully:"
for f in "${DIST_FILES[@]}"; do
    echo "  dist/${f}"
done
if [[ -f "$DIST_DIR/SHA256SUMS" ]]; then
    echo "  dist/SHA256SUMS"
fi

# ── Push + GitHub release ───────────────────────────────────────────

if [[ "$PUSH" == true ]]; then
    echo ""
    echo "Pushing tag ${TAG}..."
    git push origin main
    git push origin "$TAG"

    # Collect all artifacts from dist/ for upload.
    RELEASE_ASSETS=()
    for f in "$DIST_DIR"/*; do
        [[ -f "$f" ]] && RELEASE_ASSETS+=("$f")
    done

    if git rev-parse "$TAG" >/dev/null 2>&1 && gh release view "$TAG" >/dev/null 2>&1; then
        echo "GitHub release ${TAG} already exists. Uploading additional assets..."
        gh release upload "$TAG" "${RELEASE_ASSETS[@]}" --clobber
    else
        echo "Creating GitHub release..."
        gh release create "$TAG" \
            --title "Impulse ${TAG}" \
            --generate-notes \
            "${RELEASE_ASSETS[@]}"
    fi

    echo ""
    echo "GitHub release: $(gh release view "$TAG" --json url -q .url)"
else
    echo ""
    echo "To publish this release:"
    echo "  git push origin main && git push origin ${TAG}"
    echo "  gh release create ${TAG} --title \"Impulse ${TAG}\" --generate-notes dist/*"
fi
