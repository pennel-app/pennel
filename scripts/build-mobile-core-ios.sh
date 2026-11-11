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
qualification_registry="$root/assets/qualifications/meeting-assistance.json"
qualification_snapshot="$(mktemp "${TMPDIR:-/tmp}/pennel-mobile-registry.XXXXXX")"
trap 'rm -f "$qualification_snapshot"' EXIT
python3 - "$qualification_registry" "$qualification_snapshot" <<'PY'
import base64,hashlib,json,pathlib,sys
registry=pathlib.Path(sys.argv[1]).read_bytes()
document=json.loads(registry)
if not isinstance(document,dict) or not isinstance(document.get('adopted_records'),list):
 raise ValueError('The qualification registry must declare its adopted records.')
snapshot={'bytes_base64':base64.b64encode(registry).decode('ascii'),
 'qualification_registry_sha256':hashlib.sha256(registry).hexdigest(),
 'qualification_registry_adopted_records':len(document['adopted_records'])}
pathlib.Path(sys.argv[2]).write_text(json.dumps(snapshot))
PY
verify_qualification_registry() {
  python3 - "$qualification_registry" "$qualification_snapshot" <<'PY'
import base64,json,pathlib,sys
pinned=json.loads(pathlib.Path(sys.argv[2]).read_text())
if pathlib.Path(sys.argv[1]).read_bytes()!=base64.b64decode(pinned['bytes_base64'],validate=True):
 raise SystemExit('The qualification registry changed during native preparation. Start native preparation again.')
PY
}
verify_qualification_registry
bash "$root/scripts/build-keyboard-core-ios.sh"
verify_qualification_registry
binding_dir="$root/core/target/$binding_target/release"
bindings="$root/build/bindings-ios"
framework="$artifacts/PennelCoreFFI.xcframework"

for target in "${targets[@]}"; do
  verify_qualification_registry
  cargo build --manifest-path "$root/core/Cargo.toml" --release --target "$target" --lib --no-default-features
  verify_qualification_registry
done

rm -rf "$bindings"
# Bindgen reads crate metadata from its working directory; use the matching lean library.
verify_qualification_registry
(cd "$root/core" && cargo run --quiet --no-default-features --bin uniffi-bindgen -- \
  generate --library "$binding_dir/libpennel_core.dylib" --language swift --out-dir "$bindings")
verify_qualification_registry

mkdir -p "$bindings/headers"
mv "$bindings/pennel_coreFFI.h" "$bindings/headers/"
mv "$bindings/pennel_coreFFI.modulemap" "$bindings/headers/module.modulemap"
rm -rf "$framework"
mkdir -p "$artifacts"
framework_args=(-create-xcframework)
for target in "${targets[@]}"; do
  framework_args+=(-library "$root/core/target/$target/release/libpennel_core.a" -headers "$bindings/headers")
done
verify_qualification_registry
xcodebuild "${framework_args[@]}" -output "$framework" >/dev/null
verify_qualification_registry

mkdir -p "$artifacts/PennelCore"
cp "$bindings/pennel_core.swift" "$artifacts/PennelCore/"
python3 - "$root" "$artifacts" "$qualification_snapshot" "${targets[@]}" <<'PY'
import base64,hashlib,json,pathlib,subprocess,sys
root,artifacts=map(pathlib.Path,sys.argv[1:3])
pinned=json.loads(pathlib.Path(sys.argv[3]).read_text())
registry=base64.b64decode(pinned['bytes_base64'],validate=True)
digest=hashlib.sha256()
for p in sorted((root/'core').rglob('*')):
 if p.is_file() and 'target' not in p.parts and (p.suffix in {'.rs','.toml'} or p.name=='Cargo.lock'):
  digest.update(str(p.relative_to(root)).encode())
  digest.update(p.read_bytes())
manifest={'schema':1,'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),
 'core_sha256':digest.hexdigest(),
 'bindings_sha256':hashlib.sha256((artifacts/'PennelCore/pennel_core.swift').read_bytes()).hexdigest(),
 'qualification_registry_sha256':pinned['qualification_registry_sha256'],
 'qualification_registry_adopted_records':pinned['qualification_registry_adopted_records'],
 'features':[],'targets':sys.argv[4:]}
if (root/'assets/qualifications/meeting-assistance.json').read_bytes()!=registry:
 raise SystemExit('The qualification registry changed during native preparation. Start native preparation again.')
(artifacts/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
PY
verify_qualification_registry
echo "Prepared matching mobile Rust libraries and Swift bindings in $artifacts"
node "$root/apps/mobile/plugins/native-sources.cjs"
verify_qualification_registry
