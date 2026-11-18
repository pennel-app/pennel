#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
if ! command -v cargo >/dev/null && [ -d "$root/build/toolchain/cargo" ]; then
  export CARGO_HOME="$(cd "$root/build/toolchain/cargo" && pwd -P)"
  export RUSTUP_HOME="$(cd "$root/build/toolchain/rustup" && pwd -P)"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
output="$root/build/peer-comparison"
cargo_target="$output/cargo-target"
mkdir -p "$output"
stage="$(mktemp -d "$output/.host-build.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
bindings="$stage/bindings-host"
bin="$stage/bin"
library="$bin/libpennel_core.dylib"
mkdir -p "$bin" "$bindings/headers"
cargo build --manifest-path "$root/core/Cargo.toml" --target-dir "$cargo_target" \
  --release --no-default-features --features peer-transport-qa --lib --example peer_compare_probe
cp "$cargo_target/release/libpennel_core.dylib" "$library"
(cd "$root/core" && cargo run --quiet --no-default-features --features peer-transport-qa --bin uniffi-bindgen -- \
  generate --library "$library" --language swift --out-dir "$bindings")
cp "$bindings/pennel_coreFFI.h" "$bindings/headers/"
cp "$bindings/pennel_coreFFI.modulemap" "$bindings/headers/module.modulemap"
install_name_tool -id '@rpath/libpennel_core.dylib' "$library"
swiftc -swift-version 6 -O -D PENNEL_PEER_TRANSPORT_QA -module-name PeerComparisonQA \
  -I "$bindings/headers" -L "$bin" -lpennel_core \
  -Xlinker -headerpad_max_install_names -Xlinker -rpath -Xlinker '@executable_path' \
  "$bindings/pennel_core.swift" \
  "$root/apps/mobile/native/ios/Services/PeerComparisonTransport.swift" \
  "$root/apps/mobile/native/ios/Services/PeerComparisonBridge.swift" \
  "$root/scripts/peer-comparison-native-cli.swift" -o "$bin/peer-comparison-probe"
python3 - "$bin/peer-comparison-probe" <<'PY'
import subprocess
import sys

lines = subprocess.check_output(["otool", "-L", sys.argv[1]], text=True).splitlines()[1:]
dependencies = [line.strip().split(" (", 1)[0] for line in lines]
core = [path for path in dependencies if path.rsplit("/", 1)[-1] == "libpennel_core.dylib"]
if core != ["@rpath/libpennel_core.dylib"]:
    raise SystemExit("The QA helper must link exactly one bundle-relative core library.")
PY
cargo build --manifest-path "$root/tools/peer-comparison-relay/Cargo.toml" --release
python3 -B "$root/scripts/publish-peer-comparison-directory.py" "$bindings" "$output/bindings-host"
python3 -B "$root/scripts/publish-peer-comparison-directory.py" "$bin" "$output/bin"
echo "Prepared QA probes and relay; signing and physical-device qualification remain separate."
