#!/usr/bin/env bash
# release.sh — Tag a release, build, and produce .deb, .rpm, and .pkg.tar.zst packages.
#
# Usage:
#   ./scripts/release.sh 0.1.0           # tag + build + package
#   ./scripts/release.sh 0.1.0 --push    # also push tag and create GitHub release
set -euo pipefail

VERSION="${1:-}"
PUSH=false

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [--push]"
    echo "  e.g. $0 0.1.0"
    exit 1
fi

if [[ "${2:-}" == "--push" ]]; then
    PUSH=true
fi

TAG="v${VERSION}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"

cd "$PROJECT_ROOT"

# ── Preflight checks ────────────────────────────────────────────────────

echo "Checking required tools..."

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo not found. Install Rust: https://rustup.rs" >&2
    exit 1
fi

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

if [[ "$PUSH" == true ]] && ! command -v gh >/dev/null 2>&1; then
    echo "Error: gh (GitHub CLI) not found. Install it or omit --push." >&2
    exit 1
fi

# ── Check working tree ──────────────────────────────────────────────────

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "Error: working tree has uncommitted changes. Commit or stash first." >&2
    exit 1
fi

# ── Update version in Cargo.toml ────────────────────────────────────────

echo "Setting version to ${VERSION}..."
sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-core/Cargo.toml
sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-editor/Cargo.toml
sed -i "0,/^version = \".*\"/s//version = \"${VERSION}\"/" impulse-linux/Cargo.toml

# Update lockfile
cargo check -p impulse-linux --quiet 2>/dev/null || true

# Commit version bump if anything changed
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    git add -A
    git commit -m "Release ${TAG}"
fi

# ── Tag ─────────────────────────────────────────────────────────────────

if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Tag ${TAG} already exists. Skipping tag creation."
else
    echo "Creating tag ${TAG}..."
    git tag -a "$TAG" -m "Release ${TAG}"
fi

# ── Build ───────────────────────────────────────────────────────────────

echo "Building release binary..."
cargo build --release -p impulse-linux

# ── Package ─────────────────────────────────────────────────────────────

mkdir -p "$DIST_DIR"
DIST_FILES=()

echo "Building .deb package..."
DEB_PATH=$(cargo deb -p impulse-linux --no-build --no-strip 2>&1 | tail -1)
cp "$DEB_PATH" "$DIST_DIR/"
DEB_NAME=$(basename "$DEB_PATH")
DIST_FILES+=("$DEB_NAME")

echo "Building .rpm package..."
cargo generate-rpm -p impulse-linux
RPM_PATH=$(find target/generate-rpm -name '*.rpm' -type f | head -1)
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

# ── Checksums ───────────────────────────────────────────────────────────

echo "Generating checksums..."
cd "$DIST_DIR"
sha256sum "${DIST_FILES[@]}" > "SHA256SUMS"
cd "$PROJECT_ROOT"

# ── Summary ─────────────────────────────────────────────────────────────

echo ""
echo "Release ${TAG} built successfully:"
for f in "${DIST_FILES[@]}"; do
    echo "  dist/${f}"
done
echo "  dist/SHA256SUMS"

# ── Push + GitHub release ───────────────────────────────────────────────

if [[ "$PUSH" == true ]]; then
    echo ""
    echo "Pushing tag ${TAG}..."
    git push origin main
    git push origin "$TAG"

    RELEASE_ASSETS=()
    for f in "${DIST_FILES[@]}"; do
        RELEASE_ASSETS+=("$DIST_DIR/$f")
    done
    RELEASE_ASSETS+=("$DIST_DIR/SHA256SUMS")

    echo "Creating GitHub release..."
    gh release create "$TAG" \
        --title "Impulse ${TAG}" \
        --generate-notes \
        "${RELEASE_ASSETS[@]}"

    echo ""
    echo "GitHub release created: $(gh release view "$TAG" --json url -q .url)"
else
    echo ""
    echo "To publish this release:"
    echo "  git push origin main && git push origin ${TAG}"
    echo "  gh release create ${TAG} --title \"Impulse ${TAG}\" --generate-notes dist/*"
fi
