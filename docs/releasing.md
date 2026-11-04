# Releasing Pennel

## Automatic releases (shared app)

The `release` job in `.github/workflows/desktop.yml` publishes the shared Tauri app. It runs after a push to `main` once every other job in that run passed, and only when product code, assets or the canonical release/signing inputs changed since the newest `v*` tag. Signing-only fixes therefore cut fresh artifacts too.

- **Version.** `scripts/desktop-auto-release.py plan` bumps the patch of the newest `v*` tag. For a minor or major release, raise `version` in `apps/desktop/src-tauri/tauri.conf.json` (and `package.json`, `Cargo.toml`); a configured version higher than the bump wins. The first release uses the configured version.
- **Assets.** The tag `v<version>` and a GitHub release with `Pennel_<version>_aarch64.dmg`, the Windows NSIS `.exe` and `.msi`, the Linux `.deb` and `.AppImage`, and `SHA256SUMS.txt`.
- **Homebrew.** The job rewrites `Casks/pennel.rb` in [`pennel-app/homebrew-pennel`](https://github.com/pennel-app/homebrew-pennel) through the `HOMEBREW_TAP_DEPLOY_KEY` secret (a write deploy key on that repository). Users install with `brew install --cask pennel-app/pennel/pennel`.
- **Mac signing.** New published Mac builds require a personal-team Developer ID Application certificate, a matching macOS Developer ID provisioning profile, and notarization credentials. Missing, ad-hoc, self-signed or other-team identities fail publication. Non-publishing CI can still build ad-hoc artifacts, whose private credential store is unavailable. Windows/Linux packaging remains independent; signing a Homebrew Mac DMG does not require Windows certificates or Tauri updater keys.
- **Identity.** The canonical bundle and credential service remain `dev.pennel.Pennel`. `scripts/mac_signing.py` verifies the certificate's actual class, Apple trust chain and personal team `B538Q2K76C`; it authenticates the profile's CMS signature, Mac platform, expiry, exact App ID, signing-certificate authorization and private Keychain group. It derives the App ID prefix from the profile and generates one exact group, `<prefix>.dev.pennel.Pennel`, with corresponding signed entitlements. Profiles and generated overlays stay under ignored `build/mac-signing/`; `Contents/embedded.provisionprofile` enters the bundle before signing. This does not enable App Sandbox.
- **Notarization and promotion.** The shared helper notarizes and staples the app, packages and signs the DMG, then notarizes and staples that exact container. It verifies the signed app/profile, tickets, the app inside a read-only mounted DMG, and Gatekeeper assessment of quarantined DMG/app copies. `mac-notarization.json` binds the final DMG SHA-256 and version to those checks. Publication verifies that receipt against the actual bytes. The cask generator removes the legacy quarantine-removal workaround only when given that matching verified receipt; the published tap changes only when those artifacts are promoted. No resources or profiles may be edited or re-signed after notarization.
- **Existing installations.** Earlier published packages used the retained self-signed *Pennel Release Signing* identity and an unnotarized cask workaround. Those assets are retained; the new dedicated credentials below do not replace them. Moving to Developer ID deliberately changes the old signing requirement once. Legacy credential migration can require one macOS approval; subsequent Data Protection Keychain access uses the app's authorized private group. Actual prior-installation credential/permission migration and two-version signed-update acceptance remain required under [#10](https://github.com/pennel-app/pennel/issues/10) and [#11](https://github.com/pennel-app/pennel/issues/11); passing packaging checks alone does not establish them.
- Main runs queue instead of cancelling each other, so a release is never interrupted halfway.

## Signed shared candidates and legacy Mac upgrades

`scripts/desktop-release.py` prepares signed candidates without publishing or rewriting tracked versions. The Mac-only `--homebrew` path does not require an updater key. Explicit native build numbers, when supplied, must exceed the provided prior build; automatic Homebrew builds retain the semantic-version bundle numbering used by the cask packages. A legacy Sparkle transition still needs its explicit higher native build number. The regular candidate path retains the existing Tauri updater key/public-key requirements and generates the Mac updater archive from the final stapled app. See [candidate and update artifact gates](linux-packaging.md#candidate-and-update-artifacts).

Local Mac commands use existing credentials; these tools never generate or replace a certificate or private key:

```sh
export APPLE_SIGNING_IDENTITY='Developer ID Application: Lander Parren (B538Q2K76C)'
export PENNEL_MAC_PROVISIONING_PROFILE=/secure/path/Pennel.provisionprofile
export PENNEL_NOTARY_KEYCHAIN_PROFILE=personal-notary
/Users/landerparren/.local/bin/worktree-storage run -- scripts/with-toolchain.sh \
  python3 scripts/desktop-release.py <version> --homebrew
```

`PENNEL_NOTARY_KEYCHAIN_PROFILE` selects an existing personal notarization profile. Alternatively provide `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`, or the personal `APPLE_ID`, `APPLE_PASSWORD` app-specific password and `APPLE_TEAM_ID=B538Q2K76C`. Errors suppress raw credential/tool output. The build receives the validated identity and signing overlay; notarization runs explicitly afterward.

`make desktop` also requires that personal identity and canonical profile on Mac, prepares the signed private-group overlay and inspects the resulting app. This local build can use dirty source and does not contact the notary; use the clean-checkout `--homebrew` path for exact distribution acceptance. A prerelease semantic version requires explicit increasing numeric build numbers for `CFBundleVersion`.

GitHub uses dedicated settings, retaining the old self-signed settings unchanged:

| Setting | Purpose |
| --- | --- |
| Variable `PENNEL_MAC_DEVELOPER_ID_IDENTITY` | Existing personal Developer ID Application identity name |
| Secrets `PENNEL_MAC_DEVELOPER_ID_CERTIFICATE`, `PENNEL_MAC_DEVELOPER_ID_CERTIFICATE_PASSWORD` | Base64 existing encrypted `.p12` and its password; imported into a temporary runner keychain |
| Secret `PENNEL_MAC_PROVISIONING_PROFILE_BASE64` | Existing canonical Mac Developer ID `.provisionprofile`, decoded into the temporary runner directory |
| Secrets `PENNEL_MAC_NOTARY_API_ISSUER`, `PENNEL_MAC_NOTARY_API_KEY`, `PENNEL_MAC_NOTARY_API_KEY_CONTENT` | Existing personal notarization issuer, key ID and private `.p8` contents |
| Optional secrets `PENNEL_MAC_NOTARY_APPLE_ID`, `PENNEL_MAC_NOTARY_APPLE_PASSWORD` | Personal Apple ID alternative; the workflow fixes the team to `B538Q2K76C` |

The manual candidate workflow defaults to Mac only and a notarized DMG. Choose updater artifacts to require the existing Tauri updater keys, or turn off Mac only to include the existing Windows/Linux candidate paths. Profile expiry is a release constraint: renew it before expiry without changing the App ID prefix or private group. Developer ID signature verification anchors the personal team and bundle identifier rather than a particular leaf certificate, so normal certificate renewal does not require changing the identity policy.

For a credential-capable local Preview, set `PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE` to its separate personal Mac profile and optionally pin its personal `APPLE_SIGNING_IDENTITY`/`PENNEL_SIGN_IDENTITY`, then run `make desktop-preview`. Preparation validates that identity/profile and sanitizes inherited signing/notary credentials before the first Tauri invocation, so the profile and private-group entitlements enter the initial signed bundle. The generated group is `<prefix>.dev.pennel.Pennel.preview`; production profiles/groups are refused. Apple Development profiles authorize registered Macs, so signed runtime tests need an authorized test Mac. An explicitly isolated CI preview can opt into `CI=true PENNEL_ALLOW_ADHOC_PREVIEW=1`; its initial build uses a separate ad-hoc overlay without a profile or private credential access. Work-team fallback and ordinary local ad-hoc fallback are refused.

`make packaging-test` exercises identity/profile rejection, exact private groups, notarization failures, final-byte hashes, promotion/cask gates and packaging-only release detection. Real Developer ID profile validation, notarized installation with quarantine intact, existing-item migration and two different signed builds must still be recorded for the accepted candidate; synthetic fixtures are not runtime acceptance.

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
