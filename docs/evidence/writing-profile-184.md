# Writing-profile preview verification — 8 November 2026

Tracking: [#184](https://github.com/pennel-app/pennel/issues/184). Implementation: [PR #186](https://github.com/pennel-app/pennel/pull/186), code commit `290193df4734cf5158b7740021efb0fa9b9e8698`. Cloud implementation and operational evidence remain in the private tracker.

## Automated verification

| Check | Result |
| --- | --- |
| `make core` | Release core build passed. |
| `make test` | Core: 1,105 passed, seven existing gated tests ignored; keyboard: 17 passed. |
| Core `cargo fmt -- --check` and `cargo clippy --lib --tests -- -D warnings` | Passed. |
| Final `cargo test --manifest-path core/Cargo.toml --no-default-features profile` | 31 passed, including dispatch barriers and account-bound sharing. |
| Desktop `npm run build` / `npm test` | Passed; 415 tests. |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | 275 passed, including late enable versus disable/reset and response/account races. |
| `swift test --package-path apps/macos-platform` | 146 passed. |
| `make mobile-test` | Type/plugin/widget/UI checks passed; MobileKit: 116 passed. |
| `bash scripts/build-mobile-core-ios.sh` | Device/simulator Rust libraries and matching Swift bindings prepared. |
| Mobile `npm run test:prebuild` | Clean and repeated non-clean generation passed in an isolated source copy. |
| Mobile `npm run build:ios` | Unsigned Release arm64 simulator app build passed. |
| `make desktop-preview` | App and DMG built; strict/deep signature verification passed. |

Rust commands use `scripts/with-toolchain.sh` (Rust 1.99.1). Large builds use `worktree-storage run`; no manual whole-worktree, symbol, compiler-cache or QA cleanup was performed. Swift/Xcode: 6.4 / 27.0.

The initial iOS compile found an older generated Pods graph missing an existing summary bridge. Restoring the generated lock from the existing canonical pin and running `pod _1.16.2_ install --deployment` refreshed membership. No dependency versions or tracked mobile source/lockfiles changed. Clean-Prebuild checks ran in an isolated copied workspace, preserving existing generated projects and native artifacts.

## Visual evidence and review

PR #186 embeds English/Dutch light/dark before/after images published only on `pr-assets/pr-186/`. Twenty browser images cover ready, insufficient-evidence, offline and interaction states. Baseline `3272f0abdca5ed8802c50f85850032b43233d2e7` and current frontend use identical synthetic fixture inputs, with matching fixture and per-case hashes. These are headless app renders with mock data, not native-window or human acceptance.

Interaction checks cover persistent overrides, focus restoration, whole-capture exclusion/readiness, pause, independent sharing and confirmed profile deletion. Review found dispatch, late-enable/reset and final-source response races; fixes and deterministic regressions were reviewed again without further material findings.

## Retained Mac preview

- File: ignored `build/writing-profile-verification/Pennel-Preview-writing-profile-290193d.dmg`.
- Size: 16,169,110 bytes.
- SHA-256: `2c3f85be6eee8d97383f701a2713a36ad816dd5c1a18e4b94c936daf1fcb8b7b`.
- Personal Apple Development team: `B538Q2K76C`; strict/deep signature verified. The certificate was explicitly pinned before packaging.
- Previous preview image retained separately before the normal build output was replaced.

## Open acceptance

Hosted checks must pass for the final PR head. Review/merge, private Cloud deployment, actual ChatGPT web and Claude web linking, and the owner's blind 8/10 preference criterion remain distinct gates. The installer does not establish installed-upgrade, native accessibility, real audio, iPhone, model-quality or production acceptance. No physical iPhone session was started.
