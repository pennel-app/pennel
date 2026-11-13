# Native fixture synchronization — 3 November 2026

[PR #129](https://github.com/pennel-app/pennel/pull/129), candidate `9ef8dcf`, failed its [hosted native job](https://github.com/pennel-app/pennel/actions/runs/41661357479/job/126005093860) in `transcriptProjectionCacheCoalescesSessionsAndRetainsTheActualEvidenceRevision()`. The test threw `fixture_timeout` after 43.402 seconds. The runtime suite reported 133 tests with one issue; several unrelated concurrent fixtures took 38–71 seconds. Release simulator/device builds and compilation had succeeded.

The synthetic loader had a five-second semaphore timeout, and its MainActor test polled against a five-second wall-clock deadline. Actor scheduling could consume that budget before the orchestrated handshake resumed. Those limits belonged to the fixture; the production projection cache has neither timeout.

## Repair and preserved assertions

Buffered entry and publication events now drive each step. A condition latch holds each synthetic loader until explicitly released. Every throw, cancellation or ordinary teardown clears the private cache and releases all fixture latches. A private 120-second watchdog runs outside the MainActor, closes event streams and releases loaders on a leaked handshake; it never emits a successful entry or publication. This is a test-only hang bound, not a product latency threshold.

The test still proves that the current request waits behind the physically active old loader, the obsolete session publishes nothing, revision 5 remains readable while revision 6 loads, and exactly two current publications occur. Each loader runs once; shared counters additionally assert a maximum of one physical loader and zero remaining active loaders after publication. Production cache code, deadlines, priority, capacity, credentials and model behavior did not change.

Independent narrow review approved the synchronization, failure paths and unchanged behavioral assertions before commit.

## Verification

Both result bundles were checked with `xcresulttool`, rather than inferring success from cleanup or the command's exit status:

| Check | Result |
|---|---|
| Exact selected cache test | 1 passed, 0 failed, 0 skipped |
| Full hosted native suite | 169 passed, 0 failed, 0 skipped; 177 parameterized device invocations |
| Runtime / keyboard groups | 133 / 36 passed |
| Core/backend formatting and diff check | Passed |

The full-suite cache case completed in 5.370 seconds; the isolated case completed in 0.001 seconds. These are fixture execution times, not app responsiveness or inference measurements. Exploratory selectors that were rejected or selected zero tests were excluded from passing evidence.

From the repository root, with the existing matching #99 Rust libraries/bindings and generated workspace:

```sh
"$HOME/.local/bin/worktree-storage" run -- bash scripts/ci-xcodebuild.sh test \
  -workspace apps/mobile/ios/Pennel.xcworkspace -scheme Pennel -configuration Debug \
  -destination 'platform=iOS Simulator,id=7B0CD7FD-879B-49FF-BCD0-DA64090BD209' \
  -derivedDataPath apps/mobile/build/TestDerivedData \
  -resultBundlePath build/notetaker-2026-11-03/ci/native-fixture-targeted.xcresult \
  '-only-testing:PennelRuntimeTests/PennelRuntimeTests/transcriptProjectionCacheCoalescesSessionsAndRetainsTheActualEvidenceRevision()' \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=
"$HOME/.local/bin/worktree-storage" run -- make mobile-native-test \
  IOS_DESTINATION='platform=iOS Simulator,id=7B0CD7FD-879B-49FF-BCD0-DA64090BD209'
scripts/with-toolchain.sh cargo fmt --manifest-path core/Cargo.toml -- --check
scripts/with-toolchain.sh cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check
git diff --check
```

Ignored evidence is retained under `build/notetaker-2026-11-03/ci/`: the original job's raw/sanitized logs, `native-fixture-targeted-exact.log`, `native-fixture-full-green.log`, both `*-summary.json` files and both `native-fixture-{targeted,full}.xcresult` bundles. The full result was cloned before subsequent Xcode log rotation.

Hosted CI on the repaired commit remains pending. The [original acceptance gates](../live-speakers-99.md#remaining-acceptance), including real-speaker quality, hardware resources, signed iPhone and installed-platform evidence, remain open. No provider or model ran, no asset was downloaded, and no release or deployment occurred.
