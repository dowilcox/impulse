# macos-bundle.sh: the parts of a macOS app bundle that impulse-macos/build.sh
# (the Swift app) and scripts/package-macos.sh (the Rust app) share. Sourced,
# not run.

# render_info_plist <out> <app name> <bundle id> <version> [extra keys]
# Writes assets/macos/Info.plist.in with its @TOKENS@ filled in; `extra keys`
# is plist text put inside the top-level dict (the Swift app's NSServices).
render_info_plist() {
    local out="$1" name="$2" id="$3" version="$4" extra="${5:-}"
    local root
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    local plist
    plist="$(<"${root}/assets/macos/Info.plist.in")"
    # bash 5.2 reads & in a replacement as the match; these are literal.
    shopt -u patsub_replacement 2>/dev/null || true
    plist="${plist//@APP_NAME@/${name}}"
    plist="${plist//@BUNDLE_ID@/${id}}"
    plist="${plist//@VERSION@/${version}}"
    if [[ -n "${extra}" ]]; then
        plist="${plist//@EXTRA_KEYS@/${extra}}"
    else
        plist="${plist//@EXTRA_KEYS@$'\n'/}"
    fi
    printf '%s\n' "${plist}" > "${out}"
}

# make_icns <svg> <out.icns>
# An .icns from an SVG, through rsvg-convert and iconutil. Says so and
# returns 1 when it can't (the bundle then has no icon).
make_icns() {
    local svg="$1" out="$2"
    local tmp
    tmp="$(mktemp -d)"
    local iconset="${tmp}/$(basename "${out}" .icns).iconset"
    mkdir -p "${iconset}"
    if command -v rsvg-convert >/dev/null 2>&1; then
        local size
        for size in 16 32 64 128 256 512 1024; do
            rsvg-convert -w ${size} -h ${size} "${svg}" \
                -o "${iconset}/icon_${size}x${size}.png" 2>/dev/null || true
        done
        # @2x variants
        for size in 16 32 128 256 512; do
            local double=$((size * 2))
            if [[ -f "${iconset}/icon_${double}x${double}.png" ]]; then
                cp "${iconset}/icon_${double}x${double}.png" \
                   "${iconset}/icon_${size}x${size}@2x.png"
            fi
        done
        if command -v iconutil >/dev/null 2>&1; then
            iconutil -c icns "${iconset}" -o "${out}" 2>/dev/null || true
        fi
    fi
    rm -rf "${tmp}"
    if [[ ! -f "${out}" ]]; then
        echo "    Note: Could not generate $(basename "${out}") (install rsvg-convert for the icon)"
        return 1
    fi
    echo "    OK: $(basename "${out}")"
}
