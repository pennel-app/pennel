# Production writing-profile availability verification — 8 November 2026

Tracking: [#184](https://github.com/pennel-app/pennel/issues/184). Implementation: [PR #187](https://github.com/pennel-app/pennel/pull/187), feature commit `af55516fde93d6d7eb69160528d84a40a24ff976`.

The regular desktop now exposes the existing profile controls, background analysis and style-only relay. Learning and sharing remain off by default. Production and Preview keep their existing separate libraries, profile stores, credentials and Cloud endpoints. No Cloud deployment, dependency or UniFFI contract change is required by this follow-up.

## Automated checks

| Check | Result |
| --- | --- |
| `make core` | Release core built. |
| `make test` | 1,118 core tests passed; seven existing gated tests ignored. Keyboard: 17 passed. |
| Core `cargo fmt -- --check` / `cargo clippy --lib --tests -- -D warnings` | Passed. |
| Desktop `npm ci`, `npm run build`, `npm test` | Passed; 429 tests. |
| Desktop backend `cargo test` | 277 passed. |
| Desktop backend `cargo clippy --all-targets -- -D warnings` | Passed after simplifying a test closure. |
| Theme generation and desktop packaging helpers | Passed; 61 helper tests. |
| `make mobile-test` | Typecheck, 17 plugin, seven widget, 159 UI and 116 MobileKit tests passed. |
| `swift test --package-path apps/macos-platform` | 146 passed. |
| `scripts/build-mobile-core-ios.sh` | Both lean device/simulator targets prepared; all nine generated Swift/header/modulemap files unchanged. |
| Existing generated Pods ownership checks | Six passed. No dependency or generated-project repair needed. |
| Mobile unsigned Release simulator build | ARM64 Preview-isolated app built; no simulator installation or physical-device acceptance claimed. |

Rust uses the pinned 1.99.1 toolchain through `scripts/with-toolchain.sh`. Large native commands use `worktree-storage run`; no manual cache, worktree or artifact deletion was performed. Clean/repeated Prebuild evidence from the original feature remains applicable because Expo configuration, native graph and all generated FFI files are unchanged.

Production regressions exercise actual dictation consent, default-off/reopen behavior, trait/source controls, selected-provider analysis and cancellation, owner enrollment invalidation, separate Preview stores, sharing races and independent meeting-tool permissions. Production fixtures seed temporary meeting libraries before engine startup; tests do not scan the user's default library.

## Visual and interaction evidence

Twelve matched mock before/after pairs cover production ready/off and existing Preview states in English/Dutch and light/dark. Baseline `9c1227bba9651a112e14b1f4a27a9f71d3d6691d` and the changed frontend use identical fixture inputs. Twelve browser flows cover first-use disclosure, cancellation/acceptance, separate sharing, production URL copying, correction/focus, source exclusion, pause and confirmed deletion. Images live only on `pr-assets/pr-187/production-writing-profiles/` and are embedded in the PR.

Independent review found no material boundary issues. Synthetic renders establish frontend behavior, not native or real-account acceptance.

## Installed account acceptance

Production Cloud support was deployed with the original feature. A normal desktop installation, production OAuth linking and live profile retrieval are separate operational checks recorded under #184. Existing Preview connectors continue to use staging until explicitly replaced or supplemented. A successful tool call can correctly report insufficient evidence: a large meeting library alone does not establish owner attribution or readiness. The owner's blind style evaluation remains open.
