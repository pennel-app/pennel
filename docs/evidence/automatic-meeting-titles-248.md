# Automatic meeting and conversation names

11 November 2026. Owner-approved implementation under #248, based on released main `27b75bec`.

## Behavior

New empty-title meetings and conversations immediately receive dated names. Calendar and supplied titles remain protected. The desktop recording/startup recovery flow has no initial title field; the retained header and Rename actions still edit the name. Detected-call paths pass an empty creation title when no calendar title exists.

After Stop, existing speaker/summary/Review processing finishes before optional title work starts. The core writes a local transcript-derived fallback, then permits one selected-text-model request with a fifteen-second deadline at background priority. Its JSON-wrapped untrusted source is limited to 4,000 UTF-8 bytes (first/last excerpt); typed notes and summaries are excluded. The one-line result is limited to sixty characters and preserves the content language. No configured model, timeout, failure or foreground cancellation keeps the local name. Naming runs independently of automatic-summary consent; Meetings settings explicitly explains this transmission.

Conversations use their first submitted question as an immediate local fallback, then start a separate optional title worker only after their first successful answer has been published and its foreground lease released. Empty legacy conversations get a deterministic fallback on load without inference. Nonempty legacy titles are preserved. Reopening never causes a title request.

Additive private title metadata records ownership, revision, value/source fingerprints and consumed attempts. Manual rename (including same-name edits), external title changes, source/configuration changes and newer title revisions reject late results. Live valid title edits claim manual ownership before deferred folder rename. Automatic meeting updates patch only title/owned metadata in frontmatter without moving the folder or changing UID; summary bodies and notes remain exact. Conversation title commits update only title fields against durable current metadata, preserving concurrent drafts and requests. Existing exported UniFFI shapes are unchanged by source review; helpers are Rust-only.

## Verification

- `make core`: release library built.
- `make test`: 1,306 core and 17 typing tests passed; seven existing core ignores retained.
- `scripts/with-toolchain.sh cargo fmt --manifest-path core/Cargo.toml -- --check`: passed.
- `scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings`: passed.
- Desktop backend `cargo test`: 408 passed, including no-calendar detected-session naming. Desktop `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check` also passed; the hosted formatter exposed and corrected layout in that test only, with no production-source change.
- Desktop frontend: 525 tests, TypeScript/Vite and theme checks passed.
- Eight focused core naming tests and 24 conversation tests cover generated/local/failed names, same-name/late overrides, source/configuration/restart/legacy behavior and title-only persistence.
- Twelve EN/NL light/dark synthetic browser frames cover title refresh without replacing notes or losing conversation draft/composition/focus/selection, manual Rename, and recording-start recovery without a title field. Four matching Settings before/after pairs explain the independent naming policy.
- A fresh ARM64 desktop Preview package built and passed strict/deep signature verification with the verified personal team `B538Q2K76C`. Executable SHA-256: `08af9d604774dcd2334b5cc00b8755fd6108b88faee5ea8bf60e52d2230bb1de`. This development package is not installed, launched, notarized or published by this task.
- Shared-core mobile compatibility: typecheck, 17 plugin, seven widget, 160 UI, 116 MobileKit and 217 Mac service tests passed. Final generation/native verification is recorded below.

Local logs, manifests, retained prior products and environment/signing receipts are under ignored `build/meeting-panels-248/`. Old native products were retained with APFS clones before rebuilding; no source, product, symbol or unrelated simulator was deleted. Large native commands use the worktree-storage guard.

## Final iOS compatibility

Both lean ARM64 iOS libraries and their matching Swift/XCFramework generation passed. Clean → non-clean → non-clean generation, final clean Prebuild, canonical locked Pods and six ownership checks passed. Unsigned Release simulator and iPhoneOS builds passed sequentially; hosted native tests passed with 144 runtime/service plus 36 keyboard tests (180 total).

The generated manifest matches final core source SHA-256 `ecfc5fe81a0255f24c2d2a88cc920ef2c96859c19b4c1db72eb3281cd62a64c1`; generated bindings SHA-256 is `daf6056fa775ffadf91cd83a3ae73f004925f7b750a672699b7b9f70cd58ef21`. The retained binding baseline predates current main and is not used to claim task-specific byte equality. Existing exported shape preservation is based on source review plus matched-library runtime/native checks. The exact command/product receipt is `build/meeting-panels-248/mobile/summary.json`.

Commands used the storage guard: `make mobile-test`; `swift test --package-path apps/macos-platform`; `env CI=1 PENNEL_VARIANT=preview make mobile-prebuild`; `npm --prefix apps/mobile run build:ios`; `env PENNEL_IOS_DESTINATION=generic/platform=iOS npm --prefix apps/mobile run build:ios`; and `make mobile-native-test IOS_DESTINATION='platform=iOS Simulator,id=7B5A513C-6538-4F9C-AE8A-8C5DC2FACEA3'`. Simulator tests used ad-hoc signing and an empty development team. The task-owned simulator is shut down; unrelated simulators, Metro and physical devices were untouched.

## Acceptance boundaries

Model-topic quality on real meetings, physical iPhone, keyboard cold start, VoiceOver, signed upgrades, Cloud deployment and store/release acceptance are not established. Synthetic browser/core transports do not become real provider-quality evidence. The desktop hosts the optional AI naming hooks; this change does not add a new iPhone automatic-topic UI or exported naming API. Experimental Decision qualification is separate under #96 and remains gated after the two actual identity-first checks.
