#!/usr/bin/env bash
# Build a vendored static libgit2 for the macOS app (ImpulseGit).
#
# Network features are disabled (USE_HTTPS=OFF, USE_SSH=OFF) — Impulse only
# performs local git operations — so no OpenSSL/libssh2 is needed. The static
# library and headers land in impulse-macos/.libgit2/<version>/, which
# Package.swift references via the Clibgit2 system-library target.
#
# Idempotent: exits immediately if the pinned version is already built.
# Requires: cmake (brew install cmake), curl, Xcode command line tools.
set -euo pipefail

LIBGIT2_VERSION="1.9.7"
LIBGIT2_SHA256="1a4fbe7589e814777ae76b64734ad80f4ecad22cd33a22682a2aaea4ae5375e7"
DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-26.0}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/impulse-macos/.libgit2/${LIBGIT2_VERSION}"
WORK_DIR="${ROOT_DIR}/impulse-macos/.libgit2/build-${LIBGIT2_VERSION}"
MARKER="${OUT_DIR}/.complete"

if [[ -f "${MARKER}" && "$(cat "${MARKER}")" == "${LIBGIT2_VERSION}-${DEPLOYMENT_TARGET}" ]]; then
    echo "libgit2 ${LIBGIT2_VERSION} already built at ${OUT_DIR}"
    exit 0
fi

command -v cmake > /dev/null || {
    echo "ERROR: cmake not found. Install it with: brew install cmake" >&2
    exit 1
}

mkdir -p "${WORK_DIR}"
TARBALL="${WORK_DIR}/libgit2-${LIBGIT2_VERSION}.tar.gz"

if [[ ! -f "${TARBALL}" ]]; then
    echo "==> Downloading libgit2 ${LIBGIT2_VERSION}..."
    curl -fsSL -o "${TARBALL}" \
        "https://github.com/libgit2/libgit2/archive/refs/tags/v${LIBGIT2_VERSION}.tar.gz"
fi

echo "${LIBGIT2_SHA256}  ${TARBALL}" | shasum -a 256 -c - > /dev/null || {
    echo "ERROR: libgit2 tarball checksum mismatch" >&2
    rm -f "${TARBALL}"
    exit 1
}

echo "==> Extracting..."
rm -rf "${WORK_DIR}/src"
mkdir -p "${WORK_DIR}/src"
tar xzf "${TARBALL}" -C "${WORK_DIR}/src" --strip-components 1

echo "==> Configuring (static, no HTTPS/SSH — local git operations only)..."
cmake -S "${WORK_DIR}/src" -B "${WORK_DIR}/out" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="${DEPLOYMENT_TARGET}" \
    -DCMAKE_INSTALL_PREFIX="${OUT_DIR}" \
    -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TESTS=OFF \
    -DBUILD_CLI=OFF \
    -DBUILD_EXAMPLES=OFF \
    -DBUILD_FUZZERS=OFF \
    -DUSE_HTTPS=OFF \
    -DUSE_SSH=OFF \
    -DUSE_NTLMCLIENT=OFF \
    -DUSE_GSSAPI=OFF \
    -DUSE_BUNDLED_ZLIB=OFF \
    -DUSE_ICONV=ON \
    > "${WORK_DIR}/cmake-configure.log" 2>&1 || {
    echo "ERROR: cmake configure failed — see ${WORK_DIR}/cmake-configure.log" >&2
    exit 1
}

echo "==> Building..."
cmake --build "${WORK_DIR}/out" --parallel > "${WORK_DIR}/cmake-build.log" 2>&1 || {
    echo "ERROR: libgit2 build failed — see ${WORK_DIR}/cmake-build.log" >&2
    exit 1
}

echo "==> Installing to ${OUT_DIR}..."
rm -rf "${OUT_DIR}"
cmake --install "${WORK_DIR}/out" > /dev/null

[[ -f "${OUT_DIR}/lib/libgit2.a" ]] || {
    echo "ERROR: expected static library at ${OUT_DIR}/lib/libgit2.a" >&2
    exit 1
}

echo "${LIBGIT2_VERSION}-${DEPLOYMENT_TARGET}" > "${MARKER}"
echo "    OK: libgit2 ${LIBGIT2_VERSION} static library ready"
