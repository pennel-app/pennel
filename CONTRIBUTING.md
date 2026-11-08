# Contributing to Pennel

Thanks for helping. Pennel aims to stay small, fast and private, so a few ground rules:

## Before you start

- For anything bigger than a bug fix, open an issue first so we can agree on the approach. The [desktop roadmap](docs/desktop-roadmap.md) collects the current acceptance and release work.
- Keep the app light: no new dependency without a clear reason, no Electron, no telemetry.
- Privacy is a feature: never log or send dictated text or meeting content anywhere the user didn't choose.

## Where changes belong

- `core/` owns shared product behavior, providers, models, recording, storage and processing.
- `apps/desktop/` is the Tauri 2 application for macOS, Windows and Linux. Keep its interface and copy consistent across platforms; explain unavailable platform features visibly.
- `apps/macos-platform/` provides Apple system integrations through an embedded Swift library.
- `apps/mobile/` is the Expo/React Native application. Keep owned native services, MobileKit, keyboard, widget sources and native tests under `apps/mobile/native/ios/`; generated `ios/` and `android/` projects are not edited or committed.

The old SwiftUI shells have been retired; their historical source is available in Git. Device, signed-upgrade, keyboard, Cloud and store acceptance remain separate work.

See [desktop development](docs/desktop.md#build-and-test) and [mobile development](docs/mobile.md#build-and-verification) for prerequisites, previews, generation and platform-specific setup. Shared design assets and writing guidance live in the [brand guide](assets/brand/brand.md).

## Workflow

1. Fork, create a branch from `main`.
2. Verify the parts your change affects. Add tests for new logic in the shared Rust core, Tauri host, frontend or Swift platform library.
3. Run the relevant checks below. Verify every active consumer affected by shared core, bindings or native-service changes.
4. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:` …).
5. Open a pull request describing what changed and how you tested it. Include OS version, devices and package revision for audio, permission or installer changes; keep private meeting content and credentials out of reports.

Run checks from the repository root:

```sh
python3 scripts/generate-theme.py --check
(cd apps/desktop && npm ci && npm run build && npm test)
cargo test --manifest-path core/Cargo.toml --lib
cargo fmt --manifest-path core/Cargo.toml -- --check
cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
make packaging-test
```

On macOS, also check affected Apple integrations:

```sh
swift test --package-path apps/macos-platform
```

For mobile changes, run the generated-app workflow and preserve preview/production isolation:

```sh
npm --prefix apps/mobile ci
make mobile-test
make mobile-prebuild
npm --prefix apps/mobile run build:ios
PENNEL_IOS_DESTINATION='generic/platform=iOS' npm --prefix apps/mobile run build:ios
make mobile-native-test IOS_DESTINATION='platform=iOS Simulator,id=<assigned-simulator-UUID>'
```

See [mobile verification](docs/mobile.md#build-and-verification) for generation ownership and sequential native-build requirements.

Documentation-only changes need link and rendering checks. UI changes need light/dark app-window captures or owned headless renders. Installed-package and real-device evidence are separate from unit tests and screenshots; use the [pilot checklist](docs/improvement-pilot.md) when recording that evidence.

## Pull request checks

PR jobs run only for affected components. README and documentation changes run
lightweight change-selection tests without building apps. During the
[macOS-first phase](docs/ci-performance.md#current-macos-first-phase-8-november-2026),
desktop changes build the Mac app; core changes also run Mac default/lean and
speaker checks and validate the generated mobile consumer. macOS platform changes
run platform tests and desktop builds. Mobile changes run the generated mobile
workflow, including native service and keyboard tests. Shared assets validate their
active consumers; Tauri bundle configuration also runs packaging tests. Build
scripts, workflows, the Makefile, Cargo configuration, and unrecognized inputs
conservatively run all active checks. Windows/Linux product validation is paused
for routine runs; platform source and explicit manual validation remain.

Selection uses the complete merge-base diff, including deleted and renamed files.
Jobs are skipped within workflows so checks still report a result. Branch pushes
do not duplicate PR builds. Main pushes retain full active Mac/mobile validation;
manual desktop and evaluation dispatches default to `platforms=macos`. Choose
`platforms=all` to validate the retained Windows/Linux targets explicitly. Manual
runs never publish. Eligible main releases publish the Mac DMG and checksums with
the existing Homebrew update; see [releasing](docs/releasing.md).

Component rules live in `scripts/ci-changes.py`, and platform scope is selected by
`scripts/ci-platforms.py`;
verify them with:

```sh
python3 -B -m unittest discover -s scripts/tests -p 'test_ci_changes.py'
python3 -B -m unittest discover -s scripts/tests -p 'test_ci_platforms.py'
actionlint -shellcheck=
```

## Code style

- Clear names and small functions over comments; comment only non-obvious intent, constraints or workarounds.
- Keep canonical defaults, application-owned diagnostics and tool messages in English. Localize UI copy through the shared English/Dutch catalogs; keep strings short and plain. Follow the [language policy](docs/language-policy.md) for generated content and compatibility aliases.

## License

By contributing you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
