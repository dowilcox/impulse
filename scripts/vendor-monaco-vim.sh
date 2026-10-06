#!/usr/bin/env bash
# vendor-monaco-vim.sh — Download and vendor monaco-vim (MIT) for the
# editor's optional Vim mode.
# Output: vendor/monaco-vim/ (the UMD build and its licenses)
# Run once, or when upgrading monaco-vim.
set -euo pipefail

MONACO_VIM_VERSION="0.4.4"
# SHA256 of the npm tarball.
MONACO_VIM_SHA256="4555ad079de3d63ba1dd9138dd7da581bfe4b29f1f3b4d9caeea06fc4008e3e8"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENDOR_DIR="$PROJECT_ROOT/vendor/monaco-vim"

echo "Vendoring monaco-vim v${MONACO_VIM_VERSION}..."

WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

curl -sfL "https://registry.npmjs.org/monaco-vim/-/monaco-vim-${MONACO_VIM_VERSION}.tgz" \
    -o "$WORK_DIR/monaco-vim.tgz"

echo "${MONACO_VIM_SHA256}  ${WORK_DIR}/monaco-vim.tgz" | shasum -a 256 -c - >/dev/null || {
    echo "ERROR: monaco-vim download checksum verification failed!" >&2
    exit 1
}

tar -xzf "$WORK_DIR/monaco-vim.tgz" -C "$WORK_DIR"

rm -rf "$VENDOR_DIR"
mkdir -p "$VENDOR_DIR"
cp "$WORK_DIR/package/dist/monaco-vim.umd.js" "$VENDOR_DIR/"
cp "$WORK_DIR/package/LICENSE" "$VENDOR_DIR/LICENSE"
cp "$WORK_DIR/package/LICENSE.codemirror.txt" "$VENDOR_DIR/LICENSE.codemirror.txt"

echo "Done: monaco-vim v${MONACO_VIM_VERSION} vendored to vendor/monaco-vim/"
