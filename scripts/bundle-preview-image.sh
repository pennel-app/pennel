#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "$(uname -s)" != Darwin ]; then
  echo "Preview disk images require macOS." >&2
  exit 1
fi
app="$root/apps/desktop/src-tauri/target/release/bundle/macos/Pennel Preview.app"
if [ ! -d "$app" ]; then
  echo "Build the shared preview app before creating its disk image." >&2
  exit 1
fi
identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")"
if [ "$identifier" != dev.pennel.Pennel.preview ]; then
  echo "Refusing to sign a non-preview application." >&2
  exit 1
fi
if [ "${CI:-}" = true ] && [ "${PENNEL_ALLOW_ADHOC_PREVIEW:-}" = 1 ]; then
  # This isolated CI bundle deliberately has no credential access entitlement.
  codesign --force --options runtime --timestamp=none --sign - \
    --entitlements "$root/apps/desktop/src-tauri/Pennel.entitlements" "$app"
else
  python3 - "$root" "$app" <<'PY'
import pathlib
import shutil
import sys
root, app = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
sys.path.insert(0, str(root / "scripts"))
import mac_signing
output = root / "build/mac-signing/preview"
plan = mac_signing.prepare("preview", output)
shutil.copyfile(output / "embedded.provisionprofile", app / "Contents/embedded.provisionprofile")
mac_signing.command(["codesign", "--force", "--options", "runtime", "--timestamp=none", "--sign", plan["identity"],
                     "--entitlements", output / "Pennel.entitlements", app])
mac_signing.inspect_signed_app(app, identifier=plan["identifier"], developer_id=plan["developer_id"])
PY
fi
codesign --verify --deep --strict "$app"
mkdir -p "$root/build"
stage="$(mktemp -d "$root/build/preview-image-XXXXXX")"
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/Pennel Preview.app"
ln -s /Applications "$stage/Applications"
# Finder layout automation requires an interactive desktop; the preview image does not.
image="$root/build/Pennel Preview.dmg"
hdiutil create -quiet -volname "Pennel Preview" -srcfolder "$stage" -ov -format UDZO "$image"
hdiutil verify -quiet "$image"
echo "built $image"
