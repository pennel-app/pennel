#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
crate="$root/core/keyboard"
artifacts="$root/apps/mobile/artifacts/ios"
framework="$artifacts/PennelTypingFFI.xcframework"
case "${PENNEL_IOS_ARCHIVE_ONLY-0}" in
  0) targets=(aarch64-apple-ios aarch64-apple-ios-sim) ;;
  1) targets=(aarch64-apple-ios) ;;
  *) echo "PENNEL_IOS_ARCHIVE_ONLY must be 0 or 1" >&2; exit 2 ;;
esac
export IPHONEOS_DEPLOYMENT_TARGET=26.0

for target in "${targets[@]}"; do
  cargo build --manifest-path "$crate/Cargo.toml" --release --target "$target" --lib
done

mkdir -p "$artifacts"
rm -rf "$framework"
framework_args=(-create-xcframework)
for target in "${targets[@]}"; do
  framework_args+=(-library "$crate/target/$target/release/libpennel_typing.a" -headers "$crate/include")
done
xcodebuild "${framework_args[@]}" -output "$framework" >/dev/null
echo "Prepared isolated keyboard typing policy in $framework"
