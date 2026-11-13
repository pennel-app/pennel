#!/usr/bin/env bash
# A restored cache can retain sherpa's build-script output while its native libraries are absent.
# Invalidate only selected run metadata so Cargo reruns the script without deleting compiled
# products or symbols. Complete native downloads are reused; broken downloads are replaced.
set -euo pipefail
profile_name=debug
target_triple=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile)
      [ "$#" -ge 2 ] || { echo 'A cache profile is required.' >&2; exit 2; }
      case "$2" in debug|release) profile_name="$2" ;; *) echo 'Refusing unknown cache profile.' >&2; exit 2 ;; esac
      shift 2
      ;;
    --target)
      [ "$#" -ge 2 ] || { echo 'A cache target is required.' >&2; exit 2; }
      if [[ ! "$2" =~ ^[a-z0-9_]+-[a-z0-9_]+-[a-z0-9_]+(-[a-z0-9_]+)?$ ]]; then
        echo 'Refusing unsafe cache target.' >&2
        exit 2
      fi
      target_triple="$2"
      shift 2
      ;;
    --) shift; break ;;
    -*) echo 'Unknown cache repair option.' >&2; exit 2 ;;
    *) break ;;
  esac
done
for target in "$@"; do
  [ -d "$target" ] || [ -L "$target" ] || continue
  selected_root="$target"
  [ -z "$target_triple" ] || selected_root="$target/$target_triple"
  profile="$selected_root/$profile_name"
  native="$target/sherpa-onnx-prebuilt"
  new_package="$profile/build/sherpa-onnx-sys"
  for path in "$target" "$selected_root" "$profile" "$profile/build" "$profile/.fingerprint" "$native" "$new_package"; do
    if [ -L "$path" ]; then
      echo 'Refusing symlinked CI sherpa cache path.' >&2
      exit 1
    fi
  done
  # Cargo 1.100 groups artifacts by package/unit. Only the run metadata and
  # its fingerprints are disposable; each unit's out/ can contain products/symbols.
  for unit in "$new_package"/*; do
    [ -e "$unit" ] || [ -L "$unit" ] || continue
    if [ -L "$unit" ] || [ ! -d "$unit" ] || [[ ! "${unit##*/}" =~ ^[0-9a-f]{16}$ ]]; then
      echo 'Refusing unknown CI sherpa unit layout.' >&2
      exit 1
    fi
    for path in "$unit/run" "$unit/fingerprint"; do
      if [ -L "$path" ] || { [ -e "$path" ] && [ ! -d "$path" ]; }; then
        echo 'Refusing unknown CI sherpa metadata directory.' >&2
        exit 1
      fi
    done
    for path in "$unit/run"/{stdout,stderr,root-output,invoked.timestamp} \
                "$unit/fingerprint"/{run-build-script-build-script-build,run-build-script-build-script-build.json}; do
      if [ -L "$path" ] || { [ -e "$path" ] && [ ! -f "$path" ]; }; then
        echo 'Refusing unknown CI sherpa metadata file.' >&2
        exit 1
      fi
    done
  done
  # Validate every path before invalidating anything; outputs in other profiles stay untouched.
  for package in "$profile"/{build,.fingerprint}/sherpa-onnx-sys-*; do
    if [ -L "$package" ]; then
      echo 'Refusing symlinked CI sherpa package metadata.' >&2
      exit 1
    fi
    [ -d "$package" ] || continue
    for name in output stderr root-output invoked.timestamp run-build-script-build-script-build run-build-script-build-script-build.json; do
      if [ -L "$package/$name" ]; then
        echo 'Refusing symlinked CI sherpa metadata file.' >&2
        exit 1
      fi
    done
  done
  incomplete=false
  if [ -d "$native" ] && ! find "$native" -type f \( -name 'libsherpa-onnx-c-api.a' -o -name 'sherpa-onnx-c-api.lib' -o -name 'libsherpa-onnx-c-api.so' \) -print -quit | grep -q .; then
    incomplete=true
    if find "$native" -type l -print -quit | grep -q .; then
      echo 'Refusing symlinked incomplete CI sherpa download.' >&2
      exit 1
    fi
    if find "$native" \( -type d -name '*.dSYM' -o -type f \( \
      -perm -100 -o -perm -010 -o -perm -001 -o -name '*.a' -o -name '*.lib' -o -name '*.so' -o -name '*.so.*' -o \
      -name '*.dll' -o -name '*.dylib' -o -name '*.o' -o -name '*.obj' -o \
      -name '*.rmeta' -o -name '*.rlib' -o -name '*.pdb' -o -name '*.debug' -o \
      -name '*.dwp' -o -name '*.dwo' -o -name '*.tar*' -o -name '*.tgz' -o -name '*.zip' \) \) -print -quit | grep -q .; then
      echo 'Refusing to remove retained native products or recovery archives.' >&2
      exit 1
    fi
  fi
  for package in "$profile"/build/sherpa-onnx-sys-*; do
    [ -d "$package" ] || continue
    for name in output stderr root-output invoked.timestamp; do
      [ ! -f "$package/$name" ] || rm -- "$package/$name"
    done
  done
  for package in "$profile"/.fingerprint/sherpa-onnx-sys-*; do
    [ -d "$package" ] || continue
    for name in run-build-script-build-script-build run-build-script-build-script-build.json; do
      [ ! -f "$package/$name" ] || rm -- "$package/$name"
    done
  done
  for unit in "$new_package"/*; do
    [ -d "$unit" ] || continue
    for path in "$unit/run"/{stdout,stderr,root-output,invoked.timestamp} \
                "$unit/fingerprint"/{run-build-script-build-script-build,run-build-script-build-script-build.json}; do
      [ ! -f "$path" ] || rm -- "$path"
    done
  done
  if [ "$incomplete" = true ]; then
    rm -rf -- "$native"
  fi
done
