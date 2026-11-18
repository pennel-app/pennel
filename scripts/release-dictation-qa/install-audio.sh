#!/usr/bin/env bash
set -euo pipefail

[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted ]]
[[ "${GITHUB_REPOSITORY:-}" == pennel-app/pennel && "${GITHUB_REF:-}" == refs/heads/codex/mac-release-loopback-dictation ]]
[[ "$(uname -sm)" == "Darwin arm64" ]]

package="$RUNNER_TEMP/BlackHole2ch-0.7.1.pkg"
digest='57b540f27a3e29c37e310e01bee0fdfab76733087e47f997ef9dccf851400dcf'
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
  'https://existential.audio/downloads/BlackHole2ch-0.7.1.pkg' --output "$package"
printf '%s  %s\n' "$digest" "$package" | shasum -a 256 -c -
pkgutil --check-signature "$package" | tee "$RUNNER_TEMP/blackhole-installer-signature.txt"
grep -Eq 'Developer ID Installer: (Existential Audio|Devin Roth)' "$RUNNER_TEMP/blackhole-installer-signature.txt"
spctl --assess -vv --type install "$package"
sudo /usr/sbin/installer -pkg "$package" -target /
if ! system_profiler SPAudioDataType | grep -q 'BlackHole 2ch:'; then
  # The vendor documents restarting Core Audio to load the HAL plug-in.
  sudo killall coreaudiod
  for attempt in {1..15}; do
    if system_profiler SPAudioDataType | grep -q 'BlackHole 2ch:'; then break; fi
    sleep 2
  done
fi
system_profiler SPAudioDataType
