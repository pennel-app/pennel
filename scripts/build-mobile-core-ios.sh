#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
core_features=(--no-default-features)
case "${PENNEL_PEER_TRANSPORT_QA-0}" in
  0) ;;
  1)
    if [ "${PENNEL_VARIANT-preview}" = production ] || [ "${PENNEL_TESTFLIGHT-0}" = 1 ]; then
      echo "Peer transport qualification is restricted to local Pennel Preview builds" >&2
      exit 2
    fi
    core_features+=(--features peer-transport-qa)
    ;;
  *) echo "PENNEL_PEER_TRANSPORT_QA must be 0 or 1" >&2; exit 2 ;;
esac
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
support_calibration="$root/assets/qualifications/meeting-support-cloud-v1.json"
qualification_snapshot="$(mktemp "${TMPDIR:-/tmp}/pennel-mobile-registry.XXXXXX")"
trap 'rm -f "$qualification_snapshot"' EXIT
python3 - "$qualification_registry" "$support_calibration" "$qualification_snapshot" <<'PY'
import base64,hashlib,json,pathlib,sys
registry=pathlib.Path(sys.argv[1]).read_bytes()
document=json.loads(registry)
if not isinstance(document,dict) or not isinstance(document.get('adopted_records'),list):
 raise ValueError('The qualification registry must declare its adopted records.')
support=pathlib.Path(sys.argv[2]).read_bytes()
policy=json.loads(support)
if (not isinstance(policy,dict) or policy.get('schema_version')!=1
    or not isinstance(policy.get('runtime_policy'),str) or not policy['runtime_policy']):
 raise ValueError('The support calibration asset must declare its supported schema and runtime policy.')
snapshot={'bytes_base64':base64.b64encode(registry).decode('ascii'),
 'qualification_registry_sha256':hashlib.sha256(registry).hexdigest(),
 'qualification_registry_adopted_records':len(document['adopted_records']),
 'support_calibration_bytes_base64':base64.b64encode(support).decode('ascii'),
 'support_calibration_sha256':hashlib.sha256(support).hexdigest()}
pathlib.Path(sys.argv[3]).write_text(json.dumps(snapshot))
PY
verify_qualification_inputs() {
  python3 - "$qualification_registry" "$support_calibration" "$qualification_snapshot" <<'PY'
import base64,json,pathlib,sys
pinned=json.loads(pathlib.Path(sys.argv[3]).read_text())
if pathlib.Path(sys.argv[1]).read_bytes()!=base64.b64decode(pinned['bytes_base64'],validate=True):
 raise SystemExit('The qualification registry changed during native preparation. Start native preparation again.')
if pathlib.Path(sys.argv[2]).read_bytes()!=base64.b64decode(pinned['support_calibration_bytes_base64'],validate=True):
 raise SystemExit('The support calibration asset changed during native preparation. Start native preparation again.')
PY
}
verify_embedded_qualification_inputs() {
  python3 - "$qualification_snapshot" "$root/core/target/$1/release/libpennel_core.a" <<'PY'
import base64,json,mmap,pathlib,sys
pinned=json.loads(pathlib.Path(sys.argv[1]).read_text())
library=pathlib.Path(sys.argv[2])
with library.open('rb') as stream:
 with mmap.mmap(stream.fileno(),0,access=mmap.ACCESS_READ) as archive:
  for label,key in [('qualification registry','bytes_base64'),('support calibration asset','support_calibration_bytes_base64')]:
   if archive.find(base64.b64decode(pinned[key],validate=True))<0:
    raise SystemExit('The '+label+' is missing from '+str(library)+'. Start native preparation again.')
PY
}
verify_qualification_inputs
mkdir -p "$artifacts"
# A failed switch must not leave new libraries labelled by an old ordinary/QA manifest.
python3 - "$artifacts/manifest.json" <<'PY'
import json,os,pathlib,sys,uuid
path=pathlib.Path(sys.argv[1]); temporary=path.with_name(path.name+'.pending-'+uuid.uuid4().hex)
temporary.write_text(json.dumps({'schema':1,'preparing':True})+'\n')
os.replace(temporary,path)
PY
bash "$root/scripts/build-keyboard-core-ios.sh"
verify_qualification_inputs
binding_dir="$root/core/target/$binding_target/release"
bindings="$root/build/bindings-ios"
framework="$artifacts/PennelCoreFFI.xcframework"

for target in "${targets[@]}"; do
  verify_qualification_inputs
  cargo build --manifest-path "$root/core/Cargo.toml" --release --target "$target" --lib "${core_features[@]}"
  verify_qualification_inputs
  verify_embedded_qualification_inputs "$target"
done

rm -rf "$bindings"
# Bindgen reads crate metadata from its working directory; use the matching lean library.
verify_qualification_inputs
(cd "$root/core" && cargo run --quiet "${core_features[@]}" --bin uniffi-bindgen -- \
  generate --library "$binding_dir/libpennel_core.dylib" --language swift --out-dir "$bindings")
verify_qualification_inputs

mkdir -p "$bindings/headers"
mv "$bindings/pennel_coreFFI.h" "$bindings/headers/"
mv "$bindings/pennel_coreFFI.modulemap" "$bindings/headers/module.modulemap"
rm -rf "$framework"
mkdir -p "$artifacts"
framework_args=(-create-xcframework)
for target in "${targets[@]}"; do
  framework_args+=(-library "$root/core/target/$target/release/libpennel_core.a" -headers "$bindings/headers")
done
verify_qualification_inputs
xcodebuild "${framework_args[@]}" -output "$framework" >/dev/null
verify_qualification_inputs

mkdir -p "$artifacts/PennelCore"
cp "$bindings/pennel_core.swift" "$artifacts/PennelCore/"
python3 - "$root" "$artifacts" "$qualification_snapshot" "${targets[@]}" <<'PY'
import base64,hashlib,json,os,pathlib,subprocess,sys,uuid
root,artifacts=map(pathlib.Path,sys.argv[1:3])
pinned=json.loads(pathlib.Path(sys.argv[3]).read_text())
registry=base64.b64decode(pinned['bytes_base64'],validate=True)
support=base64.b64decode(pinned['support_calibration_bytes_base64'],validate=True)
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
 'support_calibration_sha256':pinned['support_calibration_sha256'],
 'features':['peer-transport-qa'] if os.environ.get('PENNEL_PEER_TRANSPORT_QA')=='1' else [],'targets':sys.argv[4:]}
if (root/'assets/qualifications/meeting-assistance.json').read_bytes()!=registry:
 raise SystemExit('The qualification registry changed during native preparation. Start native preparation again.')
if (root/'assets/qualifications/meeting-support-cloud-v1.json').read_bytes()!=support:
 raise SystemExit('The support calibration asset changed during native preparation. Start native preparation again.')
destination=artifacts/'manifest.json'
temporary=destination.with_name(destination.name+'.complete-'+uuid.uuid4().hex)
temporary.write_text(json.dumps(manifest,indent=2)+'\n')
os.replace(temporary,destination)
PY
verify_qualification_inputs
echo "Prepared matching mobile Rust libraries and Swift bindings in $artifacts"
node "$root/apps/mobile/plugins/native-sources.cjs"
verify_qualification_inputs
