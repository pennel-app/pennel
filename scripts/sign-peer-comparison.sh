#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
: "${APPLE_SIGNING_IDENTITY:?Set the verified personal Developer ID identity before signing}"
: "${PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE:?Set the verified personal Preview provisioning profile}"
output="$root/build/peer-comparison"
python3 -B "$root/scripts/mac_signing.py" --flavor preview --output "$output/mac-signing"
app="$output/Pennel Peer Comparison.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks"
cp "$output/bin/peer-comparison-probe" "$app/Contents/MacOS/peer-comparison-probe"
cp "$root/core/target/release/libpennel_core.dylib" "$app/Contents/Frameworks/libpennel_core.dylib"
cp "$output/mac-signing/embedded.provisionprofile" "$app/Contents/embedded.provisionprofile"
python3 - "$app/Contents/Info.plist" <<'PY'
import pathlib,plistlib,sys
path=pathlib.Path(sys.argv[1])
path.write_bytes(plistlib.dumps({'CFBundleIdentifier':'dev.pennel.Pennel.preview',
 'CFBundleName':'Pennel Peer Comparison','CFBundleExecutable':'peer-comparison-probe',
 'CFBundlePackageType':'APPL','CFBundleVersion':'1','CFBundleShortVersionString':'0.1.0'}))
PY
install_name_tool -delete_rpath "$root/core/target/release" \
  -add_rpath '@executable_path/../Frameworks' "$app/Contents/MacOS/peer-comparison-probe"
python3 - "$app/Contents/MacOS/peer-comparison-probe" "$app/Contents/Frameworks/libpennel_core.dylib" <<'PY'
import pathlib,subprocess,sys
binary,library=sys.argv[1:]
dependencies=subprocess.check_output(['otool','-L',binary],text=True).splitlines()[1:]
matches=[line.strip().split(' (',1)[0] for line in dependencies if 'libpennel_core.dylib (' in line]
if len(matches)!=1:
 raise SystemExit('The QA helper must link exactly one owned core library.')
subprocess.run(['install_name_tool','-change',matches[0],'@rpath/libpennel_core.dylib',binary],check=True)
subprocess.run(['install_name_tool','-id','@rpath/libpennel_core.dylib',library],check=True)
PY
codesign --force --options runtime --timestamp=none --sign "$APPLE_SIGNING_IDENTITY" "$app/Contents/Frameworks/libpennel_core.dylib"
codesign --force --options runtime --timestamp=none --sign "$APPLE_SIGNING_IDENTITY" \
  --entitlements "$output/mac-signing/Pennel.entitlements" "$app"
codesign --verify --strict --deep "$app"
echo "Signed isolated comparison helper at $app"
