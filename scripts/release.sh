#!/usr/bin/env bash
# release.sh — Build, and from main publish, a release of the Rust app.
#
# Usage:
#   ./scripts/release.sh 0.37.0          # build this platform's release into dist/
#   ./scripts/release.sh 0.37.0 --push   # macOS, on main: also commit, tag, push
#                                        # and create the GitHub release
#
# Without --push it runs on any branch, on macOS or Linux, and builds
# exactly what a release would ship from this commit, at <version>:
#   macOS  dist/Impulse-<version>.dmg, signed and notarized
#          (scripts/package-macos.sh --sign --notarize --dmg)
#   Linux  dist/impulse-<version>-linux-<arch>.tar.gz and the .deb
#          (scripts/package-linux.sh)
# plus dist/SHA256SUMS. The version files are put back afterwards; nothing
# is committed or tagged.
#
# With --push (macOS, main only, in step with origin/main) the version
# bump is committed and tagged once the build succeeds, main and the tag
# are pushed, and the GitHub release is created with the disk image. Its
# publication starts .github/workflows/release-linux.yml, which builds the
# Linux packages on x86_64 and arm64, attaches them and adds their lines to
# SHA256SUMS.
#
# The top-level VERSION file is the single source of truth for the app
# version; every workspace crate carries the same version, since the
# binaries report theirs (CARGO_PKG_VERSION).
set -euo pipefail

VERSION="${1:-}"
PUSH=false

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [--push]"
    exit 1
fi
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: '$VERSION' isn't a version (x.y.z)." >&2
    exit 1
fi

shift
for arg in "$@"; do
    case "$arg" in
        --push) PUSH=true ;;
        *)
            echo "Error: unknown flag '$arg'" >&2
            exit 1 ;;
    esac
done

TAG="v${VERSION}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$PROJECT_ROOT/dist"
PLATFORM="$(uname)"

cd "$PROJECT_ROOT"

# ── Preflight checks ────────────────────────────────────────────────────

case "$PLATFORM" in
    Darwin)
        # rsvg-convert renders the app icon and DMG background; without it
        # the release would ship with a generic icon. create-dmg builds the
        # disk image.
        TOOLS=(cargo rsvg-convert create-dmg) ;;
    Linux)
        TOOLS=(cargo)
        # The .deb is skipped without it (the release workflow has it).
        if ! command -v dpkg-deb >/dev/null 2>&1; then
            echo "Note: dpkg-deb not found; this build makes the tarball only."
        fi ;;
    *)
        echo "Error: releases are built on macOS or Linux." >&2
        exit 1 ;;
esac
for tool in "${TOOLS[@]}"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Error: $tool not found." >&2
        exit 1
    fi
done

if [[ "$PUSH" == true ]]; then
    if [[ "$PLATFORM" != Darwin ]]; then
        echo "Error: --push runs on macOS; the Linux packages are built by" >&2
        echo ".github/workflows/release-linux.yml once the release is published." >&2
        exit 1
    fi
    BRANCH="$(git symbolic-ref --quiet --short HEAD || true)"
    if [[ "$BRANCH" != main ]]; then
        echo "Error: releases are published from main (this is ${BRANCH:-a detached HEAD})." >&2
        echo "Leave off --push to build one from here." >&2
        exit 1
    fi
    if ! command -v gh >/dev/null 2>&1; then
        echo "Error: gh (GitHub CLI) not found. Install it or omit --push." >&2
        exit 1
    fi
    git fetch --quiet origin main
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]] \
        && ! git merge-base --is-ancestor origin/main HEAD; then
        echo "Error: main is behind or apart from origin/main; pull first." >&2
        exit 1
    fi
fi

# VERSION, the lock file and every workspace member's Cargo.toml.
MEMBERS=()
while IFS= read -r member; do
    MEMBERS+=("$member")
done < <(sed -n '/^members = \[/,/^\]/p' Cargo.toml | sed -n 's/^ *"\([^"]*\)",*$/\1/p')
VERSION_FILES=(VERSION Cargo.lock)
for member in "${MEMBERS[@]}"; do
    VERSION_FILES+=("$member/Cargo.toml")
done

# ── Clean tree, before anything is written ─────────────────────────────

# Anything else — staged, unstaged or untracked — would end up built into
# (or committed with) the release. A build that isn't pushed puts the
# version files back afterwards, so they must start clean too.
if [[ "$PUSH" == true ]]; then
    DIRTY=$(git status --porcelain | awk '{print $NF}' | grep -v -x -F -f <(printf '%s\n' "${VERSION_FILES[@]}") || true)
else
    DIRTY=$(git status --porcelain | awk '{print $NF}')
fi
if [[ -n "$DIRTY" ]]; then
    echo "Error: working tree has changes:" >&2
    echo "$DIRTY" >&2
    echo "Commit or stash first." >&2
    exit 1
fi

# A re-run after the tag exists builds exactly that commit, or nothing.
if [[ "$PUSH" == true ]] && git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
    if [[ "$(git rev-parse "${TAG}^{commit}")" != "$(git rev-parse HEAD)" ]]; then
        echo "Error: ${TAG} exists but isn't HEAD; the build wouldn't match the tag." >&2
        exit 1
    fi
