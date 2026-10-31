#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
crate="$root/core/keyboard"
artifacts="$root/apps/mobile/artifacts/ios"
framework="$artifacts/PennelTypingFFI.xcframework"
export IPHONEOS_DEPLOYMENT_TARGET=26.0

for target in aarch64-apple-ios aarch64-apple-ios-sim; do
  cargo build --manifest-path "$crate/Cargo.toml" --release --target "$target" --lib
done

mkdir -p "$artifacts"
rm -rf "$framework"
xcodebuild -create-xcframework \
  -library "$crate/target/aarch64-apple-ios/release/libpennel_typing.a" -headers "$crate/include" \
  -library "$crate/target/aarch64-apple-ios-sim/release/libpennel_typing.a" -headers "$crate/include" \
  -output "$framework" >/dev/null
echo "Prepared isolated keyboard typing policy in $framework"
