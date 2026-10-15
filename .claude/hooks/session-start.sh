#!/bin/bash
# Prepares Claude Code cloud sessions to build and test mccboard
# (Tauri v2 + Rust workspace + Svelte/TypeScript UI) on Linux.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}"

# Tauri v2 system dependencies for Linux.
TAURI_DEPS=(
  build-essential curl wget file pkg-config
  libwebkit2gtk-4.1-dev libxdo-dev libssl-dev
  libayatana-appindicator3-dev librsvg2-dev
)
missing=()
for pkg in "${TAURI_DEPS[@]}"; do
  dpkg -s "$pkg" >/dev/null 2>&1 || missing+=("$pkg")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "Installing Tauri system dependencies: ${missing[*]}" >&2
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends "${missing[@]}" >/dev/null
fi

# Toolchains.
command -v cargo >/dev/null || { echo "cargo not found" >&2; exit 1; }
command -v node >/dev/null || { echo "node not found" >&2; exit 1; }
if command -v rustup >/dev/null; then
  rustup component add clippy rustfmt >/dev/null 2>&1 || true
fi

# Project dependencies, once they exist.
if [ -f Cargo.toml ]; then
  cargo fetch --quiet
fi
for dir in . ui; do
  if [ -f "$dir/package.json" ]; then
    (cd "$dir" && npm install --no-audit --no-fund --loglevel=error)
  fi
done
