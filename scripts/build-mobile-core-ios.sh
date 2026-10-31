#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
artifacts="$root/apps/mobile/artifacts/ios"
export IPHONEOS_DEPLOYMENT_TARGET=26.0
bash "$root/scripts/build-keyboard-core-ios.sh"
device_target=aarch64-apple-ios
sim_target=aarch64-apple-ios-sim
device_dir="$root/core/target/$device_target/release"
sim_dir="$root/core/target/$sim_target/release"
bindings="$root/build/bindings-ios"
framework="$artifacts/PennelCoreFFI.xcframework"

for target in "$device_target" "$sim_target"; do
  cargo build --manifest-path "$root/core/Cargo.toml" --release --target "$target" --lib --no-default-features
done

rm -rf "$bindings"
# Bindgen reads crate metadata from its working directory; use the matching lean library.
(cd "$root/core" && cargo run --quiet --no-default-features --bin uniffi-bindgen -- \
  generate --library "$sim_dir/libpennel_core.dylib" --language swift --out-dir "$bindings")

mkdir -p "$bindings/headers"
mv "$bindings/pennel_coreFFI.h" "$bindings/headers/"
mv "$bindings/pennel_coreFFI.modulemap" "$bindings/headers/module.modulemap"
rm -rf "$framework"
mkdir -p "$artifacts"
xcodebuild -create-xcframework \
  -library "$device_dir/libpennel_core.a" -headers "$bindings/headers" \
  -library "$sim_dir/libpennel_core.a" -headers "$bindings/headers" \
  -output "$framework" >/dev/null

mkdir -p "$artifacts/PennelCore"
cp "$bindings/pennel_core.swift" "$artifacts/PennelCore/"
python3 - "$root" "$artifacts" <<'PY'
import hashlib,json,pathlib,subprocess,sys
root,artifacts=map(pathlib.Path,sys.argv[1:])
digest=hashlib.sha256()
for p in sorted((root/'core').rglob('*')):
 if p.is_file() and 'target' not in p.parts and (p.suffix in {'.rs','.toml'} or p.name=='Cargo.lock'):
  digest.update(str(p.relative_to(root)).encode())
  digest.update(p.read_bytes())
manifest={'schema':1,'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'core_sha256':digest.hexdigest(),'bindings_sha256':hashlib.sha256((artifacts/'PennelCore/pennel_core.swift').read_bytes()).hexdigest(),'features':[],'targets':['aarch64-apple-ios','aarch64-apple-ios-sim']}
(artifacts/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
PY
echo "Prepared matching mobile Rust libraries and Swift bindings in $artifacts"
node "$root/apps/mobile/plugins/native-sources.cjs"
