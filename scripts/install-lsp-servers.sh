#!/usr/bin/env bash
set -euo pipefail

if ! command -v npm >/dev/null 2>&1; then
  echo "npm was not found in PATH. Install Node.js + npm first." >&2
  exit 1
fi

if [[ -n "${XDG_DATA_HOME:-}" ]]; then
  IMPULSE_DATA_DIR="${XDG_DATA_HOME}/impulse"
else
  IMPULSE_DATA_DIR="${HOME}/.local/share/impulse"
fi

LSP_ROOT="${IMPULSE_DATA_DIR}/lsp"
mkdir -p "${LSP_ROOT}"

if [[ ! -f "${LSP_ROOT}/package.json" ]]; then
  cat >"${LSP_ROOT}/package.json" <<'JSON'
{
  "name": "impulse-lsp-servers",
  "private": true,
  "description": "Managed web LSP dependencies for Impulse",
  "license": "UNLICENSED"
}
JSON
fi

echo "Installing Impulse managed web LSP servers into ${LSP_ROOT} ..."

npm install \
  --prefix "${LSP_ROOT}" \
  --no-audit \
  --no-fund \
  typescript \
  typescript-language-server \
  intelephense \
  vscode-langservers-extracted \
  @tailwindcss/language-server \
  @vue/language-server \
  svelte-language-server \
  graphql-language-service-cli \
  emmet-ls \
  yaml-language-server \
  dockerfile-language-server-nodejs \
  bash-language-server

echo
echo "LSP servers installed."
echo "Managed binaries: ${LSP_ROOT}/node_modules/.bin"
echo "You can verify from the app with: impulse --check-lsp-servers"
