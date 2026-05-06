#!/usr/bin/env bash
set -euo pipefail

out="${1:-docs/settings.schema.json}"
mkdir -p "$(dirname "$out")"
cargo run -q -p impulse-core --example settings_schema > "$out"
