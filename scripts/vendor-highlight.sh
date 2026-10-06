#!/usr/bin/env bash
# vendor-highlight.sh — Download and vendor highlight.js (BSD-3-Clause), the
# common-languages browser build used for Markdown preview code blocks and
# review/history syntax colors.
# Output: vendor/highlight/ (highlight.min.js and its license)
# Run once, or when upgrading highlight.js.
set -euo pipefail

HIGHLIGHT_VERSION="11.12.0"
# SHA256 of the @highlightjs/cdn-assets npm tarball (checked against the
# registry's sha512 integrity). To update: download the new tarball, compare
# it with `npm view @highlightjs/cdn-assets@<version> dist.integrity`, then
# `shasum -a 256` it.
HIGHLIGHT_SHA256="b8a006d30f45afe783072569f3d69c5b60c0e7b9ca28cd474e12f2584e2a3bd9"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENDOR_DIR="$PROJECT_ROOT/vendor/highlight"

echo "Vendoring highlight.js v${HIGHLIGHT_VERSION}..."

WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

curl -fsSL "https://registry.npmjs.org/@highlightjs/cdn-assets/-/cdn-assets-${HIGHLIGHT_VERSION}.tgz" \
    -o "$WORK_DIR/highlight.tgz"

echo "${HIGHLIGHT_SHA256}  ${WORK_DIR}/highlight.tgz" | shasum -a 256 -c - >/dev/null || {
    echo "ERROR: highlight.js download checksum verification failed!" >&2
    exit 1
}

tar -xzf "$WORK_DIR/highlight.tgz" -C "$WORK_DIR"

rm -rf "$VENDOR_DIR"
mkdir -p "$VENDOR_DIR"
cp "$WORK_DIR/package/highlight.min.js" "$VENDOR_DIR/"
cp "$WORK_DIR/package/LICENSE" "$VENDOR_DIR/LICENSE"

echo "Done: highlight.js v${HIGHLIGHT_VERSION} vendored to vendor/highlight/"
