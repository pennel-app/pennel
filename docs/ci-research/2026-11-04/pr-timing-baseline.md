# Pennel PR check timing baseline — 4 November 2026

Research for [Where does Pennel PR check time go today?](https://github.com/pennel-app/pennel/issues/150), within [Make Pennel PR checks faster without losing coverage](https://github.com/pennel-app/pennel/issues/149). This report supports later latency-target and design decisions; it chooses neither. All current affected-platform validation remains pre-merge, using the existing free hosted runners.

## Findings

1. **Affected scope determines the execution floor.** Audited desktop-only successful heads finish their applicable workflow sets in about 16 minutes; the audited full mobile job alone executes for 45–59 minutes. Mobile native preparation plus Release simulator, Release device and Debug runtime/keyboard validation dominate its execution. These configurations and clean/repeated generation checks are current coverage, not redundant checks to remove.
2. **Scheduling can dominate an otherwise short job.** A successful desktop cohort took 77m17s while its last macOS app job executed for 9m32s, starting 67m44s after the event. A successful mobile cohort took 95m01s, including 36m19s before its one job started. The exact ready-runner boundary is unavailable; these are event-to-start intervals, not proven pure runner-queue measurements.
3. **Ordinary Rust caching works in the five bounded log audits.** Both mobile jobs and all three desktop app OS jobs restored exact Rust-cache keys. They still compiled the required native/configuration work. No observed prefix restore, missing archive or stale key explains those five timings; unaudited cache states remain unknown.
4. **Check selection already avoids substantial work.** Desktop-only audited changes omit mobile/evaluation, and the documentation-only audit executes only selection/status jobs. Broad script/Makefile/unknown-input selection remains conservative. The evidence supports investigating individual overbroad ownership rules; it does not justify removing their validation.
5. **The long tail combines source, scheduling, retries and validation.** There is no defensible p95, matched performance comparison or overall cache speedup in this sample. The earlier native-cache experiment remains unqualified and off.

## Measurement and sample limits

Collected on 2026-11-04, approximately 20:55–21:03 UTC. Freshly verified main: [`543ffe7b18f2e102ad61fb092c31879b18987d17`](https://github.com/pennel-app/pennel/commit/543ffe7b18f2e102ad61fb092c31879b18987d17). Read-only evidence preparation used a private temporary directory; no checkout, index, main ref, CI configuration, runner, cache or credential was changed. Only this report is published on the throwaway research branch; no PR or merge is intended.

The bounded sample is **39 completed head-SHA cohorts / 161 workflow-run records**, from 2026-10-30 20:48:49 UTC through 2026-11-04 20:07:26 UTC. It consists of the newest 32 completed heads in the first three 100-record PR-run API pages, plus seven older category controls: meeting workspace, desktop Glaze, frozen Pod installation, native async tests, TestFlight workflow tooling, worktree configuration, and docs-only. This is a purposeful engineering sample, heavily weighted toward recent core/mixed Notetaker work, not a random sample of Pennel PRs. [Actions API source](https://api.github.com/repos/pennel-app/pennel/actions/runs?event=pull_request&per_page=100&page=1).

The agreed outcome is **push → every applicable check complete**. The original push timestamp and push→workflow-dispatch gap are **unknown** in this collection. The cohort table therefore uses an explicitly weaker, observable proxy: earliest selected workflow `created_at` → latest selected workflow `updated_at`. `updated_at` is terminal workflow metadata, not an independently guaranteed execution-end field. Audited examples below instead use physical job `started_at`/`completed_at`; the workflow update follows their last job by approximately 0–1 seconds. Do not use the table alone to set a precise push-to-green SLA.

For each head, the latest run record per workflow path represents the surviving workflow set. Earlier duplicate run IDs, including same-second events, remain recorded as cancelled work rather than being mistaken for separate heads or speed samples. Of 161 records, 102 show success, 54 cancelled and 5 failure; these are record conclusions, **not counts of physical execution attempts**. Five records have latest `run_attempt=2` and 156 have `run_attempt=1`. Original successful sibling jobs carried forward on reruns must not be counted as new physical observations. Retries remain separate below.

| Latest workflow-set outcome | Cohorts | Observable proxy range | Last terminal workflow |
| --- | ---: | ---: | --- |
| All success, attempt 1 only | 14 | 0m19s–95m01s | mobile 9; desktop 3; ci 2 |
| All success, at least one rerun | 5 | 48m46s–93m11s | mobile 4; desktop 1 |
| Failure present, attempt 1 | 4 | 40m49s–96m49s | mobile 3; desktop 1 |
| Cancellation/supersession remains in latest set | 16 | 1m19s–48m08s | mobile 8; desktop 8 |

Failed/cancelled sets do **not** establish successful all-applicable completion. Some failed sets also contain cancellation. There are 17 currently observed PR heads in the sample: 12 all-success, 2 failure-present and 3 cancelled. The other 22 heads were superseded. “Current head” is the PR metadata observed during collection, not a promise that it stays final.

PR [Simplify Pennel into Dictation and Meetings](https://github.com/pennel-app/pennel/pull/148), head [`c5c335ef4754fe829b67f04d7d3d8e5092a7a715`](https://github.com/pennel-app/pennel/commit/c5c335ef4754fe829b67f04d7d3d8e5092a7a715), was still incomplete when sampled. Its desktop job audit is useful, but its cohort is excluded from completed-head totals. No waiting or workflow dispatch was needed.

## Audited critical paths

These are primary job API timestamps and step durations. Seconds are GitHub's whole-second summaries; setup/post steps and gaps explain why a phase sum need not equal total job execution.

| Exact job/run evidence | Event → job start | Physical job execution | Workflow metadata elapsed | Interpretation |
| --- | ---: | ---: | ---: | --- |
| [Mac Keychain mobile, job 126732636711](https://github.com/pennel-app/pennel/actions/runs/41874349847/job/126732636711) | 10m06s | 52m16s | 62m23s | One independent macos-26 job; no declared job dependency. Other workflows failed, so this is not a green cohort. |
| [People links mobile, job 126581066474](https://github.com/pennel-app/pennel/actions/runs/41831945512/job/126581066474) | 0m04s | 45m17s | 45m22s | Successful full cohort; mobile is its last workflow. |
| [Live speakers mobile, job 126232700761](https://github.com/pennel-app/pennel/actions/runs/41725798670/job/126232700761) | 36m19s | 58m41s | 95m01s | Large scheduling interval plus long native execution. |
| [Incremental qualification macOS app, job 126232275424](https://github.com/pennel-app/pennel/actions/runs/41726226537/job/126232275424) | 67m44s | 9m32s | 77m17s | App dependencies complete at event+1m22s; another 66m22s elapses before start. Exact ready/dispatch boundary unknown. |
| [Scheduling ci build, job 126232430143](https://github.com/pennel-app/pennel/actions/runs/41726225636/job/126232430143) | 56m47s | 6m19s | 63m14s | Selection ends at event+1m40s, leaving 55m07s before build start; status gate then takes 6s. |
| [Current PR148 Windows app, job 126840554945](https://github.com/pennel-app/pennel/actions/runs/41905099778/job/126840554945) | 5m26s | 15m28s | 20m55s desktop | First changes job itself waits 5m04s. App depends on changes and version; version finishes event+5m24s. Windows is the desktop critical job. |
| [Desktop record prompt Windows app, job 126480305397](https://github.com/pennel-app/pennel/actions/runs/41802056861/job/126480305397) | 0m53s | 15m03s | 15m57s desktop | Desktop-only final head; no mobile/evaluation workflow. |
| [Documentation ci selection/status](https://github.com/pennel-app/pennel/actions/runs/41126163968) | first job 0m03s | selection 8s; status 6s | 0m19s ci | Core/platform jobs skipped; companion desktop build jobs skipped. |

`run_started_at` equals `created_at` in the checked examples even where the first job starts minutes later; it is **not** a runner-start measurement. Where the source declares dependencies, their completion yields a dependency-satisfied interval, not proof that GitHub had already admitted the job to a ready-runner queue. Account concurrency, hosted capacity, scheduler and undisclosed ready-state transitions cannot be separated by this API evidence. No attribution to a particular concurrency limit is established.

### Mobile execution

| Phase | Mac Keychain job | People links job | Work retained |
| --- | ---: | ---: | --- |
| Rust cache restore step | 22s | 16s | Exact dependency cache; native CAS off |
| Type/unit regression step | 85s | 84s | JS/TS/plugin/widget/UI, Python helper and Swift helper checks |
| Generate/validate preview workspace | 462s | 391s | Rust device/simulator libraries, bindings, clean/repeated generation and pod ownership |
| Release simulator application | 782s | 708s | ARM64 simulator Release build |
| Unsigned device candidate | 738s | 582s | Device Release build |
| Debug runtime/keyboard regression | 958s | 855s | Separate Debug build/test configuration |
| Total physical job | 3136s | 2717s | Includes other setup/post work |

The four generation/native phases are **2940/3136 seconds (93.8%)** in the Keychain job and **2536/2717 seconds (93.3%)** in People links. These are descriptive phase shares, not removable-work estimates. The same runner image and exact Rust key do not make the two runs a matched experiment: source and PR merge base differ. [Mobile workflow at main](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/.github/workflows/mobile.yml), [generation/native targets](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/Makefile), [lean Rust mobile preparation](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/scripts/build-mobile-core-ios.sh).

The routine job intentionally runs the native configurations sequentially. The shared Expo workspace/precompiled framework preparation prevents treating simultaneous commands against the same directories as an established safe acceleration. Splitting work onto isolated free hosted jobs would be a later design/qualification question, including additional setup and scheduling costs; this report does not choose it. [Existing experiment constraints](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/docs/ci-performance.md).

### Desktop execution and repeated compilation

Current PR148 desktop app execution is Windows **928s**, macOS **631s**, Linux **574s**. Windows spends **624s** in the installer/baseline step, **96s** in Rust lint and **102s** in Rust tests; its Rust-cache restore takes **56s**. The installer step is 67.2% of Windows execution. Linux spends **418s** in installer/baseline work, **32s** in system package setup and **26s** restoring Rust cache. Mac restore is **56s**, installer/baseline work **398s**. The audited desktop-only record-prompt head shows the same Windows-last shape: 903s execution with 597s installer/baseline work. [Current desktop run](https://github.com/pennel-app/pennel/actions/runs/41905099778), [desktop-only run](https://github.com/pennel-app/pennel/actions/runs/41802056861).

The source runs debug lint/test and then distributable Release packaging, across three app OS targets, plus affected core OS validation and a separate no-native-model evaluation target. Mobile additionally builds lean Rust for device/simulator and a host binding generator. Different OS/architecture, feature, profile and validation products make this repeated compilation real work, not interchangeable artifacts. Dependency cache reuse is not proof of product-binary or Xcode object reuse. Any later reuse design must retain CPU-baseline inspection, packaging verification and every current test. [Desktop workflow](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/.github/workflows/desktop.yml), [core/macOS workflow](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/.github/workflows/ci.yml), [evaluation workflow](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/.github/workflows/decision-evaluation.yml).

## Source, image, cache and attempt identity

Bounded log inspection retained only the first 196,608 bytes of each of five jobs, sufficient for setup, checkout, compiler and cache restore output; no full workflow log archive was downloaded. API step summaries provide later durations. Other job image/cache details are **unknown**, so no aggregate is stratified by unobserved warmth.

| Audited jobs | Checked-out source from log | Hosted image | Rust/cache evidence |
| --- | --- | --- | --- |
| Keychain mobile 126732636711 | merge `a9670e8`: head `fcafa0a012` into base `543ffe7b18` | macos-26-arm64 `20261026.0455.2`; Xcode 26.6 | Rust 1.99.1; exact `expo-mobile-lean…fc985acc-9f6ae85c`; native CAS off |
| People links mobile 126581066474 | merge `88ef951`: head `46ac1d2a74` into base `ec2695a396` | macos-26-arm64 `20261026.0455.2`; Xcode 26.6 | Same exact Rust key; native CAS off |
| PR148 Windows app 126840554945 | merge `b54708d`: head `c5c335ef47` into base `543ffe7b18` | windows-2025-vs2026 `20261030.294.1` | Rust 1.99.1; exact `sherpa-prebuilt-app-Windows_NT-x64-e70cdc14-a6385968` |
| PR148 Linux app 126840554961 | same PR148 merge | ubuntu-22.04 `20261101.347.1` | Rust 1.99.1; exact `sherpa-prebuilt-app-Linux-x64-9d2bb98b-d003e3fc`; `/home/runner/.rustup` |
| PR148 macOS app 126840555031 | same PR148 merge | macos-15-arm64 `20261028.0489.1`; Xcode 26.3 | Rust 1.99.1; exact `sherpa-prebuilt-app-Darwin-arm64-fc985acc-d003e3fc` |

Every key above has the `v0-rust-` prefix; mobile includes the `ios-generated-build` job identity. The logs explicitly say `full match: true`; a generic “cache hit” alone would not establish an exact restore. npm restore hits were also present in these prefixes. Log evidence is available through each job link in this report.

The run-list head SHA identifies the PR source cohort; actual checkout is its generated merge with a base revision, as shown above. They must not be confused. Workflow source identities were read at each exact head SHA without checkout, then SHA-256 compared. Abbreviated content hashes:

| Owning source | Cohort-head content groups |
| --- | --- |
| ci.yml | `b1eb8870e0` 37 heads; `7ac372af94` worktree control; `4372f70b4b` docs control |
| desktop.yml | `7154d15d28` 34; `6dbe14a1d8` three Keychain heads; `8fb5612d05` worktree/docs controls |
| mobile.yml | `0624220e3a` 37; `f1d3a83aec` worktree control; `4075f3d5a7` docs control |
| decision-evaluation.yml | `d37e0601ed` all 39 |
| changes.yml / ci-changes.py | one shared version in 38 heads; docs control differs |

The earlier controls are deliberately source-stratified, not folded into a “same CI” performance comparison. Sources: [Keychain head](https://github.com/pennel-app/pennel/tree/fcafa0a012194b4ba39fda86f734bf6ceb8165b1/.github/workflows), [worktree head](https://github.com/pennel-app/pennel/tree/5a5f58a2c3c17529277664718c88fdae78db35bc/.github/workflows), [docs head](https://github.com/pennel-app/pennel/tree/9a185308a052e94edaf66b5777dfe8e9e35438a0/.github/workflows). Changes to imported scripts, Makefile and product source can still change work even when the workflow file hash matches.

### Cache invalidation boundaries

Routine PR jobs restore dependency caches; routine successful main jobs own saves. Job/OS/architecture, compiler/environment fingerprints and dependency/configuration hashes partition identities. A dependency/toolchain/environment change may legitimately select a different exact key or fall back to a compatible prefix; the PR cannot be assumed to produce a shared new main archive before merge. Ephemeral runner names are not supposed to define hosted Linux identities. The current Linux audit confirms the stable home and exact key; it does not requalify every cache. [Cache ownership policy](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/docs/ci-performance.md), [stable Rust setup](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/scripts/ci-rustup.sh).

The sherpa helper deliberately discards restored build-script fingerprints/output so the download script reruns against complete native libraries; that is a correctness guard, not evidence of an accidental stale-cache miss. The workflow preserves `sherpa-onnx-prebuilt` alongside the Rust workspace cache. No cache was deleted or modified for this research. [Owning guard](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/scripts/ci-sherpa-cache.sh).

The native CAS pilot remains **off** in ordinary mobile runs. [Accelerate CI while preserving affected-platform coverage](https://github.com/pennel-app/pennel/issues/73) records only one image-matched warm/control pair, no matched cold control, and a failed storage gate. Its one-pair 38.57% lower execution and mixed-image descriptive medians do not qualify adoption. Today's exact Rust restores are not new native-CAS qualification. No new experiment, seed, cache cleanup or paid capacity is authorized here.

## Check selection and enforcement

The current selector uses the complete merge-base diff, disables rename detection, and selects all known jobs for scripts/workflow/Makefile or unknown build inputs. Core affects core+app; Mac platform affects platform+app; desktop selects app; mobile belongs to its own lane; docs are excluded. Mobile path filtering includes all `core/**`, selected native/generation scripts and Makefile. Evaluation includes `core/**` and its own inputs. Mac platform tests are consolidated with the core Mac job when both are selected. [Selector](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/scripts/ci-changes.py), [reusable selection job](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/.github/workflows/changes.yml).

The audited desktop-only and docs-only cases demonstrate useful avoidance already. The worktree control still ran a 46m25s mobile job and took 46m39s at the workflow metadata boundary, but its exact triggering diff/ownership has not been audited here: it is a candidate for later inspection, not proof that the check was avoidable. The same caution applies to “Mac Keychain” work: its observed diff also touches core/locales/scripts/Makefile, so its title alone cannot justify skipping mobile/evaluation. [Worktree mobile job](https://github.com/pennel-app/pennel/actions/runs/41205150940/job/124548604539), [Keychain exact head](https://github.com/pennel-app/pennel/commit/fcafa0a012194b4ba39fda86f734bf6ceb8165b1).

Live enforcement was checked independently on 2026-11-04: `GET /branches/main` returned `protected=false`, disabled protection and `required_status_checks.enforcement_level=off` with empty checks/contexts. `GET /rules/branches/main` and `GET /rulesets?includes_parents=true` both returned `[]`. Consequently this report makes **no claim that GitHub currently blocks merge on these checks**. The job named `required` is a failure-aware operational status gate, not evidence of branch protection. [Branch API](https://api.github.com/repos/pennel-app/pennel/branches/main), [effective rules API](https://api.github.com/repos/pennel-app/pennel/rules/branches/main), [ruleset API](https://api.github.com/repos/pennel-app/pennel/rulesets?includes_parents=true), [gate source](https://github.com/pennel-app/pennel/blob/543ffe7b18f2e102ad61fb092c31879b18987d17/scripts/ci-status.py). Nothing was changed.

## Completed-head evidence index

Elapsed values below are the weaker workflow-metadata proxy defined above. `final` means the observed current PR head; `prior` means a superseded head. Scope is an audited comparison where available, otherwise explicitly an observed lane-selection category. Changed-file comparisons against the PR base observed now can differ from historical event merge bases; they are not replayed selector receipts. The link on the last workflow opens the exact terminal run; the head links pin source. All values are descriptive, with failed/cancelled cases separate.

| Head SHA | Head role | Scope/category | Latest set | Latest attempts | Metadata elapsed | Last workflow |
| --- | --- | --- | --- | --- | ---: | --- |
| [`829c1cbd7b`](https://github.com/pennel-app/pennel/commit/829c1cbd7b28d19360e7c517abd939e0488b060a) | prior | core/mixed | cancelled | 1 | 14m32s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41903550151) |
| [`fcafa0a012`](https://github.com/pennel-app/pennel/commit/fcafa0a012194b4ba39fda86f734bf6ceb8165b1) | final | core/mixed | failure present | 1 | 62m23s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41874349847) |
| [`90f3a18615`](https://github.com/pennel-app/pennel/commit/90f3a1861525a8435050b64bb5acc441a0570e4a) | prior | core/mixed | cancelled | 1 | 13m36s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41872865429) |
| [`88c62ae357`](https://github.com/pennel-app/pennel/commit/88c62ae3572c9cb93f7602198ae06d2037164f6f) | prior | core/mixed | cancelled | 1 | 1m49s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41872682446) |
| [`46ac1d2a74`](https://github.com/pennel-app/pennel/commit/46ac1d2a744fe085e81a6d24def94f561e84295a) | final | core/mixed | success | 1 | 45m22s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41831945512) |
| [`588209707f`](https://github.com/pennel-app/pennel/commit/588209707fd218989b69fef30c3dfc7a140664bc) | final | core/mixed | success | 1 | 57m06s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41831944797) |
| [`68c4326468`](https://github.com/pennel-app/pennel/commit/68c432646878557fbf9067073a136ed931914587) | prior | core/mixed | cancelled | 1 | 48m08s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41825412813) |
| [`65b5a7c9e2`](https://github.com/pennel-app/pennel/commit/65b5a7c9e25f695f05b90b361c57364aade1f9ea) | prior | core/mixed | success | 1 | 44m50s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41819439953) |
| [`51c53ba3e1`](https://github.com/pennel-app/pennel/commit/51c53ba3e147439211393c443e6fe0634f4c3f58) | prior | core/mixed | cancelled | 1 | 13m09s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41818064012) |
| [`3b6d0524a3`](https://github.com/pennel-app/pennel/commit/3b6d0524a353fd671648652008ad270679814a9b) | prior | core/mixed | success | 1 | 47m59s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41803788448) |
| [`a16628aa9e`](https://github.com/pennel-app/pennel/commit/a16628aa9e679adfdd114829f37c25c547b466da) | prior | core/mixed | cancelled | 1 | 8m15s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41802882336) |
| [`1d9ef671b7`](https://github.com/pennel-app/pennel/commit/1d9ef671b7b9b7bfd8cb0079784f1ce981afd051) | final | desktop only | success | 1 | 15m57s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41802056861) |
| [`25b8bd6c11`](https://github.com/pennel-app/pennel/commit/25b8bd6c11a675c9319fe8e7888f78ecc55c65ba) | prior | desktop only | cancelled | 1 | 4m26s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41801563443) |
| [`f567d5f24f`](https://github.com/pennel-app/pennel/commit/f567d5f24f162961e15173f33bebf896da9d2600) | final | core/mixed | failure present | 1 | 96m49s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41727385713) |
| [`e00bb9ac2b`](https://github.com/pennel-app/pennel/commit/e00bb9ac2b4adcc758ab01de94e7338d6179d19b) | final | core/mixed | success | 1 | 63m14s | [ci](https://github.com/pennel-app/pennel/actions/runs/41726225636) |
| [`f41d428888`](https://github.com/pennel-app/pennel/commit/f41d4288881ad2cba659ef7845e50a1e21b57d18) | final | core/mixed | success | 1 | 77m17s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41726226537) |
| [`119c1dc196`](https://github.com/pennel-app/pennel/commit/119c1dc196e86ed9a952211efef69f7edfcfbb0d) | final | core/mixed | success | 1 | 95m01s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41725798670) |
| [`b2c51f5d3b`](https://github.com/pennel-app/pennel/commit/b2c51f5d3b36cb3b719f397519e5aa9404571c2d) | final | core/evaluation selection | success | 1 | 62m01s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41725053562) |
| [`a1d496883e`](https://github.com/pennel-app/pennel/commit/a1d496883ea662493c387be96b4ba49df00d01cc) | prior | core/mixed | cancelled | 1 | 21m50s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41723203170) |
| [`a89661ae89`](https://github.com/pennel-app/pennel/commit/a89661ae89ecd33d27003a6f8d7541b3fcd9b92a) | prior | core/mixed | cancelled | 1 | 20m24s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41723200771) |
| [`50e22e8a59`](https://github.com/pennel-app/pennel/commit/50e22e8a59eb791c3b4396535a4534f13af3a37c) | prior | core/mixed | cancelled | 1 | 35m34s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41721227718) |
| [`5da3212b7a`](https://github.com/pennel-app/pennel/commit/5da3212b7af116b38d1bb4cc71ec697cb5e8ded3) | prior | core/mixed | failure present | 1 | 40m49s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41719034067) |
| [`10eb92ccb5`](https://github.com/pennel-app/pennel/commit/10eb92ccb5317e13aaee406a7a862945da523a51) | final | desktop only | success | 1 | 16m12s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41718169365) |
| [`bac38c7ed8`](https://github.com/pennel-app/pennel/commit/bac38c7ed8b8965173fffb644ae18b1468fb7531) | prior | desktop only | cancelled | 1 | 2m02s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41717982021) |
| [`f776ba58ba`](https://github.com/pennel-app/pennel/commit/f776ba58ba42ce77ec21d3c4c5e304c1b6f0fa56) | prior | core/mixed | success | 1,2 | 57m00s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41694004955) |
| [`b94119a65e`](https://github.com/pennel-app/pennel/commit/b94119a65e3378c15546d1b1fc3ae656f563ba2d) | prior | core/mixed | success | 1,2 | 58m14s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41692718396) |
| [`8ba325c7ac`](https://github.com/pennel-app/pennel/commit/8ba325c7ac3ef97581eef7959fcd279db492fa14) | prior | core/mixed | failure present | 1 | 69m32s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41678611474) |
| [`52befd1e06`](https://github.com/pennel-app/pennel/commit/52befd1e0674ad38e7d221d869fad0aff2995562) | prior | core/mixed | success | 1,2 | 93m11s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41677400222) |
| [`a8d54b1af5`](https://github.com/pennel-app/pennel/commit/a8d54b1af5248739ab90b31ce2c6ca68f261659c) | prior | core/mixed | success | 1 | 81m56s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41677395052) |
| [`2f6745312e`](https://github.com/pennel-app/pennel/commit/2f6745312e3bf6e6cab095a04c40dd8050996430) | prior | core/evaluation selection | success | 1,2 | 68m06s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41677132798) |
| [`a006e5c026`](https://github.com/pennel-app/pennel/commit/a006e5c026cac62714a4f2c72170e68627f8a69d) | prior | core/evaluation selection | cancelled | 1 | 39m22s | [desktop](https://github.com/pennel-app/pennel/actions/runs/41672788946) |
| [`80ef2e71ed`](https://github.com/pennel-app/pennel/commit/80ef2e71ed2ac379e69ddd781f5630d6829148eb) | prior | core/evaluation selection | cancelled | 1 | 35m05s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41672262334) |
| [`6799661520`](https://github.com/pennel-app/pennel/commit/679966152042c993f0a49be731c8b0fee72e68be) | final | core/evaluation selection | success | 1,2 | 48m46s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41486736247) |
| [`9253723534`](https://github.com/pennel-app/pennel/commit/925372353420a04f3d04497be204a4b253488c8f) | final | mobile/tooling selection | success | 1 | 53m18s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41446962354) |
| [`b163616a96`](https://github.com/pennel-app/pennel/commit/b163616a96bd9cc0305441c5d8f2581e3a0970d2) | final | mobile/tooling selection | cancelled | 1 | 2m24s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41242513021) |
| [`a50c5b2104`](https://github.com/pennel-app/pennel/commit/a50c5b21045129a6b4f7f3a6fac0421eb849a845) | final | mobile/tooling selection | cancelled | 1 | 1m19s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41261657901) |
| [`2824669a97`](https://github.com/pennel-app/pennel/commit/2824669a97983024934928c58e5452bc3bdf2e1b) | final | mobile/tooling selection | cancelled | 1 | 5m18s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41237703510) |
| [`5a5f58a2c3`](https://github.com/pennel-app/pennel/commit/5a5f58a2c3c17529277664718c88fdae78db35bc) | final | mobile/tooling selection | success | 1 | 46m39s | [mobile](https://github.com/pennel-app/pennel/actions/runs/41205150940) |
| [`9a185308a0`](https://github.com/pennel-app/pennel/commit/9a185308a052e94edaf66b5777dfe8e9e35438a0) | final | docs only (job audit) | success | 1 | 0m19s | [ci](https://github.com/pennel-app/pennel/actions/runs/41126163968) |

Five latest-attempt-2 run records are [reviewed insights evaluation](https://github.com/pennel-app/pennel/actions/runs/41694004953), [question scheduling evaluation](https://github.com/pennel-app/pennel/actions/runs/41692718358), [question scheduling mobile](https://github.com/pennel-app/pennel/actions/runs/41677400222), [live speakers desktop](https://github.com/pennel-app/pennel/actions/runs/41677132798), and [meeting workspace ci](https://github.com/pennel-app/pennel/actions/runs/41486736240). These must remain distinct from first-attempt baselines.

## What the later decisions need

Latency targets should distinguish affected scope and schedule-inclusive elapsed time from physical execution. A design aimed only at setup seconds cannot remove the measured 45–59-minute mobile native work or the observed hour-long dependency-satisfied scheduling intervals. The evidence makes bounded questions concrete: preserving all configuration/test coverage while reusing or safely overlapping compilation; addressing packaging work on Windows; auditing individual broad selector inputs; and measuring scheduler admission/ready time well enough to avoid calling dependency waits “runner queue.” These are decision inputs, not a selected architecture or authorized CI implementation.

No p95, savings guarantee, matched warm/cold claim, native-cache adoption, branch-protection guarantee, release/device acceptance or paid-runner recommendation is established by this report.
