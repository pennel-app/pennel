#!/usr/bin/env bash
set -euo pipefail

root="${CI_PRIMARY_REPOSITORY_PATH:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$root"
export PENNEL_VARIANT=preview
export PENNEL_IOS_ARCHIVE_ONLY=1
export IPHONEOS_DEPLOYMENT_TARGET=26.0
export RUSTUP_TOOLCHAIN=1.99.1
export HOMEBREW_NO_AUTO_UPDATE=1
helper="$root/scripts/mobile-native-cache.py"
python3 "$helper" validate-environment --root "$root"

phase() {
  local label="$1" started="$SECONDS" result=0
  shift
  "$@" || result=$?
  python3 "$helper" record --root "$root" --phase "$label" --elapsed "$((SECONDS - started))" --exit-code "$result"
  return "$result"
}

# The selected source is an ancestor of the generated snapshot; shallow clones may omit it.
source_sha="$(python3 - "$root/apps/mobile/ios/pennel-testflight.json" <<'PY'
import json,re,sys
source=json.load(open(sys.argv[1]))['source_sha']
if not re.fullmatch('[0-9a-f]{40}',source): raise SystemExit('Invalid selected source SHA')
print(source)
PY
)"
if ! git cat-file -e "$source_sha^{commit}" 2>/dev/null; then
  git fetch --no-tags --depth=1 origin "$source_sha"
fi
phase snapshot-validation python3 "$helper" validate-snapshot --root "$root"

if [ "$(xcodebuild -version | sed -n '1p')" != 'Xcode 27.0' ]; then
  echo 'This Preview workflow requires the validated Xcode 27.0 environment.' >&2
  exit 1
fi

prepare_tools() {
  local tool_prefix gem_directory
  if ! command -v node >/dev/null || [ "$(node -p 'process.versions.node.split(".")[0]')" != 22 ]; then
    brew list node@22 >/dev/null 2>&1 || brew install node@22 || return
    tool_prefix="$(brew --prefix node@22)" || return
    export PATH="$tool_prefix/bin:$PATH"
  fi
  [ "$(node -p 'process.versions.node.split(".")[0]')" = 22 ] || return
  if ! command -v rsvg-convert >/dev/null; then
    brew list librsvg >/dev/null 2>&1 || brew install librsvg || return
    tool_prefix="$(brew --prefix librsvg)" || return
    export PATH="$tool_prefix/bin:$PATH"
  fi
  command -v rsvg-convert >/dev/null || return

  export PATH="$HOME/.cargo/bin:$PATH"
  if ! command -v rustup >/dev/null; then
    local installer
    installer="$(mktemp -t pennel-rustup)" || return
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$installer" || return
    sh "$installer" -y --profile minimal --default-toolchain 1.99.1 --no-modify-path || return
    rm "$installer" || return
  fi
  rustup toolchain install 1.99.1 --profile minimal --no-self-update || return
  rustup target add --toolchain 1.99.1 aarch64-apple-ios || return
  case "$(rustc --version)" in 'rustc 1.99.1 '*) ;; *) echo 'Rust 1.99.1 is required.' >&2; return 1 ;; esac

  local pod_version
  pod_version="$(python3 - "$root/apps/mobile/Podfile.lock" <<'PY'
import re,sys
match=re.search(r'^COCOAPODS: ([0-9]+\.[0-9]+\.[0-9]+)$',open(sys.argv[1]).read(),re.M)
if not match: raise SystemExit('The canonical CocoaPods version is missing')
print(match[1])
PY
)" || return
  if ! command -v pod >/dev/null || [ "$(pod "_${pod_version}_" --version 2>/dev/null | tail -1)" != "$pod_version" ]; then
    if ! command -v ruby >/dev/null || ! ruby -e 'exit(Gem::Version.new(RUBY_VERSION) >= Gem::Version.new("3.1") ? 0 : 1)'; then
      brew list ruby >/dev/null 2>&1 || brew install ruby || return
      tool_prefix="$(brew --prefix ruby)" || return
      export PATH="$tool_prefix/bin:$PATH"
    fi
    gem install --user-install cocoapods --version "$pod_version" --no-document || return
    gem_directory="$(ruby -e 'puts Gem.user_dir')" || return
    export PATH="$gem_directory/bin:$PATH"
  fi
  [ "$(pod "_${pod_version}_" --version 2>/dev/null | tail -1)" = "$pod_version" ] || return
  export PENNEL_COCOAPODS_VERSION="$pod_version"

  python3 - "$root/apps/mobile/ios/.xcode.env.local" "$(command -v node)" <<'PY'
import pathlib,shlex,sys
pathlib.Path(sys.argv[1]).write_text('export NODE_BINARY='+shlex.quote(sys.argv[2])+'\n')
PY
}

phase toolchain-setup prepare_tools
phase assets-prepare bash "$root/scripts/prepare-mobile-assets.sh"
phase npm-install npm --prefix "$root/apps/mobile" ci

if phase native-cache-restore python3 "$helper" restore --root "$root"; then
  cache_hit=true
else
  result=$?
  if [ "$result" -ne 3 ]; then exit "$result"; fi
  cache_hit=false
  phase native-preparation bash "$root/scripts/with-toolchain.sh" bash "$root/scripts/build-mobile-core-ios.sh"
  phase native-cache-store python3 "$helper" store --root "$root"
fi
python3 "$helper" record --root "$root" --phase native-cache-result --elapsed 0 --cache-hit "$cache_hit"
phase native-links node "$root/apps/mobile/plugins/native-sources.cjs"
cmp "$root/apps/mobile/Podfile.lock" "$root/apps/mobile/ios/Podfile.lock"
install_pods() { (cd "$root/apps/mobile/ios" && pod "_${PENNEL_COCOAPODS_VERSION:?}_" install --deployment); }
phase pod-install install_pods
phase generated-seal python3 "$helper" seal-pods --root "$root"
