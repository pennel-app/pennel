#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
if ! command -v cargo >/dev/null && [ -d "$root/build/toolchain/cargo" ]; then
  export CARGO_HOME="$(cd "$root/build/toolchain/cargo" && pwd -P)"
  export RUSTUP_HOME="$(cd "$root/build/toolchain/rustup" && pwd -P)"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
output="$root/build/peer-comparison"
bindings="$output/bindings-host"
mkdir -p "$output/bin" "$bindings/headers"
cargo build --manifest-path "$root/core/Cargo.toml" --release --no-default-features --features peer-transport-qa --lib --example peer_compare_probe
(cd "$root/core" && cargo run --quiet --no-default-features --features peer-transport-qa --bin uniffi-bindgen -- \
  generate --library "$root/core/target/release/libpennel_core.dylib" --language swift --out-dir "$bindings")
cp "$bindings/pennel_coreFFI.h" "$bindings/headers/"
cp "$bindings/pennel_coreFFI.modulemap" "$bindings/headers/module.modulemap"
swiftc -swift-version 6 -O -D PENNEL_PEER_TRANSPORT_QA -module-name PeerComparisonQA \
  -I "$bindings/headers" -L "$root/core/target/release" -lpennel_core \
  -Xlinker -rpath -Xlinker "$root/core/target/release" \
  "$bindings/pennel_core.swift" \
  "$root/apps/mobile/native/ios/Services/PeerComparisonTransport.swift" \
  "$root/apps/mobile/native/ios/Services/PeerComparisonBridge.swift" \
  "$root/scripts/peer-comparison-native-cli.swift" -o "$output/bin/peer-comparison-probe"
cargo build --manifest-path "$root/tools/peer-comparison-relay/Cargo.toml" --release
echo "Prepared QA probes and relay; signing and physical-device qualification remain separate."
