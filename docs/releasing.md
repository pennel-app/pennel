# Releasing Pennel

## Automatic releases (shared app)

The `release` job in `.github/workflows/desktop.yml` publishes the shared Tauri Mac app during the [macOS-first phase approved on 8 November 2026 (#174)](https://github.com/pennel-app/pennel/issues/174). It runs after a push to `main` once the required Mac app, packaging-tool and version jobs passed, and only when `core/`, `apps/desktop/`, `apps/macos-platform/` or `assets/` changed since the newest `v*` tag. Routine Mac core/native-platform and iPhone/mobile validation remain in place. Paused Windows/Linux core jobs and installers are not release prerequisites.

- **Version.** `scripts/desktop-auto-release.py plan` bumps the patch of the newest `v*` tag. For a minor or major release, raise `version` in `apps/desktop/src-tauri/tauri.conf.json` (and `package.json`, `Cargo.toml`); a configured version higher than the bump wins. The first release uses the configured version.
- **Assets.** The tag `v<version>` and a GitHub release containing exactly the current `Pennel_<version>_aarch64.dmg` and `SHA256SUMS.txt`. Automatic Windows/Linux publication is paused; [previously published installers](https://github.com/pennel-app/pennel/releases) remain available.
- **Homebrew.** The job rewrites `Casks/pennel.rb` in [`pennel-app/homebrew-pennel`](https://github.com/pennel-app/homebrew-pennel) through the `HOMEBREW_TAP_DEPLOY_KEY` secret (a write deploy key on that repository). Users install with `brew install --cask pennel-app/pennel/pennel`.
- **Signing.** Mac builds are signed with the self-signed certificate *Pennel Release Signing* (hardened runtime, `Pennel.entitlements`), not notarized. macOS ties Microphone, Accessibility, Input Monitoring and Audio Capture approvals to the signature's designated requirement, `identifier "dev.pennel.Pennel" and certificate root = H"<sha1>"`; keeping the certificate stable preserves that requirement across `brew upgrade`. Gatekeeper does not trust the certificate, so the cask still removes the quarantine attribute after install. Previously published Windows installers are unsigned; no in-app updater manifest is published. Developer ID/notarization and installed signing/upgrade acceptance remain separate under [#140](https://github.com/pennel-app/pennel/issues/140). Once the required signing/notarization configuration and Tauri updater key exist, use the signed path of `scripts/desktop-release.py` (switching to the Developer ID changes the requirement once, so users grant permissions one more time).
- **Release certificate.** Stored as the secrets `APPLE_CERTIFICATE` (base64 `.p12`) and `APPLE_CERTIFICATE_PASSWORD` and the variable `APPLE_SIGNING_IDENTITY` (`Pennel Release Signing`); valid until 2046. No copy exists outside GitHub. `scripts/import-mac-signing-cert.sh` imports it into a throwaway keychain on the runner, because Tauri's own `APPLE_CERTIFICATE` import only accepts Apple-issued identities. The bundle inspection fails when the designated requirement differs from `PENNEL_RELEASE_CERT_SHA1` in `desktop.yml`. Runs without the secret (forks) sign ad hoc. Replacing the certificate resets every user's approvals: create it like `scripts/create-dev-cert.sh` does, update the secrets and `PENNEL_RELEASE_CERT_SHA1` together, and say so in the release notes.
- Main runs queue instead of cancelling each other, so a release is never interrupted halfway.

## Manual platform validation

`desktop.yml` and `decision-evaluation.yml` dispatches default to `platforms=macos`. To include the retained Windows/Linux core tests, desktop installers and evaluation targets, explicitly run:

```sh
gh workflow run desktop.yml -f platforms=all
gh workflow run decision-evaluation.yml -f platforms=all
```

Manual validation never publishes a release. Resuming routine Windows/Linux validation or automatic publication requires a deliberate policy/workflow change and current full-platform evidence; an `all` run does not resume them. The Mac build still needs its upstream Sherpa download, so narrowing platform scope does not resolve that dependency failure or waive Mac checks.

## Signed shared candidates and legacy Mac upgrades

`scripts/desktop-release.py` prepares signed Tauri candidates without publishing or rewriting tracked versions. It requires the existing Tauri updater key/public key and platform signing configuration. Mac candidates also require an existing Developer ID identity, notarization credentials, and a build number greater than the last distributed native or shared build. See [candidate and update artifact gates](linux-packaging.md#candidate-and-update-artifacts).

The manual `desktop-candidate.yml` workflow defaults to `platforms=macos`. Select `platforms=all` to include Windows/Linux candidates, together with its existing required `version`, `mac_build_number` and `previous_mac_build_number` inputs and the signing configuration above. It only prepares candidate artifacts and never publishes a release.

Existing native Mac installs still need their Sparkle-to-Tauri transition. Keep the existing Sparkle signing key in Keychain account `pennel`; `SUPublicEDKey` remains in `apps/desktop/src-tauri/Info.plist`. The shared release helper's `--legacy-sparkle` option creates the replacement app archive and appcast using that existing identity.

Set `PENNEL_SPARKLE_BIN` to the directory containing an independently available Sparkle `generate_appcast` executable. The helper requires this explicit path; it no longer resolves tools from the deleted Mac Swift package. For example, after supplying the required existing signing/notarization/updater configuration:

```sh
export PENNEL_SPARKLE_BIN=/absolute/path/to/Sparkle/bin
python3 scripts/desktop-release.py <version> \
  --build-number <new-build-number> --previous-build-number <last-distributed-build-number> \
  --legacy-sparkle
```

The command prepares artifacts; publication and deployment remain separate. Validate the exact candidate archives, previous-installation upgrade/recovery, signature/identity and appcast before promoting them. Preserve the existing key, bundle identity, credential relationships and migration handling. The retired SwiftUI shell is not needed to generate a transition archive.

## Historical native releases

The old `scripts/release.sh` flow built and published the frozen SwiftUI app and has been removed with [cleanup #75](https://github.com/pennel-app/pennel/issues/75). Its [pre-retirement instructions](https://github.com/pennel-app/pennel/blob/e6f02b2cd178b541dcbfcdb46125336c762dbf07/docs/releasing.md) and [source](https://github.com/pennel-app/pennel/blob/e6f02b2cd178b541dcbfcdb46125336c762dbf07/scripts/release.sh) remain historical records. Current desktop releases use the shared workflows above; source retirement does not establish device, signed-update or release acceptance.
