#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/apps/desktop"
npm ci
if [ "$(uname -s)" = Darwin ]; then
  "$root/scripts/with-toolchain.sh" python3 "$root/scripts/mac_signing.py" \
    --flavor preview --output "$root/build/mac-signing/preview" --build
  "$root/scripts/bundle-preview-image.sh"
else
  "$root/scripts/with-toolchain.sh" npx tauri build --config src-tauri/tauri.preview.conf.json
fi
