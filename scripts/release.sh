#!/usr/bin/env bash
# release.sh — Tag a release, build, and produce .deb + .rpm packages.
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

echo "Building .deb package..."
DEB_PATH=$(cargo deb -p impulse-linux --no-build --no-strip 2>&1 | tail -1)
cp "$DEB_PATH" "$DIST_DIR/"
DEB_NAME=$(basename "$DEB_PATH")

echo "Building .rpm package..."
cargo generate-rpm -p impulse-linux
RPM_PATH=$(find target/generate-rpm -name '*.rpm' -type f | head -1)
cp "$RPM_PATH" "$DIST_DIR/"
RPM_NAME=$(basename "$RPM_PATH")

# ── Checksums ───────────────────────────────────────────────────────────

echo "Generating checksums..."
cd "$DIST_DIR"
sha256sum "$DEB_NAME" "$RPM_NAME" > "SHA256SUMS"
cd "$PROJECT_ROOT"

# ── Summary ─────────────────────────────────────────────────────────────

echo ""
echo "Release ${TAG} built successfully:"
echo "  dist/${DEB_NAME}"
echo "  dist/${RPM_NAME}"
echo "  dist/SHA256SUMS"

# ── Push + GitHub release ───────────────────────────────────────────────

if [[ "$PUSH" == true ]]; then
    echo ""
    echo "Pushing tag ${TAG}..."
    git push origin main
    git push origin "$TAG"

    echo "Creating GitHub release..."
    gh release create "$TAG" \
        --title "Impulse ${TAG}" \
        --generate-notes \
        "$DIST_DIR/$DEB_NAME" \
        "$DIST_DIR/$RPM_NAME" \
        "$DIST_DIR/SHA256SUMS"

    echo ""
    echo "GitHub release created: $(gh release view "$TAG" --json url -q .url)"
else
    echo ""
    echo "To publish this release:"
    echo "  git push origin main && git push origin ${TAG}"
    echo "  gh release create ${TAG} --title \"Impulse ${TAG}\" --generate-notes dist/*"
fi
