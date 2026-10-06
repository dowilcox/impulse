#!/usr/bin/env bash
# vendor-monaco.sh — Download and vendor Monaco Editor for offline bundling.
# Output: vendor/monaco/vs/
# Run once, or when upgrading Monaco version.
set -euo pipefail

MONACO_VERSION="0.57.0"
# SHA256 of the npm tarball (checked against the registry's sha512
# integrity). To update: download the new tarball, compare it with
# `npm view monaco-editor@<version> dist.integrity`, then `shasum -a 256` it.
MONACO_SHA256="3ea1712fbacd3290cf4751007e3a5b57cc279767607812d4c3e57925fb0b05c2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENDOR_DIR="$PROJECT_ROOT/vendor/monaco"

echo "Vendoring Monaco Editor v${MONACO_VERSION}..."

# Work in a temp directory
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

# Download the npm tarball
echo "Downloading monaco-editor@${MONACO_VERSION}..."
curl -fsSL "https://registry.npmjs.org/monaco-editor/-/monaco-editor-${MONACO_VERSION}.tgz" \
    -o "$WORK_DIR/monaco.tgz"

echo "Verifying download integrity..."
echo "${MONACO_SHA256}  ${WORK_DIR}/monaco.tgz" | shasum -a 256 -c - >/dev/null || {
    echo "ERROR: Monaco download checksum verification failed!" >&2
    echo "The downloaded file may be corrupted or tampered with." >&2
    exit 1
}

# Extract tarball
echo "Extracting..."
tar -xzf "$WORK_DIR/monaco.tgz" -C "$WORK_DIR"

# Clean existing vendor dir and recreate
rm -rf "$VENDOR_DIR"
mkdir -p "$VENDOR_DIR"

# Copy min/vs/ tree
echo "Copying min/vs/ tree..."
cp -r "$WORK_DIR/package/min/vs" "$VENDOR_DIR/vs"

# Remove heavy language workers — we use external LSP servers for
# language intelligence, so these bundled workers are dead weight.
echo "Removing unnecessary language workers..."
rm -rf "$VENDOR_DIR/vs/language"

# Summary
echo ""
echo "Vendored Monaco files:"
du -sh "$VENDOR_DIR"
echo "$(find "$VENDOR_DIR" -type f | wc -l) files"
echo ""
echo "Done! Vendored Monaco Editor v${MONACO_VERSION} to vendor/monaco/"
