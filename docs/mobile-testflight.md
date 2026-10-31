# Pennel Preview on TestFlight

Distribution belongs to [#20](https://github.com/pennel-app/pennel/issues/20); performance evidence belongs to [#73](https://github.com/pennel-app/pennel/issues/73). A configured workflow, a successful archive, a processed TestFlight build and installation on both testers' iPhones are separate acceptance steps.

## Distribution identity

| Setting | Value |
| --- | --- |
| Apple team | Lander Parren / B538Q2K76C |
| App Store Connect team context | 4dc89f53-aed3-450a-9094-b1251bf1dbe3 |
| App Store Connect app | Pennel Preview / 6829248409 |
| App | app.pennel.ios.preview |
| Keyboard | app.pennel.ios.preview.keyboard |
| Widgets | app.pennel.ios.preview.widgets |
| App Group | group.app.pennel.preview |
| Internal test group | Pennel Preview |
| Generated branch | codex/ios-testflight |
| Workspace / scheme | apps/mobile/ios/Pennel.xcworkspace / Pennel |

Preview URL, Keychain and handoff namespaces remain isolated from production. The collaborator uses their existing Developer role with selected app access. Signing stays with the personal team; no certificate/private key is committed or exported to GitHub.

## Select a version

After the publisher is available on the default branch:

```sh
gh workflow run mobile-testflight.yml --ref main -f source_ref=SOURCE_COMMIT_OR_REF -f what_to_test='Short instructions for the testers'
```

The publisher resolves the source once and reuses successful direct-source CI, desktop and mobile runs for that exact SHA. Pull-request merge checks cannot satisfy this gate. Missing checks run against an immutable validation tag; a failed check is retried once. Failure/timeout stops publication.

The selected source must contain this pipeline and the canonical pod lock. Linux installs locked npm dependencies and generates iOS without CocoaPods or Rust compilation. The snapshot includes the complete source layout, an explicit allowlist of generated files, workspace/shared archive scheme, Cloud hooks, frozen pod lock and a checksummed provenance manifest. Test notes are data. Local Node paths, user settings, Pods, build products and credentials are excluded.

The publication commit has the source commit as a parent and retains the previous snapshot as another parent. Publication uses a fast-forward push. Treat the generated branch as an output: edit normal source and select a new version.

## Configure Xcode Cloud

Open the generated workspace in Xcode with dependencies prepared. In the Cloud report navigator, onboard **Pennel Preview** under the personal team. Grant Xcode Cloud access to the Pennel repository only. Configure **Pennel Preview TestFlight**:

- Start on **codex/ios-testflight** branch changes only.
- Pin **Xcode 27.0** and a compatible macOS image. Bootstrap requires **Node 22**, **Rust 1.99.1** and the CocoaPods version in the canonical lock.
- Leave **Clean** disabled for internal distribution so Cloud can reuse DerivedData.
- Use one **Archive** action, **Release**, **iOS**, workspace/scheme above, deployment preparation **TestFlight (Internal Testing Only)**.
- Add the **TestFlight Internal Testing** post-action for **Pennel Preview**.
- After the owner resolves Apple agreements and encryption/export answers, set **PENNEL_APPLE_DECLARATIONS_CONFIRMED=1** in the workflow environment. Both hooks fail before upload without confirmation. This flag is not an encryption classification.

Post-clone verifies source/check/file provenance, bootstraps tools, prepares the canonical icon, installs npm dependencies and prepares device-only Rust artifacts. It recreates native links and runs frozen pod installation. It never runs Prebuild. A post-pod seal protects generated files and pod integration configuration.

Before archiving, the hook validates Preview signing/namespaces and applies **CI_BUILD_NUMBER** to all three shipping targets, preserving the attested marketing version. Xcode Cloud signs and uploads. Do not infer an encryption exemption or set ITSAppUsesNonExemptEncryption to false merely to unblock distribution.

For the current internal Preview, the owner confirmed **no distribution in France** on 31 October 2026. The app uses published encryption primitives outside Apple OS services, including XChaCha20-Poly1305 and rustls. Apple's [documentation table](https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption/) requires the French declaration for standard encryption only when distributing in France. TestFlight Preview generation therefore explicitly sets ITSAppUsesNonExemptEncryption=false for this documentation determination. Ordinary development and production configurations do not infer that value. Review the determination if the cryptography or distribution scope changes; the flag does not mean that the app lacks encryption or establish a broader legal classification.

Membership includes 25 compute hours/month. Check usage and start selected candidates; do not subscribe to paid capacity automatically.

The existing free macos-26 Actions image provides Xcode 26.6 rather than 27. Mobile QA explicitly pins that available version and Rust 1.99.1; the distribution archive is additionally validated locally and in Cloud with Xcode 27.0. CocoaPods execution explicitly selects the canonical lock's gem version. The two Expo evaluated podspecs containing absolute checkout paths use a narrowly scoped location-independent checksum; actual scripts/download paths and every other pod checksum remain unchanged.

## Archive preparation and reuse

```sh
PENNEL_IOS_ARCHIVE_ONLY=1 scripts/with-toolchain.sh bash scripts/build-mobile-core-ios.sh
```

Archive mode builds only aarch64-apple-ios for both Rust crates. UniFFI reads device-dylib metadata; ordinary preparation still includes device and simulator for full QA.

Cloud retains complete release inputs under DerivedData. Compatibility includes native sources, headers/modulemaps, Cargo manifests/locks, preparation helpers, compiler/profile flags and exact Rust/UniFFI/Xcode/SDK identities. Libraries and matching bindings have checksums. Missing/corrupt/incompatible entries rebuild; JavaScript-only edits can reuse compatible native inputs.

Replacing entries/staging artifacts preserves previous release inputs. Retain consuming archives/dSYMs and referenced symbols. These release bundles are outside the automatically evictable local debug compiler-cache category.

## Verification and recovery

```sh
python3 -B -m unittest discover -s scripts/tests -p 'test_mobile*.py'
bash -n scripts/build-mobile-core-ios.sh scripts/build-keyboard-core-ios.sh scripts/xcode-cloud-post-clone.sh scripts/xcode-cloud-pre-xcodebuild.sh scripts/xcode-cloud-post-xcodebuild.sh
actionlint .github/workflows/mobile-testflight.yml .github/workflows/mobile.yml .github/workflows/ci.yml
```

Run existing mobile type/unit, clean/repeated generation, frozen-pod ownership and native checks. Compare device/simulator Swift/header/modulemap bytes. Verify the signed archive's team, identifiers, entitlements, versions and embedded extensions.

The publisher retains validation/publication/snapshot receipts as Actions artifacts. Cloud phase receipts are build/testflight/cloud-timings.json; the post-archive hook emits the same receipt as PENNEL_TESTFLIGHT_TIMINGS in downloadable Cloud logs. Retain logs, archives and dSYMs before Apple's retention expires. Measure queue and Apple processing separately. Compare one cold and three matched warm observations against equivalent controls. Comparing a single archive with the entire QA job does not establish compiler speed.

Install and update through TestFlight on both iPhones; start without Metro and exercise dictation, keyboard and widgets. A development archive or simulator does not establish TestFlight acceptance.

If a candidate fails, keep the previous TestFlight build available, fix source and select another revision. Disable the Cloud workflow/trigger to stop delivery. Preserve existing signing material, production credentials and release evidence.

Sources: [project requirements](https://developer.apple.com/documentation/xcode/setting-up-your-project-to-use-xcode-cloud), [TestFlight distribution](https://developer.apple.com/documentation/xcode/distributing-your-xcode-cloud-builds-through-testflight), [Cloud caching](https://developer.apple.com/documentation/xcode/xcode-cloud-workflow-reference), [included hours](https://developer.apple.com/xcode-cloud/get-started/).
