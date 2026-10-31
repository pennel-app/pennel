#!/usr/bin/env bash
set -euo pipefail

root="${CI_PRIMARY_REPOSITORY_PATH:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$root"
export PENNEL_VARIANT=preview
export PENNEL_IOS_ARCHIVE_ONLY=1
export IPHONEOS_DEPLOYMENT_TARGET=26.0
helper="$root/scripts/mobile-native-cache.py"
python3 "$helper" validate-environment --root "$root"

# Xcode runs this hook separately; use the Node executable prepared after cloning.
if [ -f "$root/apps/mobile/ios/.xcode.env.local" ]; then
  source "$root/apps/mobile/ios/.xcode.env.local"
  export PATH="$(dirname "$NODE_BINARY"):$PATH"
fi
started="$SECONDS"
python3 "$helper" prepare-build --root "$root" --build-number "${CI_BUILD_NUMBER:?The Xcode Cloud build number is required}"
python3 "$helper" record --root "$root" --phase archive-identity --elapsed "$((SECONDS - started))"
python3 - "$root/build/testflight/archive-start.json" <<'PY'
import json,pathlib,sys,time
pathlib.Path(sys.argv[1]).write_text(json.dumps({"started": time.time()})+"\n")
PY
