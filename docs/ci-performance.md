# CI performance, validation qualification and compiler-cache pilot

## Current macOS-first phase (8 November 2026)

The user-approved [macOS-first phase (#174)](https://github.com/pennel-app/pennel/issues/174) pauses routine PR/main Windows/Linux product tests, builds, installers and automatic publication. Mac core/native-platform checks, full Mac desktop frontend/backend/build/package checks and current iPhone/mobile checks remain. Shared platform source is retained. Linux selector, Python packaging-helper and release/status jobs remain lightweight utilities, without establishing Linux product validation.

Manual `desktop.yml` and `decision-evaluation.yml` dispatches default to `platforms=macos`. Explicit full-platform validation remains available:

```sh
gh workflow run desktop.yml -f platforms=all
gh workflow run decision-evaluation.yml -f platforms=all
```

`all` restores the retained Windows/Linux core tests and installers in the desktop workflow and all evaluation targets in decision evaluation. The [signed candidate workflow](releasing.md#signed-shared-candidates-and-legacy-mac-upgrades) also defaults to Mac and accepts `all` with its existing required version/build inputs. Manual validation never publishes a release. Resuming routine Windows/Linux validation or automatic publication requires a deliberate policy/workflow change backed by current full-platform evidence; one manual run does not resume them.

Automatic releases now contain exactly the current Mac aarch64 DMG and `SHA256SUMS.txt`, with the existing Homebrew cask update and serialized main pipeline. Mac app, packaging-tool and version gates remain required. The Mac build still needs its upstream Sherpa download; reducing OS scope does not resolve that dependency failure or waive Mac checks. See [releasing](releasing.md) for signing and acceptance boundaries.

This phase changes validation coverage, not scheduling qualification. The full-platform [#73](https://github.com/pennel-app/pennel/issues/73) campaign below remains historical evidence for its frozen sources and selected jobs; it does not qualify the reduced matrix or establish a new completion-time claim. Routine native CAS remains off.

## Selected TestFlight archives

The [Preview distribution route](mobile-testflight.md) adds an independent Xcode Cloud archive lane. It reuses exact-source validation, prepares only device Rust libraries and retains complete checksummed native release inputs in Cloud DerivedData. This does not qualify or enable the GitHub CAS experiment below.

Measure publisher preparation, ready-job queue, toolchain/dependencies, native cache restore/build, archive/signing, upload and Apple processing separately. Preserve source/toolchain/configuration identities, native hit/miss and artifact/binding hashes. Compare one cold and at least three matched warm observations with equivalent archive controls, including setup and transfer costs. Installation/update on both iPhones is separate acceptance. No measured Cloud speedup is established merely by adding the pipeline.

The original CI acceleration work is tracked in [#73](https://github.com/pennel-app/pennel/issues/73). Its initial design preserved the then-required affected-platform coverage on standard hosted runners. Desktop release planning/build/publication remains serialized so concurrent main pushes cannot choose the same automatic version.

## Cache ownership

Routine Rust dependency caches are saved by successful main pushes; pull requests restore available dependency caches. The desktop workflow's explicit manual `cache_probe` exception, with `platforms=all`, may save branch-scoped Linux caches to verify stable hosted cache keys before merge. A probe must show a subsequent exact cache hit before obsolete Linux entries are retired. Do not remove working main caches merely to create space for an experiment.

Hosted Linux uses a stable Rust installation path. Self-hosted runner homes remain isolated. Compiler/environment fingerprints remain enabled; compiler upgrades or relevant configuration changes legitimately produce new keys.

<a id="complete-pr-validation-qualification"></a>

## Historical complete-PR qualification (5–7 November 2026)

The [adopted design](https://github.com/pennel-app/pennel/issues/153#issuecomment-6642797787)
[passed all eight qualification gates](evidence/ci-qualification-73-rolling.md)
under the approved rolling-build protocol: 80 active observations pass, while
all 112 outcomes retain two historical failures. This campaign included the
then-selected Mac, Windows and Linux desktop/core jobs and iPhone validation.
The implementation selects
candidate/warm for ordinary same-repository PRs and exact main pushes. Manual,
privileged, foreign-head and other push contexts remain serial. The serial
override and scoped serial controls remain available. This is separate from
the native compiler-cache experiment and the reduced matrix under #174. It does
not qualify or measure the reduced matrix, or claim a code merge or release.

The [first 5 November qualification checkpoint](evidence/ci-qualification-73.md)
retains the original exact-image assessment and every physical workflow attempt.
Lander subsequently [approved rolling image build numbers](https://github.com/pennel-app/pennel/issues/152#issuecomment-6664508244)
while retaining source, toolchain and cache matching. The
[rolling-build qualification](evidence/ci-qualification-73-rolling.md) reports
the complete matched dataset, raw history, source strata and limitations.
Its [exact frozen base/head/merge scope](evidence/ci-qualification-73-rolling.md#clock-and-source-scope)
is benchmark PRs #158 (docs), #160 (mobile), #170 (desktop), #171 (core warm)
and #173 (core cold). Core warm and cold use separately frozen sources; counts
and medians never pool sources within a category/cache gate. Retain those
reports, figures and failed observations unchanged; they do not measure the
macOS-first phase.

| Affected PR category | Initial complete-cohort objective |
| --- | ---: |
| Documentation | 5 minutes |
| Desktop/frontend | 20 minutes |
| Mobile | 60 minutes |
| Core/shared, including mobile | 60 minutes |

The [accepted measurement policy](https://github.com/pennel-app/pennel/issues/152#issuecomment-6642448420)
uses the median of at least five matched complete candidate/control PR
observations per affected category. Waiting and orchestration count. Report every
observation and miss; five samples establish neither a p95 nor a maximum.
Cold/toolchain-changing controls are separate and may regress by no more than
10% in matched median elapsed time. The mobile 35–40-minute stretch goal is not
the initial acceptance threshold. A timing miss never skips a check or shortens
its timeout.

Cold observations have no adopted absolute completion ceiling. Their derived
`target_miss` value is `null`/N/A; the relative median gate still applies. Warm
misses and all original timing values remain reported unchanged.

The offline evaluation matrix pins its observed Python binary patch by OS:
3.11.18 on Linux and 3.11.9 on Mac/Windows. A bare `3.11` selector resolved
different Linux patches in the initial comparison. Stabilizing this input keeps
every evaluation check intact; earlier captures remain recorded against their
original frozen source, and a new core comparison uses the pinned source.

### Candidate and fallback

The candidate runs two mobile jobs in separate hosted VMs. The original job
retains full common checks, preparation, clean/repeated Prebuild, pod ownership,
and sequential Release simulator/device builds. The other job independently
prepares its complete workspace and runs Debug runtime/keyboard tests. Neither
shares generated projects, Pods, mutable framework links, build directories or
native products. A final Linux result checks the expected job and phase set and
rejects missing, cancelled, failed or unexpectedly skipped selected work.

Each selected desktop OS retains frontend tests, backend checks, installers and
package inspection. In candidate mode Tauri's hook performs the single production
frontend build; backend compilation checks follow packaging so frontend assets
exist. Serial mode retains the original explicit frontend build and ordering.
Uploads and release eligibility still require every selected check. Main release
version planning/build/publication remains serialized.

Both mobile jobs use the existing Rust cache family. The original job remains
the only successful-main writer; the new Debug job restores only. Static key
compatibility does not prove an exact physical restore: record cache version,
paths, compiler/configuration identity and the observed hit in each lane.
Routine native CAS remains off. Manual compiler-cache dispatches keep their
original complete serial pipeline and receipt contract.

### Scoped mode selection

`scripts/ci-mode.py` selects the qualified candidate by default only after its
same-repository PR or exact `push`/`refs/heads/main` guards pass. Ordinary receipts
have an empty qualification ID. Context/error defaults stay serial; malformed
evidence cannot promote a candidate. `PENNEL_CI_FORCE_SERIAL=true` overrides
both eligible ordinary contexts with serial/warm.

The optional non-secret repository variable `PENNEL_CI_QUALIFICATION_CONFIG`
selects an exact-source benchmark control or treatment. It requires an exact
same-repository PR number and immutable head match:

```json
{
  "repository": "pennel-app/pennel",
  "pr": 123,
  "head_sha": "0000000000000000000000000000000000000000",
  "mode": "candidate",
  "cohort": "desktop-warm-candidate-1",
  "dependency_cache": "warm"
}
```

Replace the example PR/SHA with the frozen benchmark identities. The five
identity/mode/cohort fields are required; `dependency_cache` is optional and
defaults to `warm`. Valid modes are `serial` and `candidate`; valid dependency
states are `warm` and `cold`. Another PR keeps its ordinary guarded default.
A stale designated head, malformed active configuration or unknown mode fails
safely. Manual dispatch, privileged events, foreign/deleted-head PRs and
non-main or missing-ref pushes retain serial/warm behavior. Exact scoped serial
controls override an ordinary candidate without disabling the fallback. No
setting changes a branch-protection rule or authorizes publication.

Controlled `cold` skips the workflow's configured Rust and npm dependency-cache
restores/saves for that benchmark PR. It neither deletes working caches nor
creates an alternate cache family. All installs, builds and tests still run.
This does not describe an empty machine: image family/architecture and observed
toolchains must still match. Image build numbers may vary under the approved
rolling-build policy. Unobserved cache state must not be labelled controlled cold.

Both native preparation RUN steps set `CARGO_NET_RETRY=6` only after Rust cache
lookup. This uses Cargo's bounded dependency-transport attempts inside the same
invocation; it does not rerun a build, test or workflow. Transfer/backoff time
counts, exhaustion still fails, and actual observed transport warnings remain
in evidence. The step-local setting does not change job/global environment,
Cargo config, cache inputs, compiler flags or the existing family. The prior
DNS-failed source stays failed in the retained history.

The reusable selection job records actual mode, dependency policy, PR/head/merge
identity, workflow run/attempt and cohort in `build/ci-performance/mode.json`,
uploaded as `ci-mode-<attempt>`. Verify those receipts rather than assuming a
rerun reread a mutable variable. Read back variable mutations, preserve any
previous value, and restore only the configuration owned by the experiment.

### Evidence collection

Freeze the benchmark base, head and merge context. Reexecute every applicable
workflow/job for each observation. Record workflow run IDs and attempts;
carried-forward successful siblings or failed-only retries are not fresh full
cohorts. A mobile-only dispatch cannot qualify complete-PR performance.

Keep qualifying counts and medians within one frozen source identity. Five
internally matched pairs from different base/head/merge identities cannot be
pooled into a five-pair qualification. Preserve earlier source waves and report
their results separately when a toolchain or fixture correction changes source.

`scripts/ci-qualification.py capture` records an explicit PR/head, requested
mode, dependency state, category, pair/cohort identity and whole-cohort request
time. Pass explicit workflow run/attempt identities when replaying a cohort.
The expected workflow sets are core CI and desktop for documentation/desktop,
plus mobile for mobile changes, and plus decision evaluation for core/shared
changes. Missing selected work is not success.

Reruns retain their original workflow creation time. Use the newly recorded
cohort request boundary and actual physical job completion times, labelled
**replay-request-to-complete**; keep the unavailable original push-to-dispatch
interval explicit. Report dependency/scheduling intervals separately from job
execution; the exact ready-runner boundary may remain unknown. Do not sum
overlapping job durations.

Capture preserves curated API evidence and validation artifacts. Exact-image
assessment remains the default. For the approved scheduling-design comparison,
pass `--image-policy rolling-build --approval-reference
https://github.com/pennel-app/pennel/issues/152#issuecomment-6664508244` to capture
and assessment. This omits only observed `image_version` values from comparison;
it still requires their presence and retains all raw versions and the separate
exact-image result. Image family/architecture, source, observed toolchains,
configuration, dependency/cache identities and state, actual mode and complete
coverage must match. Unknown or mismatched evidence remains unqualified and is
retained alongside successful observations. Rolling-fleet completion objectives
do not establish a same-image causal speedup. This policy does not change the
independent native-CAS gates below.

Mobile ordinary lane artifacts have separate lane/mode/attempt
names and a combined `mobile-validation-attempt-<attempt>` result; legacy manual
artifacts retain the names documented below.

Archive the complete attempt's metadata, observed log identities and timing ZIPs
before rerunning its workflows. The hosted preflight's next rerun no longer
listed the preceding attempt's artifact names. A newer receipt must never be
substituted for the earlier physical observation. Observer read failures may be
retried without rerunning successful jobs; preserve those read-error snapshots
separately from the physical performance cohort.

Only a passing complete-cohort assessment permits enabling the candidate for
ordinary validation. Until then retain the serial default. If correctness,
cache compatibility, cold behavior or schedule-inclusive targets fail, retain
or restore serial and record the unmet criterion. Never replace whole-cohort
acceptance with a faster individual lane or add paid capacity to make it pass.

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
