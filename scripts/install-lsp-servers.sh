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
  --no-fund \
  typescript@5.7.3 \
  typescript-language-server@4.3.3 \
  intelephense@1.12.6 \
  vscode-langservers-extracted@4.10.0 \
  @tailwindcss/language-server@0.14.14 \
  @vue/language-server@2.2.0 \
  svelte-language-server@0.17.7 \
  graphql-language-service-cli@3.4.1 \
  emmet-ls@0.7.1 \
  yaml-language-server@1.15.0 \
  dockerfile-language-server-nodejs@0.13.0 \
  bash-language-server@5.4.3

echo
echo "LSP servers installed."
echo "Managed binaries: ${LSP_ROOT}/node_modules/.bin"
echo "You can verify from the app with: impulse --check-lsp-servers"
