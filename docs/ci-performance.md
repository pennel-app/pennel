# CI performance and compiler-cache pilot

## Selected TestFlight archives

The [Preview distribution route](mobile-testflight.md) adds an independent Xcode Cloud archive lane. It reuses exact-source validation, prepares only device Rust libraries and retains complete checksummed native release inputs in Cloud DerivedData. This does not qualify or enable the GitHub CAS experiment below.

Measure publisher preparation, ready-job queue, toolchain/dependencies, native cache restore/build, archive/signing, upload and Apple processing separately. Preserve source/toolchain/configuration identities, native hit/miss and artifact/binding hashes. Compare one cold and at least three matched warm observations with equivalent archive controls, including setup and transfer costs. Installation/update on both iPhones is separate acceptance. No measured Cloud speedup is established merely by adding the pipeline.

The CI acceleration work is tracked in [#73](https://github.com/pennel-app/pennel/issues/73). The initial changes preserve affected-platform coverage and use existing standard hosted runners. Desktop release planning/build/publication remains serialized so concurrent main pushes cannot choose the same automatic version.

## Cache ownership

Routine Rust dependency caches are saved by successful main pushes; pull requests restore available dependency caches. The desktop workflow's explicit manual `cache_probe` exception may save branch-scoped Linux caches to verify stable hosted cache keys before merge. A probe must show a subsequent exact cache hit before obsolete Linux entries are retired. Do not remove working main caches merely to create space for an experiment.

Hosted Linux uses a stable Rust installation path. Self-hosted runner homes remain isolated. Compiler/environment fingerprints remain enabled; compiler upgrades or relevant configuration changes legitimately produce new keys.

## Mobile Xcode compilation-cache experiment

The mobile compiler cache is **off for ordinary pull-request and main runs**. A manual mobile dispatch accepts:

| Input | Values | Meaning |
| --- | --- | --- |
| `xcode_cache` | `off`, `cold`, `warm` | Control, empty-CAS seed producer, or exact-seed consumer |
| `repetitions` | `1`, `3` | Independent jobs on fresh hosted runners; cold permits only one producer |
| `cache_generation` | Identifier such as `pilot-1` | Separates immutable seed snapshots for repeated experiments |

All modes execute the same preview type/unit tests, lean Rust preparation, clean/repeated Prebuild, actual pod-ownership checks, ARM64 Release simulator and device builds, and Debug runtime/keyboard tests. Native configurations run sequentially within each job because Expo scripts switch shared precompiled frameworks. Matrix repetitions use separate runners and workspaces.

The native preparation step finishes before restoring the compiler cache so the fingerprint includes the generated `Podfile.lock` and Podfile properties. The key also includes the exact source revision, Xcode build, device/simulator SDK versions, Rust compiler identity, architecture, preview variant, dependency/configuration fingerprint and experiment generation. A compatible context prefix is reported for inspection; pilot warm runs deliberately accept only the exact seed key. Missing or incompatible seeds fail the experiment before native compilation.

The supported Xcode settings are passed consistently to all three native commands:

```text
COMPILATION_CACHE_ENABLE_CACHING=YES
COMPILATION_CACHE_ENABLE_DIAGNOSTIC_REMARKS=YES
COMPILATION_CACHE_CAS_PATH=<checkout>/build/xcode-compilation-cache
```

After all native commands finish, a cold run packages only that CAS directory into one gzip-level-1 archive using Python's standard library. GitHub persists only the compressed archive. Warm jobs require its exact key, validate a single nonempty CAS root containing regular files/directories, and extract with Python's `data` filter into temporary staging before atomically installing it into an empty CAS directory. Unexpected roots, traversal, links, devices, FIFOs, corrupt archives and duplicate member paths fail the experiment. The archive format uses a `v2` key prefix, separate from earlier directory-cache experiments.

Generated projects, Pods, DerivedData build databases, signing material and product data are not added to the archive. Release and Debug builds keep their separate DerivedData directories; a successful cold job removes those completed build outputs before packing to leave room for the snapshot. A cold run bypasses restore and refuses to overwrite an existing immutable seed; use a new generation for another cold experiment.

Before building a cold experiment, a storage preflight refuses repositories already lacking 100 MB headroom. Before saving a successful cold seed, the workflow reads usage again and requires current usage plus the **actual compressed archive** size and 100 MB headroom to fit within 10,000,000,000 bytes. The reserve covers the Actions cache's outer archive overhead; its stored size is recorded when available. An empty CAS or insufficient capacity fails the seed explicitly. This guard does not increase cache storage or delete other caches. Cache API usage can lag recent deletions, and other concurrent uploads can consume capacity after the check; warm runs must independently prove that the exact seed was stored and restored.

## Running a matched experiment

Use one benchmark PR and freeze its branch at a single commit during measurement. Dispatch against that branch, rather than comparing a PR merge commit with a branch-head dispatch:

```sh
gh workflow run mobile.yml --ref codex/73-faster-ci -f xcode_cache=off -f repetitions=3 -f cache_generation=pilot-1
gh workflow run mobile.yml --ref codex/73-faster-ci -f xcode_cache=cold -f repetitions=1 -f cache_generation=pilot-1
gh workflow run mobile.yml --ref codex/73-faster-ci -f xcode_cache=warm -f repetitions=3 -f cache_generation=pilot-1
```

Wait for each dispatch to finish before starting the next: mobile workflow concurrency cancels superseded runs on the same ref. Successful cold seeding must precede the warm dispatch. The same source revision, Xcode/SDK fingerprint and generated dependency lock must match across all receipts; a hosted image/toolchain change invalidates the comparison. The three warm jobs each restore the same immutable seed, rather than warming one another's build directories.

Every job uploads a seven-day `mobile-ci-<mode>-<repetition>` artifact containing `mobile.json` and writes a step summary. Receipts contain phase durations/exit codes (including packing/extraction), revision/toolchain/configuration fingerprints, run ID/attempt, runner image/name, exact cache-hit state, raw CAS and compressed archive bytes, available stored cache bytes and restore/save timings. Ephemeral runner names do not contribute to cache keys. Receipts contain no dictated text, meetings, credentials or command arguments. The completed GitHub job's timestamps remain authoritative for total execution and queue time; phase sums exclude setup/install overhead. Cache API timing/size metrics may be temporarily unavailable and are identified in the receipt.

Keep ordinary caching off unless at least three matched warm runs show **20% lower median total mobile-job execution**, the cold run regresses by no more than **10%** against the matched controls, every native/generation test passes, and storage fits the existing allowance. Include packing, transfer and extraction time in the result. A local incremental build or an exact cache hit alone does not establish a speed improvement or cross-runner portability.

## Local checks and rollback

Run the portable helper regressions without Xcode/network access:

```sh
python3 -B -m unittest discover -s scripts/tests -p 'test_ci_*.py'
bash -n scripts/ci-xcodebuild.sh scripts/build-mobile-ios.sh
```

The complete affected mobile checks remain listed in [mobile verification](mobile.md#build-and-verification). The pilot does not establish signed-device, keyboard cold-start, Cloud deployment or App Review acceptance.

Returning dispatch mode to `off` bypasses all compilation-cache settings and restore/save steps. Ordinary CI already defaults to that mode. Reverting the CI acceleration commit restores the previous scheduling/helper behavior without a data or credential migration.

Sources: [Apple build settings](https://developer.apple.com/documentation/xcode/build-settings-reference), [Xcode CAS defaults](https://github.com/swiftlang/swift-build/blob/main/Sources/SWBCore/Settings/Settings.swift#L1961-L1974), [GitHub caching reference](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching).