fi

# ── Version bump ───────────────────────────────────────────────────────

if [[ "$PUSH" != true ]]; then
    SAVED_VERSIONS="$(mktemp -d)"
    for f in "${VERSION_FILES[@]}"; do
        mkdir -p "$SAVED_VERSIONS/$(dirname "$f")"
        cp "$f" "$SAVED_VERSIONS/$f"
    done
    restore_versions() {
        for f in "${VERSION_FILES[@]}"; do
            cp "$SAVED_VERSIONS/$f" "$f"
        done
        rm -rf "$SAVED_VERSIONS"
    }
    trap restore_versions EXIT
fi

echo "Setting version to ${VERSION}..."
echo "$VERSION" > VERSION

# The first `version = ` line of each crate is its package's (BSD-sed-free).
for member in "${MEMBERS[@]}"; do
    toml="$member/Cargo.toml"
    awk -v ver="$VERSION" '!done && /^version = "/ { sub(/^version = ".*"/, "version = \"" ver "\""); done=1 } 1' "$toml" > "$toml.tmp" && mv "$toml.tmp" "$toml"
done
# The lock file's entries for the workspace's own crates, nothing else.
cargo update --workspace --offline --quiet

# ── Build ──────────────────────────────────────────────────────────────

# Clean dist/ so stale artifacts from previous releases are never uploaded.
# Emptied rather than removed: a Finder window showing it writes a new
# .DS_Store meanwhile, and removing the folder itself then fails.
mkdir -p "$DIST_DIR"
find "$DIST_DIR" -mindepth 1 -delete 2>/dev/null || true
if [[ -n "$(find "$DIST_DIR" -mindepth 1 ! -name .DS_Store -print -quit)" ]]; then
    echo "Error: couldn't empty $DIST_DIR." >&2
    exit 1
fi

echo ""
if [[ "$PLATFORM" == Darwin ]]; then
    echo "=== Building the macOS release (signed + notarized .app + .dmg) ==="
    echo ""
    bash scripts/package-macos.sh --sign --notarize --dmg
    DIST_FILES=("Impulse-${VERSION}.dmg")
else
    echo "=== Building the Linux release (tarball + .deb) ==="
    echo ""
    bash scripts/package-linux.sh
    DIST_FILES=()
    for f in "$DIST_DIR"/impulse-"${VERSION}"-linux-*.tar.gz "$DIST_DIR"/impulse_"${VERSION}"_*.deb; do
        [[ -f "$f" ]] && DIST_FILES+=("$(basename "$f")")
    done
fi
for f in "${DIST_FILES[@]}"; do
    if [[ ! -f "$DIST_DIR/$f" ]]; then
        echo "Error: the build didn't produce dist/$f." >&2
        exit 1
    fi
done

# ── Checksums ──────────────────────────────────────────────────────────

echo ""
echo "Generating checksums..."
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$DIST_DIR" && sha256sum "${DIST_FILES[@]}" > SHA256SUMS)
else
    (cd "$DIST_DIR" && shasum -a 256 "${DIST_FILES[@]}" > SHA256SUMS)
fi

echo ""
echo "Release ${TAG} built successfully:"
for f in "${DIST_FILES[@]}"; do
    echo "  dist/${f}"
done
echo "  dist/SHA256SUMS"

if [[ "$PUSH" != true ]]; then
    echo ""
    echo "Not published (no --push): the version files are back as they were."
    exit 0
fi

# ── Commit + tag (only after a successful signed, notarized build) ─────

if [[ -n "$(git status --porcelain -- "${VERSION_FILES[@]}")" ]]; then
    git add -- "${VERSION_FILES[@]}"
    git commit -m "Release ${TAG}" -- "${VERSION_FILES[@]}"
fi

if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
    echo "Tag ${TAG} already exists at HEAD."
else
    echo "Creating tag ${TAG}..."
    git tag -a "$TAG" -m "Release ${TAG}"
fi

# ── Push + GitHub release ──────────────────────────────────────────────

echo ""
echo "Pushing main and ${TAG}..."
git push origin main
git push origin "$TAG"

RELEASE_ASSETS=()
for f in "${DIST_FILES[@]}" SHA256SUMS; do
    RELEASE_ASSETS+=("$DIST_DIR/$f")
done

if gh release view "$TAG" >/dev/null 2>&1; then
    echo "GitHub release ${TAG} already exists. Uploading assets..."
    gh release upload "$TAG" "${RELEASE_ASSETS[@]}" --clobber
    # That SHA256SUMS has only the disk image; the workflow adds Linux's.
    echo "Re-run the Linux packages for it (they add their checksums again):"
    echo "  gh workflow run release-linux.yml -f tag=${TAG}"
else
    echo "Creating GitHub release..."
    gh release create "$TAG" \
        --title "Impulse ${TAG}" \
        --generate-notes \
        "${RELEASE_ASSETS[@]}"
fi

echo ""
echo "GitHub release: $(gh release view "$TAG" --json url -q .url)"
echo "The Linux packages follow from .github/workflows/release-linux.yml:"
echo "  gh run list --workflow release-linux.yml --limit 1"
