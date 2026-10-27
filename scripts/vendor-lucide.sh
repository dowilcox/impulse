#!/usr/bin/env bash
# Vendor the Lucide icon subset used by Impulse's chrome.
#
# Downloads each SVG from the pinned lucide-static release into vendor/lucide/,
# which the app reads at run time (impulse-app/src/resources.rs) and the
# packages copy. Lucide is ISC-licensed; the license is vendored alongside the
# icons.
#
# Usage: ./scripts/vendor-lucide.sh            (add names to ICONS below first)
set -euo pipefail

VERSION="1.52.0"
BASE="https://unpkg.com/lucide-static@${VERSION}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT}/vendor/lucide"

ICONS=(
  archive arrow-down arrow-up arrow-up-down bell bell-dot bookmark bot check
  chevron-down chevron-left chevron-right chevron-up chevrons-down-up chevrons-up-down
  circle circle-alert circle-check circle-dot circle-stop circle-x clipboard clock code
  columns-2 command copy corner-down-left ellipsis external-link eye eye-off file
  file-code file-diff file-plus files filter folder folder-git-2 folder-open folder-plus
  folder-tree git-branch git-branch-plus git-commit-horizontal git-compare git-fork
  git-graph git-merge git-pull-request globe history inbox info keyboard layers list
  list-filter list-tree loader-circle maximize-2 message-square minimize-2 minus
  package panel-bottom panel-left panel-right pencil pin pin-off play plug plus
  refresh-cw rotate-ccw rows-2 search settings sliders-horizontal square-terminal tag
  terminal trash-2 triangle-alert undo-2 upload user x zap
  space list-collapse wrap-text message-square-plus square-check square menu
  shield-alert shield-check flag palette repeat-2 split
)

mkdir -p "${OUT_DIR}/icons"
curl -fsSL "${BASE}/LICENSE" -o "${OUT_DIR}/LICENSE"
echo "${VERSION}" > "${OUT_DIR}/VERSION"

for name in "${ICONS[@]}"; do
  curl -fsSL "${BASE}/icons/${name}.svg" -o "${OUT_DIR}/icons/${name}.svg" \
    || { echo "missing icon: ${name}" >&2; exit 1; }
done

echo "Vendored ${#ICONS[@]} Lucide icons (v${VERSION}) -> ${OUT_DIR}"
