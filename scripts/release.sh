#!/usr/bin/env bash
# release.sh — Tag a release, build, and publish the macOS app.
#
# Usage:
#   ./scripts/release.sh 0.30.0          # bump version + tag + build .app/.dmg
#   ./scripts/release.sh 0.30.0 --push   # also push commit + tag and create the GitHub release
#
# The top-level VERSION file is the single source of truth for the app
# version; the Rust crate versions are kept in sync for tidiness.
set -euo pipefail

VERSION="${1:-}"
PUSH=false

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [--push]"
    exit 1
fi

shift
for arg in "$@"; do
    case "$arg" in
        --push) PUSH=true ;;
        --macos-only)
            echo "Note: --macos-only is obsolete (releases are macOS-only now); ignoring." ;;
        *)
            echo "Error: unknown flag '$arg'" >&2
            exit 1 ;;
    esac
done

TAG="v${VERSION}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"

cd "$PROJECT_ROOT"

# ── Preflight checks ────────────────────────────────────────────────────

if [[ "$(uname)" != "Darwin" ]]; then
    echo "Error: releases are built on macOS." >&2
    exit 1
fi

for tool in cargo swift; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Error: $tool not found." >&2
        exit 1
    fi
done

if [[ "$PUSH" == true ]] && ! command -v gh >/dev/null 2>&1; then
    echo "Error: gh (GitHub CLI) not found. Install it or omit --push." >&2
    exit 1
fi

# ── Version bump ───────────────────────────────────────────────────────

echo "Setting version to ${VERSION}..."
echo "$VERSION" > VERSION

# Keep Rust crate versions in sync (BSD-sed-free approach).
for toml in impulse-core/Cargo.toml impulse-ffi/Cargo.toml impulse-terminal/Cargo.toml; do
    [[ -f "$toml" ]] || continue
    awk -v ver="$VERSION" '!done && /^version = "/ { sub(/^version = ".*"/, "version = \"" ver "\""); done=1 } 1' "$toml" > "$toml.tmp" && mv "$toml.tmp" "$toml"
done
cargo check -p impulse-ffi --quiet 2>/dev/null || true

# ── Commit + tag ───────────────────────────────────────────────────────

PRE_BUMP_DIRTY=$(git diff --name-only -- ':!Cargo.lock' ':!VERSION' ':!impulse-core/Cargo.toml' ':!impulse-ffi/Cargo.toml' ':!impulse-terminal/Cargo.toml')
if [[ -n "$PRE_BUMP_DIRTY" ]]; then
    echo "Error: working tree has uncommitted changes beyond version files:" >&2
    echo "$PRE_BUMP_DIRTY" >&2
    echo "Commit or stash first." >&2
    exit 1
fi

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    git add VERSION Cargo.lock
    for toml in impulse-core/Cargo.toml impulse-ffi/Cargo.toml impulse-terminal/Cargo.toml; do
        [[ -f "$toml" ]] && git add "$toml"
    done
    git commit -m "Release ${TAG}"
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Tag ${TAG} already exists. Skipping tag creation."
else
    echo "Creating tag ${TAG}..."
    git tag -a "$TAG" -m "Release ${TAG}"
fi

# ── Build ──────────────────────────────────────────────────────────────

# Clean dist/ so stale artifacts from previous releases are never uploaded.
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
DIST_FILES=()

echo ""
echo "=== Building macOS release (signed + notarized .app + .dmg) ==="
echo ""
bash impulse-macos/build.sh --dmg --sign --notarize

DMG_NAME="Impulse-${VERSION}.dmg"
if [[ -f "$DIST_DIR/$DMG_NAME" ]]; then
    DIST_FILES+=("$DMG_NAME")
fi

# ── Checksums ──────────────────────────────────────────────────────────

if [[ ${#DIST_FILES[@]} -gt 0 ]]; then
    echo ""
    echo "Generating checksums..."
    (cd "$DIST_DIR" && shasum -a 256 "${DIST_FILES[@]}" > "SHA256SUMS")
fi

# ── Summary ────────────────────────────────────────────────────────────

echo ""
echo "Release ${TAG} built successfully:"
for f in "${DIST_FILES[@]}"; do
    echo "  dist/${f}"
done
[[ -f "$DIST_DIR/SHA256SUMS" ]] && echo "  dist/SHA256SUMS"

# ── Push + GitHub release ──────────────────────────────────────────────

if [[ "$PUSH" == true ]]; then
    echo ""
    echo "Pushing tag ${TAG}..."
    git push origin main
    git push origin "$TAG"

    RELEASE_ASSETS=()
    for f in "$DIST_DIR"/*; do
        [[ -f "$f" ]] && RELEASE_ASSETS+=("$f")
    done

    if gh release view "$TAG" >/dev/null 2>&1; then
        echo "GitHub release ${TAG} already exists. Uploading assets..."
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
