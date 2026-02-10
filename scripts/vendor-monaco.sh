#!/usr/bin/env bash
# vendor-monaco.sh — Download and vendor Monaco Editor for offline bundling.
# Output: impulse-editor/vendor/monaco/vs/
# Run once, or when upgrading Monaco version.
set -euo pipefail

MONACO_VERSION="0.52.2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENDOR_DIR="$PROJECT_ROOT/impulse-editor/vendor/monaco"

echo "Vendoring Monaco Editor v${MONACO_VERSION}..."

# Work in a temp directory
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# Download the npm tarball
echo "Downloading monaco-editor@${MONACO_VERSION}..."
curl -sL "https://registry.npmjs.org/monaco-editor/-/monaco-editor-${MONACO_VERSION}.tgz" \
    -o "$TMPDIR/monaco.tgz"

# Extract tarball
echo "Extracting..."
tar -xzf "$TMPDIR/monaco.tgz" -C "$TMPDIR"

# Clean existing vendor dir and recreate
rm -rf "$VENDOR_DIR"
mkdir -p "$VENDOR_DIR"

# Copy min/vs/ tree
echo "Copying min/vs/ tree..."
cp -r "$TMPDIR/package/min/vs" "$VENDOR_DIR/vs"

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
echo "Done! Vendored Monaco Editor v${MONACO_VERSION} to impulse-editor/vendor/monaco/"
