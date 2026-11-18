#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
: "${APPLE_SIGNING_IDENTITY:?Set the verified personal Developer ID identity before signing}"
: "${PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE:?Set the verified personal Preview provisioning profile}"
output="$root/build/peer-comparison"
if [[ ! -f "$output/bin/libpennel_core.dylib" ]]; then
  echo "Run scripts/build-peer-comparison.sh to prepare the probe and its matching core before signing." >&2
  exit 1
fi
stage="$(mktemp -d "$output/.host-sign.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/Pennel Peer Comparison.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks"
# Relative reads from the same working directory pin one published generation.
(cd "$output/bin" && cp peer-comparison-probe "$app/Contents/MacOS/" && cp libpennel_core.dylib "$app/Contents/Frameworks/")
plan="$stage/mac-signing"
python3 -B "$root/scripts/mac_signing.py" --flavor preview --output "$plan"
identity="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["identity"])' "$plan/signing-plan.json")"
cp "$plan/embedded.provisionprofile" "$app/Contents/embedded.provisionprofile"
python3 - "$app/Contents/Info.plist" <<'PY'
import pathlib,plistlib,sys
path=pathlib.Path(sys.argv[1])
path.write_bytes(plistlib.dumps({'CFBundleIdentifier':'dev.pennel.Pennel.preview',
 'CFBundleName':'Pennel Peer Comparison','CFBundleExecutable':'peer-comparison-probe',
 'NSLocalNetworkUsageDescription':'This isolated Pennel qualification exchanges synthetic fixtures with your iPhone.',
 'CFBundlePackageType':'APPL','CFBundleVersion':'1','CFBundleShortVersionString':'0.1.0'}))
PY
install_name_tool -rpath '@executable_path' '@executable_path/../Frameworks' "$app/Contents/MacOS/peer-comparison-probe"
codesign --force --options runtime --timestamp=none --sign "$identity" "$app/Contents/Frameworks/libpennel_core.dylib"
codesign --force --options runtime --timestamp=none --sign "$identity" \
  --entitlements "$plan/Pennel.entitlements" "$app"
codesign --verify --strict --deep "$app"
destination="$output/Pennel Peer Comparison.app"
python3 -B "$root/scripts/publish-peer-comparison-directory.py" "$app" "$destination"
echo "Signed isolated comparison helper at $destination"
