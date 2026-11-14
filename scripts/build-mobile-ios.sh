#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
workspace="$root/apps/mobile"
if [ ! -d "$workspace/ios/Pennel.xcworkspace" ]; then
  echo "Prepare the native core, run Expo Prebuild, and install iOS pods before building." >&2
  exit 1
fi
node "$workspace/plugins/peer-transport-qa.cjs" "$workspace"
bash "$root/scripts/ci-xcodebuild.sh" build \
  -workspace "$workspace/ios/Pennel.xcworkspace" \
  -scheme Pennel \
  -configuration Release \
  -destination "${PENNEL_IOS_DESTINATION:-generic/platform=iOS Simulator}" \
  -derivedDataPath "$workspace/build/DerivedData" \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES \
  CODE_SIGNING_ALLOWED=NO
