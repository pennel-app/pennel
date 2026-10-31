#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
artifacts="$root/apps/mobile/artifacts/ios"
device_target=aarch64-apple-ios
sim_target=aarch64-apple-ios-sim
case "${PENNEL_IOS_ARCHIVE_ONLY-0}" in
  0) targets=("$device_target" "$sim_target"); binding_target="$sim_target" ;;
  1) targets=("$device_target"); binding_target="$device_target" ;;
  *) echo "PENNEL_IOS_ARCHIVE_ONLY must be 0 or 1" >&2; exit 2 ;;
esac
export IPHONEOS_DEPLOYMENT_TARGET=26.0
bash "$root/scripts/build-keyboard-core-ios.sh"
binding_dir="$root/core/target/$binding_target/release"
bindings="$root/build/bindings-ios"
framework="$artifacts/PennelCoreFFI.xcframework"

for target in "${targets[@]}"; do
  cargo build --manifest-path "$root/core/Cargo.toml" --release --target "$target" --lib --no-default-features
done

rm -rf "$bindings"
# Bindgen reads crate metadata from its working directory; use the matching lean library.
(cd "$root/core" && cargo run --quiet --no-default-features --bin uniffi-bindgen -- \
  generate --library "$binding_dir/libpennel_core.dylib" --language swift --out-dir "$bindings")

mkdir -p "$bindings/headers"
mv "$bindings/pennel_coreFFI.h" "$bindings/headers/"
mv "$bindings/pennel_coreFFI.modulemap" "$bindings/headers/module.modulemap"
rm -rf "$framework"
mkdir -p "$artifacts"
framework_args=(-create-xcframework)
for target in "${targets[@]}"; do
  framework_args+=(-library "$root/core/target/$target/release/libpennel_core.a" -headers "$bindings/headers")
done
xcodebuild "${framework_args[@]}" -output "$framework" >/dev/null

mkdir -p "$artifacts/PennelCore"
cp "$bindings/pennel_core.swift" "$artifacts/PennelCore/"
python3 - "$root" "$artifacts" "${targets[@]}" <<'PY'
import hashlib,json,pathlib,subprocess,sys
root,artifacts=map(pathlib.Path,sys.argv[1:3])
digest=hashlib.sha256()
for p in sorted((root/'core').rglob('*')):
 if p.is_file() and 'target' not in p.parts and (p.suffix in {'.rs','.toml'} or p.name=='Cargo.lock'):
  digest.update(str(p.relative_to(root)).encode())
  digest.update(p.read_bytes())
manifest={'schema':1,'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'core_sha256':digest.hexdigest(),'bindings_sha256':hashlib.sha256((artifacts/'PennelCore/pennel_core.swift').read_bytes()).hexdigest(),'features':[],'targets':sys.argv[3:]}
(artifacts/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
PY
echo "Prepared matching mobile Rust libraries and Swift bindings in $artifacts"
node "$root/apps/mobile/plugins/native-sources.cjs"
