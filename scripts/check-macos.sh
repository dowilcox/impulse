#!/bin/sh
# Type-check the Rust app for macOS from Linux (`cargo check`, nothing is
# linked), so code behind `cfg(target_os = "macos")` is compiled before
# anyone builds on a Mac. C dependencies (SQLite, zlib, libgit2) can't be
# compiled without the macOS SDK, so a stand-in compiler only creates the
# files the build scripts expect; GPUI's shader step runs only on a Mac
# host, so an empty shader file stands in for its output.
#
# Usage: scripts/check-macos.sh [cargo check arguments…]
# Needs: rustup target add aarch64-apple-darwin
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
target=aarch64-apple-darwin
target_dir=${CARGO_TARGET_DIR:-$root/target}
stubs=$(mktemp -d)
trap 'rm -rf "$stubs"' EXIT

cat > "$stubs/cc" <<'STUB'
#!/bin/sh
out=""; prev=""
for a in "$@"; do
  [ "$prev" = "-o" ] && out="$a"
  prev="$a"
done
[ -n "$out" ] && : > "$out"
exit 0
STUB
cat > "$stubs/ar" <<'STUB'
#!/bin/sh
for a in "$@"; do case "$a" in *.a) : > "$a"; exit 0;; esac; done
exit 0
STUB
chmod +x "$stubs/cc" "$stubs/ar"

check() {
  CC_aarch64_apple_darwin="$stubs/cc" AR_aarch64_apple_darwin="$stubs/ar" \
    CARGO_TARGET_DIR="$target_dir" \
    cargo check --manifest-path "$root/Cargo.toml" --target "$target" -p impulse-app "$@"
}

# The first run creates GPUI's build folders; give each its shader file.
if ! check "$@" 2>/dev/null; then
  for out in "$target_dir/$target"/*/build/gpui-pre-apple-*/out; do
    [ -d "$out" ] && : > "$out/stitched_shaders.metal"
  done
fi
check "$@"
