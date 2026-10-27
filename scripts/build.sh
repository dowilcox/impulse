#!/usr/bin/env bash
# build.sh: build Impulse (the app and the `impulse` CLI) for this platform.
#
# Usage:
#   ./scripts/build.sh --dev                     # debug build: target/debug/impulse-app
#   ./scripts/build.sh                           # release packages in dist/
#   ./scripts/build.sh --sign --notarize --dmg   # macOS: signed, notarized .app + .dmg
#
# --dev is the build scripts/docs/capture.py and scripts/smoke/run.py run
# (target/debug/impulse-app, with the dev data folder, impulse-dev); on the
# Mac it adds the snapshot feature, which their window images need. Without
# --dev it is scripts/package-linux.sh on Linux and scripts/package-macos.sh
# (with --sign, --notarize and --dmg passed on) on macOS. Releases go through
# scripts/release.sh, which runs the same packaging.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DEV_BUILD=false
PACKAGE_ARGS=()
for arg in "$@"; do
    case "$arg" in
        --dev) DEV_BUILD=true ;;
        --sign | --notarize | --dmg) PACKAGE_ARGS+=("$arg") ;;
        *) echo "Error: unknown flag '$arg'" >&2; exit 1 ;;
    esac
done

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo not found. Install Rust: https://rustup.rs" >&2
    exit 1
fi

case "$(uname)" in
    Linux)
        if [[ ${#PACKAGE_ARGS[@]} -gt 0 ]]; then
            echo "Error: --sign, --notarize and --dmg are macOS only." >&2
            exit 1
        fi
        if [[ "$DEV_BUILD" == true ]]; then
            cargo build -p impulse-app -p impulse-cli
        else
            exec scripts/package-linux.sh
        fi ;;
    Darwin)
        if [[ "$DEV_BUILD" == true ]]; then
            if [[ ${#PACKAGE_ARGS[@]} -gt 0 ]]; then
                echo "Error: --sign, --notarize and --dmg package a release build; leave off --dev." >&2
                exit 1
            fi
            cargo build -p impulse-app -p impulse-cli --features impulse-app/snapshot
        else
            exec scripts/package-macos.sh "${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"}"
        fi ;;
    *)
        echo "Error: Impulse builds on macOS and Linux." >&2
        exit 1 ;;
esac

echo ""
echo "Run it with:"
echo "  ${ROOT}/target/debug/impulse-app [--cwd <folder>] [--open <file>]"
