# CI qualification — 5–7 November 2026

**Qualified under the approved rolling-build, per-stratum protocol.** All eight
gates have five matched complete passing control/candidate pairs. The active
80 observations pass; the complete 112-outcome history retains110 passing and
two failed observations. Neither historical failure counts positively.

The [approved rule](https://github.com/pennel-app/pennel/issues/152#issuecomment-6664508244)
omits only observed image build numbers from comparison. Raw versions remain
required and retained. Source/base/head/merge, configuration, image family and
architecture, observed toolchains, cache identity/state and complete validation
coverage still match. The independent exact-image shadow remains unqualified.
This is completion-objective qualification on the rolling free fleet, not a
same-image causal speedup, p95, maximum or guaranteed dispatch deadline.

## Complete matched results

| Category | Warm candidate median | Warm objective | Cold candidate/control median | Cold ratio /limit |
| --- | ---: | ---: | ---: | ---: |
| docs | 25s | 5m00s | 23s / 24s | 0.939668 /1.10 |
| desktop | 15m29s | 20m00s | 29m59s / 30m01s | 0.998791 /1.10 |
| mobile | 36m25s | 60m00s | 43m43s / 53m47s | 0.812761 /1.10 |
| core | 35m17s | 60m00s | 36m19s / 51m27s | 0.705952 /1.10 |

Cold has no adopted absolute completion ceiling; its absolute `target_miss`
label is null/N/A. Its unchanged gate is median(candidate)<=1.10*median(control),
from the same five complete matched pairs. All values remain included, including
slower candidates and misses. Every current applicable check remains before
merge on standard free hosted runners; routine native compilation caching is off.

## Clock and source scope

The metric is **replay-request-to-last-physical-job-complete**. Original
push-to-dispatch intervals are unknown. Request fanout, delivery reconciliation,
network transfers/backoff, orchestration, dependency waits and scheduling count.
Per-job request-to-start and execution intervals are retained separately; the
exact ready-runner boundary may be unknown. Overlapping jobs are never summed
as cohort elapsed time. The recorded wall-clock continuity gap has an unknown
cause and no duration is deducted.

This is **composite per category/cache stratum**. Counts and medians never pool
source versions within a gate. It is not five warm and five cold pairs on one
corrected current head:

| Category/cache | Frozen base | Frozen head | Benchmark PR |
| --- | --- | --- | --- |
| Docs warm/cold | `f9b6a3e5d3233f9900bd0d6218bc0128073c880a` | `5f60a9e4aaf843dd6da5eeab6ee7e9abc8f51302` |158 |
| Mobile warm/cold | same | `2fcf9f20fa7c1b749a04e2da2290140be16201c2` |160 |
| Desktop warm/cold | `20a8e6115af3ffc4d8c345b62ddd0e26794dea2c` | `832de1d1f3c59d9c2b10139d55752fdda4b186b0` |170 |
| Core warm | same | `cc2e4f1dfe53a101f5ca8ea5982e2282daea5dac` |171 |
| Core cold | `9bf4e016f66a797e9f734066871fe593e8c0f38d` | `aac713f4cf2aeba630dc71428ee9cdb4fd7655b6` |173 |

Every head changes only its original two-line semantic-no-op category probe.
All old refs remain immutable and benchmark PRs must never merge. The product
continues developing independently: current main and both independent fixture
synchronizations are integrated for final implementation validation, which is
separate from this frozen benchmark qualification.

## Every matched pair

### docs / warm

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `docs-warm-01` | 39.959324 | 24.617430 |
| `docs-warm-02` | 21.075093 | 24.647622 |
| `docs-warm-03` | 21.386119 | 21.401353 |
| `docs-warm-04` | 22.081832 | 818.699025 |
| `docs-warm-05` | 26.157352 | 20.984705 |

### docs / cold

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `docs-cold-01` | 22.186172 | 22.709833 |
| `docs-cold-02` | 24.495098 | 22.429264 |
| `docs-cold-03` | 24.167925 | 29.715616 |
| `docs-cold-04` | 34.384079 | 22.301699 |
| `docs-cold-05` | 23.117858 | 42.845771 |

### desktop / warm

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `mirror-desktop-warm-01` | 1116.652361 | 979.966444 |
| `mirror-desktop-warm-02` | 976.460728 | 928.714727 |
| `mirror-desktop-warm-03` | 757.235431 | 954.120765 |
| `mirror-desktop-warm-04` | 989.466640 | 838.502183 |
| `mirror-desktop-warm-05` | 978.044824 | 664.851164 |

### desktop / cold

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `mirror-desktop-cold-01` | 1414.611887 | 1825.824392 |
| `mirror-desktop-cold-02` | 1486.945891 | 1798.749094 |
| `mirror-desktop-cold-03` | 2184.123245 | 1173.828527 |
| `mirror-desktop-cold-04` | 1800.926722 | 1875.016485 |
| `mirror-desktop-cold-05` | 2064.553490 | 1577.078522 |

### mobile / warm

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `mobile-warm-01` | 3220.216195 | 2394.532654 |
| `mobile-warm-02` | 2668.304505 | 2533.910243 |
| `mobile-warm-03` | 3499.302723 | 2184.514467 |
| `mobile-warm-04` | 3101.863284 | 2156.570748 |
| `mobile-warm-05` | 4011.224239 | 2007.937551 |

### mobile / cold

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `mobile-cold-01` | 3290.677873 | 1942.852698 |
| `mobile-cold-02` | 2567.010599 | 2622.808330 |
| `mobile-cold-03` | 3227.036568 | 2504.341466 |
| `mobile-cold-04` | 2776.524228 | 3384.182410 |
| `mobile-cold-05` | 3572.917243 | 2990.863960 |

### core / warm

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `mirror-core-warm-01` | 2849.236943 | 1743.061636 |
| `mirror-core-warm-02` | 2869.365389 | 3136.818032 |
| `mirror-core-warm-03` | 3169.123027 | 1840.131058 |
| `mirror-core-warm-04` | 2705.510168 | 2117.164220 |
| `mirror-core-warm-05` | 2646.042759 | 2155.244680 |

### core / cold

| Pair | Control seconds | Candidate seconds |
| --- | ---: | ---: |
| `network-core-cold-01` | 3375.162797 | 2300.237969 |
| `network-core-cold-02` | 3086.826123 | 2198.780227 |
| `network-core-cold-03` | 2695.747397 | 2017.466276 |
| `network-core-cold-04` | 3353.760403 | 2179.150425 |
| `network-core-cold-05` | 2804.171767 | 1992.435003 |

## Failures, corrections and retained difficult cases

- `mirror-core-cold-serial-04` remains failed. Its model-free library first
  failed the foreground-deferral fixture's pending-PCM checkpoint; nine lock
  poison errors cascaded. A separate Desktop Mac app job was cancelled before
  hosted runner acquisition. Its image/toolchain/log remain unknown; values
  from another attempt are never borrowed. Forced queue contention reproduced
  the test failure with one legally dropped optional packet. A test-only
  rendezvous then proved accepted PCM, retained context and zero native
  admission while deferred, preserving all original resume assertions. The 40ms
  sleep was removed. Production bytes were unchanged; 1071 default/1054 lean
  tests and strict Clippy in both profiles passed.
- `checkpoint-core-cold-candidate-05` remains failed. An acquired Release
  runner exhausted Cargo's existing network retries resolving static.crates.io
  for quote1.0.47. Its preparation identity/Release phases and aggregate remain
  missing/failed. Its1705.094984s raw clock is not a passing performance sample.
  The entire old cold population—not only this failed case—stays history.
- The actual provisioning correction adds only two preparation-step
  `CARGO_NET_RETRY: '6'` settings after cache lookup. [Cargo's bounded transport
  configuration](https://doc.rust-lang.org/cargo/reference/config.html#netretry)
  does not retry builds, tests or whole workflows. All commands, checks,
  timeouts, cache actions/inputs and job/global environments remain unchanged.
  One preparation invocation per native job is retained, and all transfer
  waiting/backoff counts. Full logs record the observed warnings/attempts;
  zero observed warning lines is not a claim of zero network attempts.
- The original Docs candidate818.699025s miss, successful Desktop3350.462631s
  control/miss and Mobile4011.224239s control/miss remain recorded. The old
  Desktop control spent prolonged time on HTTP Azure-mirror package fetches
  before HTTPS fallback; it passed naturally. New Desktop/Core populations used
  the already-reviewed signed HTTPS mirror repair with all packages/checks.
- Slower matched candidates are not replaced. Desktop cold's initial ratios
  exceeded1.10; its full declared five-pair median passes at0.998791. Two Mobile
  cold candidates were slower than their paired controls and remain included.
  No fastest-five selection or timing-based extra repetition was used.
- Observer failures remain separate from physical executions. Missing image
  evidence and the212-byte unrecognized log response were recovered only from
  the same immutable jobs, preserving known source/job/artifact/time fields.
  The original212-byte body was not retained, so its cause remains unknown.
  Bad snapshots, actual successful GET responses and equality proofs remain.
- Three unconfirmed API rerun deliveries were resolved through saved intents,
  exact-source/attempt/lease readbacks and one explicitly controlled redelivery
  each. Only actual fresh full workflow attempts count. Original clocks include
  every delivery gap; persistent late-effect guards found no extra attempt.

Both negative-source archives remain immutable. Corrective-source manifests
are tied to actual failures, direct red/green evidence and byte-level patches;
unknown evidence, duplicate jobs, source mismatch or artifact corruption cannot
be waived by selecting a new source. Historical failures remain failed in every
audit, while the explicit active 80 source roster alone supplies the gate.

## Cache compatibility and independent audit

Serial, candidate Release and candidate Debug physical jobs independently
recorded exact restores of the existing key
`v0-rust-expo-mobile-lean-ios-generated-build-Darwin-arm64-fc985acc-9f6ae85c`.
The relevant-key-only API found a unique main archive ID9809973475,564852135
bytes, version
`0011543faff0431349ce5ac946a048c3d801fd47a480e65f7ce7947eba0973f3`.
Configured shared-key/workspace/default paths are identical; Release remains
main's single successful writer and Debug restore-only. Exact hits plus the
unique archive associate a compatible lookup version; no job-resolved action
SHA or runtime path printout was inferred from failed additional reads.

Independent read-only audit verified 112 capture hashes,486 ZIP hashes/CRCs,
330 actual mode receipts and 1143 unique physical jobs, with no reused IDs.
Every qualifying pair differs only in the approved image build version. All 40
active native observations retain complete coverage, and both corrective
manifests verify. The strict shadow preserves exact-image rejection and raw
versions. Old warm misses, every slower pair, failures, unknowns, transport and
recovery context remain retained.

## All 112 physical outcomes

The [machine report](ci-qualification-73-rolling.json) preserves source/merge,
request clocks, physical workflow attempts, raw observed images, per-job
intervals, errors and capture hashes for every row. `History` is an explicit
predeclared source transition, not a validity or speed filter. Failed historical
rows remain nonpassing. Source rows never substitute carried-forward successful
siblings or failed-only reruns for a fresh full cohort.

| Cohort | Scope | Seconds | Passing checks | Warm miss | Complete workflow attempts |
| --- | --- | ---: | --- | --- | --- |
| `docs-warm-serial-01` | Active | 39.959324 | yes | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/1) |
| `desktop-warm-serial-01` | History | 1009.058538 | yes | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/1) |
| `docs-warm-candidate-01` | Active | 24.617430 | yes | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/2) |
| `mobile-warm-serial-01` | Active | 3220.216195 | yes | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/1) |
| `desktop-warm-candidate-01` | History | 947.120597 | yes | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/2) |
| `docs-warm-serial-02` | Active | 21.075093 | yes | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/3) |
| `docs-warm-candidate-02` | Active | 24.647622 | yes | no | [ci a4](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/4) |
| `docs-warm-serial-03` | Active | 21.386119 | yes | no | [ci a5](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/5) |
| `docs-warm-candidate-03` | Active | 21.401353 | yes | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/6) |
| `core-warm-serial-01` | History | 3410.132955 | yes | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41933782904/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/41933782800/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41933782776/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/41933782733/attempts/1) |
| `docs-warm-serial-04` | Active | 22.081832 | yes | no | [ci a7](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/7) |
| `docs-warm-candidate-04` | Active | 818.699025 | yes | YES | [ci a8](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/8); [desktop a8](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/8) |
| `docs-warm-serial-05` | Active | 26.157352 | yes | no | [ci a9](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/9) |
| `docs-warm-candidate-05` | Active | 20.984705 | yes | no | [ci a10](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/10); [desktop a10](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/10) |
| `docs-cold-serial-01` | Active | 22.186172 | yes | N/A | [desktop a11](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/11); [ci a11](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/11) |
| `docs-cold-candidate-01` | Active | 22.709833 | yes | N/A | [desktop a12](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/12); [ci a12](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/12) |
| `docs-cold-serial-02` | Active | 24.495098 | yes | N/A | [desktop a13](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/13); [ci a13](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/13) |
| `desktop-warm-serial-02` | History | 991.073617 | yes | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/3) |
| `docs-cold-candidate-02` | Active | 22.429264 | yes | N/A | [desktop a14](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/14); [ci a14](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/14) |
| `docs-cold-serial-03` | Active | 24.167925 | yes | N/A | [desktop a15](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/15); [ci a15](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/15) |
| `docs-cold-candidate-03` | Active | 29.715616 | yes | N/A | [ci a16](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/16); [desktop a16](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/16) |
| `docs-cold-serial-04` | Active | 34.384079 | yes | N/A | [desktop a17](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/17); [ci a17](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/17) |
| `docs-cold-candidate-04` | Active | 22.301699 | yes | N/A | [ci a18](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/18); [desktop a18](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/18) |
| `docs-cold-serial-05` | Active | 23.117858 | yes | N/A | [desktop a19](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/19); [ci a19](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/19) |
| `docs-cold-candidate-05` | Active | 42.845771 | yes | N/A | [ci a20](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/20); [desktop a20](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/20) |
| `mobile-warm-candidate-01` | Active | 2394.532654 | yes | no | [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/2); [ci a2](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/2) |
| `desktop-warm-candidate-02` | History | 956.923672 | yes | no | [ci a4](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/4) |
| `core-warm-candidate-01` | History | 2383.763170 | yes | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/41933782904/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41933782776/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/41933782733/attempts/2); [decision-evaluation a2](https://github.com/pennel-app/pennel/actions/runs/41933782800/attempts/2) |
| `desktop-warm-serial-03` | History | 890.424802 | yes | no | [ci a5](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/5) |
| `mobile-warm-serial-02` | Active | 2668.304505 | yes | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/3); [mobile a3](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/3) |
| `desktop-warm-candidate-03` | History | 909.571373 | yes | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/6) |
| `desktop-warm-serial-04` | History | 841.999141 | yes | no | [ci a7](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/7) |
| `corepy-warm-serial-01` | History | 2980.256575 | yes | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41951206735/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/41951206784/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41951206736/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/41951206739/attempts/1) |
| `corefix-warm-serial-01` | History | 2978.532322 | yes | no | [desktop a1](https://github.com/pennel-app/pennel/actions/runs/42078173608/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/42078173394/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/42078173276/attempts/1); [ci a1](https://github.com/pennel-app/pennel/actions/runs/42078173920/attempts/1) |
| `mobile-warm-candidate-02` | Active | 2533.910243 | yes | no | [mobile a4](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/4); [ci a4](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/4) |
| `corefix-warm-candidate-01` | History | 2337.476662 | yes | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/42078173920/attempts/2); [decision-evaluation a2](https://github.com/pennel-app/pennel/actions/runs/42078173276/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/42078173608/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/42078173394/attempts/2) |
| `mobile-warm-serial-03` | Active | 3499.302723 | yes | no | [ci a5](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/5); [mobile a5](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/5) |
| `desktop-warm-candidate-04` | History | 969.850480 | yes | no | [desktop a8](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/8); [ci a8](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/8) |
| `desktop-warm-serial-05` | History | 3350.462631 | yes | YES | [ci a9](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/9) |
| `corefix-warm-serial-02` | History | 3187.759213 | yes | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/42078173920/attempts/3); [decision-evaluation a3](https://github.com/pennel-app/pennel/actions/runs/42078173276/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/42078173608/attempts/3); [mobile a3](https://github.com/pennel-app/pennel/actions/runs/42078173394/attempts/3) |
| `mirror-core-warm-serial-01` | Active | 2849.236943 | yes | no | [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/1); [ci a1](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/1) |
| `mirror-desktop-warm-serial-01` | Active | 1116.652361 | yes | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/1) |
| `mobile-warm-candidate-03` | Active | 2184.514467 | yes | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/6); [mobile a6](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/6) |
| `mirror-core-warm-candidate-01` | Active | 1743.061636 | yes | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/2); [decision-evaluation a2](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/2) |
| `mirror-desktop-warm-candidate-01` | Active | 979.966444 | yes | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/2) |
| `mirror-desktop-warm-serial-02` | Active | 976.460728 | yes | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/3) |
| `mirror-core-warm-serial-02` | Active | 2869.365389 | yes | no | [desktop a3](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/3); [decision-evaluation a3](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/3); [ci a3](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/3); [mobile a3](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/3) |
| `mirror-desktop-warm-candidate-02` | Active | 928.714727 | yes | no | [ci a4](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/4) |
| `mirror-desktop-warm-serial-03` | Active | 757.235431 | yes | no | [ci a5](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/5) |
| `mirror-desktop-warm-candidate-03` | Active | 954.120765 | yes | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/6) |
| `mirror-core-warm-candidate-02` | Active | 3136.818032 | yes | no | [ci a4](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/4); [decision-evaluation a4](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/4); [mobile a4](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/4) |
| `mobile-warm-serial-04` | Active | 3101.863284 | yes | no | [desktop a7](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/7); [ci a7](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/7); [mobile a7](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/7) |
| `mirror-core-warm-serial-03` | Active | 3169.123027 | yes | no | [desktop a5](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/5); [decision-evaluation a5](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/5); [ci a5](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/5); [mobile a5](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/5) |
| `mirror-desktop-warm-serial-04` | Active | 989.466640 | yes | no | [ci a7](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/7) |
| `mobile-warm-candidate-04` | Active | 2156.570748 | yes | no | [ci a8](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/8); [desktop a8](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/8); [mobile a8](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/8) |
| `mirror-core-warm-candidate-03` | Active | 1840.131058 | yes | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/6); [decision-evaluation a6](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/6); [mobile a6](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/6) |
| `mirror-desktop-warm-candidate-04` | Active | 838.502183 | yes | no | [desktop a8](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/8); [ci a8](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/8) |
| `mobile-warm-serial-05` | Active | 4011.224239 | yes | YES | [ci a9](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/9); [mobile a9](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/9) |
| `mirror-core-warm-serial-04` | Active | 2705.510168 | yes | no | [decision-evaluation a7](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/7); [ci a7](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/7); [mobile a7](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/7) |
| `mirror-core-warm-candidate-04` | Active | 2117.164220 | yes | no | [ci a8](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/8); [decision-evaluation a8](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/8); [desktop a8](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/8); [mobile a8](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/8) |
| `mirror-desktop-warm-serial-05` | Active | 978.044824 | yes | no | [ci a9](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/9) |
| `mobile-warm-candidate-05` | Active | 2007.937551 | yes | no | [ci a10](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/10); [desktop a10](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/10); [mobile a10](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/10) |
| `mirror-core-warm-serial-05` | Active | 2646.042759 | yes | no | [decision-evaluation a9](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/9); [ci a9](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/9); [mobile a9](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/9) |
| `mirror-desktop-warm-candidate-05` | Active | 664.851164 | yes | no | [ci a10](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/10); [desktop a10](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/10) |
| `mobile-cold-serial-01` | Active | 3290.677873 | yes | N/A | [ci a11](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/11); [desktop a11](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/11); [mobile a11](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/11) |
| `mirror-core-warm-candidate-05` | Active | 2155.244680 | yes | no | [desktop a10](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/10); [decision-evaluation a10](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/10); [ci a10](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/10); [mobile a10](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/10) |
| `mirror-core-cold-serial-01` | History | 2957.974616 | yes | N/A | [ci a11](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/11); [decision-evaluation a11](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/11); [desktop a11](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/11); [mobile a11](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/11) |
| `mirror-desktop-cold-serial-01` | Active | 1414.611887 | yes | N/A | [ci a11](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/11); [desktop a11](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/11) |
| `mobile-cold-candidate-01` | Active | 1942.852698 | yes | N/A | [mobile a12](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/12); [ci a12](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/12); [desktop a12](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/12) |
| `mirror-core-cold-candidate-01` | History | 2454.607235 | yes | N/A | [ci a12](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/12); [decision-evaluation a12](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/12); [desktop a12](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/12); [mobile a12](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/12) |
| `mirror-desktop-cold-candidate-01` | Active | 1825.824392 | yes | N/A | [ci a12](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/12); [desktop a12](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/12) |
| `mirror-core-cold-serial-02` | History | 2883.044423 | yes | N/A | [ci a13](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/13); [decision-evaluation a13](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/13); [desktop a13](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/13); [mobile a13](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/13) |
| `mobile-cold-serial-02` | Active | 2567.010599 | yes | N/A | [ci a13](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/13); [mobile a13](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/13); [desktop a13](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/13) |
| `mirror-desktop-cold-serial-02` | Active | 1486.945891 | yes | N/A | [ci a13](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/13); [desktop a13](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/13) |
| `mirror-core-cold-candidate-02` | History | 2486.180596 | yes | N/A | [ci a14](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/14); [mobile a14](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/14); [decision-evaluation a14](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/14); [desktop a14](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/14) |
| `mobile-cold-candidate-02` | Active | 2622.808330 | yes | N/A | [ci a14](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/14); [desktop a14](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/14); [mobile a14](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/14) |
| `mirror-core-cold-serial-03` | History | 3890.023900 | yes | N/A | [desktop a15](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/15); [decision-evaluation a15](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/15); [ci a15](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/15); [mobile a15](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/15) |
| `mirror-desktop-cold-candidate-02` | Active | 1798.749094 | yes | N/A | [ci a14](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/14); [desktop a14](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/14) |
| `mobile-cold-serial-03` | Active | 3227.036568 | yes | N/A | [ci a15](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/15); [desktop a15](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/15); [mobile a15](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/15) |
| `mirror-core-cold-candidate-03` | History | 2623.212438 | yes | N/A | [ci a16](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/16); [decision-evaluation a16](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/16); [desktop a16](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/16); [mobile a16](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/16) |
| `mirror-desktop-cold-serial-03` | Active | 2184.123245 | yes | N/A | [ci a15](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/15); [desktop a15](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/15) |
| `mirror-core-cold-serial-04` | History | 2755.957913 | FAILED | N/A | [decision-evaluation a17](https://github.com/pennel-app/pennel/actions/runs/42109970305/attempts/17); [ci a17](https://github.com/pennel-app/pennel/actions/runs/42109970554/attempts/17); [desktop a17](https://github.com/pennel-app/pennel/actions/runs/42109970331/attempts/17); [mobile a17](https://github.com/pennel-app/pennel/actions/runs/42109970439/attempts/17) |
| `checkpoint-core-cold-serial-01` | History | 3806.042581 | yes | N/A | [ci a1](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/1) |
| `mirror-desktop-cold-candidate-03` | Active | 1173.828527 | yes | N/A | [ci a16](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/16); [desktop a16](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/16) |
| `mobile-cold-candidate-03` | Active | 2504.341466 | yes | N/A | [desktop a16](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/16); [ci a16](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/16); [mobile a16](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/16) |
| `checkpoint-core-cold-candidate-01` | History | 2092.142302 | yes | N/A | [ci a2](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/2); [decision-evaluation a2](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/2) |
| `mirror-desktop-cold-serial-04` | Active | 1800.926722 | yes | N/A | [ci a17](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/17); [desktop a17](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/17) |
| `mobile-cold-serial-04` | Active | 2776.524228 | yes | N/A | [desktop a17](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/17); [ci a17](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/17); [mobile a17](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/17) |
| `checkpoint-core-cold-serial-02` | History | 3458.735794 | yes | N/A | [decision-evaluation a3](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/3); [ci a3](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/3); [mobile a3](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/3) |
| `mirror-desktop-cold-candidate-04` | Active | 1875.016485 | yes | N/A | [desktop a18](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/18); [ci a18](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/18) |
| `mirror-desktop-cold-serial-05` | Active | 2064.553490 | yes | N/A | [ci a19](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/19); [desktop a19](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/19) |
| `mirror-desktop-cold-candidate-05` | Active | 1577.078522 | yes | N/A | [ci a20](https://github.com/pennel-app/pennel/actions/runs/42110067655/attempts/20); [desktop a20](https://github.com/pennel-app/pennel/actions/runs/42110067645/attempts/20) |
| `checkpoint-core-cold-candidate-02` | History | 2918.856249 | yes | N/A | [mobile a4](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/4); [decision-evaluation a4](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/4); [ci a4](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/4) |
| `mobile-cold-candidate-04` | Active | 3384.182410 | yes | N/A | [ci a18](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/18); [desktop a18](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/18); [mobile a18](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/18) |
| `checkpoint-core-cold-serial-03` | History | 3991.502438 | yes | N/A | [ci a5](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/5); [decision-evaluation a5](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/5); [mobile a5](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/5) |
| `mobile-cold-serial-05` | Active | 3572.917243 | yes | N/A | [ci a19](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/19); [desktop a19](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/19); [mobile a19](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/19) |
| `checkpoint-core-cold-candidate-03` | History | 2745.389579 | yes | N/A | [decision-evaluation a6](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/6); [ci a6](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/6); [mobile a6](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/6) |
| `mobile-cold-candidate-05` | Active | 2990.863960 | yes | N/A | [desktop a20](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/20); [ci a20](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/20); [mobile a20](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/20) |
| `checkpoint-core-cold-serial-04` | History | 3441.545026 | yes | N/A | [decision-evaluation a7](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/7); [ci a7](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/7); [mobile a7](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/7) |
| `checkpoint-core-cold-candidate-04` | History | 2413.859555 | yes | N/A | [decision-evaluation a8](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/8); [ci a8](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/8); [desktop a8](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/8); [mobile a8](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/8) |
| `checkpoint-core-cold-serial-05` | History | 3168.815428 | yes | N/A | [desktop a9](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/9); [mobile a9](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/9); [decision-evaluation a9](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/9); [ci a9](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/9) |
| `checkpoint-core-cold-candidate-05` | History | 1705.094984 | FAILED | N/A | [decision-evaluation a10](https://github.com/pennel-app/pennel/actions/runs/42203793167/attempts/10); [ci a10](https://github.com/pennel-app/pennel/actions/runs/42203793095/attempts/10); [desktop a10](https://github.com/pennel-app/pennel/actions/runs/42203793275/attempts/10); [mobile a10](https://github.com/pennel-app/pennel/actions/runs/42203792971/attempts/10) |
| `network-core-cold-serial-01` | Active | 3375.162797 | yes | N/A | [mobile a1](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/1); [ci a1](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/1) |
| `network-core-cold-candidate-01` | Active | 2300.237969 | yes | N/A | [decision-evaluation a2](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/2); [ci a2](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/2) |
| `network-core-cold-serial-02` | Active | 3086.826123 | yes | N/A | [ci a3](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/3); [mobile a3](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/3); [decision-evaluation a3](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/3) |
| `network-core-cold-candidate-02` | Active | 2198.780227 | yes | N/A | [decision-evaluation a4](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/4); [ci a4](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/4); [mobile a4](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/4) |
| `network-core-cold-serial-03` | Active | 2695.747397 | yes | N/A | [decision-evaluation a5](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/5); [ci a5](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/5); [mobile a5](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/5) |
| `network-core-cold-candidate-03` | Active | 2017.466276 | yes | N/A | [desktop a6](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/6); [decision-evaluation a6](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/6); [ci a6](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/6); [mobile a6](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/6) |
| `network-core-cold-serial-04` | Active | 3353.760403 | yes | N/A | [decision-evaluation a7](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/7); [ci a7](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/7); [mobile a7](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/7) |
| `network-core-cold-candidate-04` | Active | 2179.150425 | yes | N/A | [ci a8](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/8); [decision-evaluation a8](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/8); [desktop a8](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/8); [mobile a8](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/8) |
| `network-core-cold-serial-05` | Active | 2804.171767 | yes | N/A | [decision-evaluation a9](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/9); [ci a9](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/9); [mobile a9](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/9) |
| `network-core-cold-candidate-05` | Active | 1992.435003 | yes | N/A | [ci a10](https://github.com/pennel-app/pennel/actions/runs/42307848187/attempts/10); [decision-evaluation a10](https://github.com/pennel-app/pennel/actions/runs/42307848265/attempts/10); [desktop a10](https://github.com/pennel-app/pennel/actions/runs/42307848135/attempts/10); [mobile a10](https://github.com/pennel-app/pennel/actions/runs/42307848134/attempts/10) |

## Reproducibility and rollout boundary

Raw captures, typed receipt ZIPs, source/probe byte proofs, immutable manifests,
mode/lease readbacks, all eight raw five-pair tables, exact-image shadow,
controller budget/order/stop proofs and complete native transport diagnostics
remain under `build/ci-qualification/`. The declared102 campaign's immutable tar
SHA256 is
`5e77b5f542ec52aefa42f8ecccdf4d804d24a0bb3574c27fe44b3eb9c6674f62`.
The final 112 archive/cleanup packet is retained at
`build/ci-qualification/final-112-archive-20261107T062904Z/`. Its tar SHA256 is
`8972532b1b236563b99d9d56fb62deaae22f555e1fc4cfb564bf676a48dffb89`;
file-index SHA256 is
`35d63e3eb6b6612f1fa6d4f227ab346a8c949fa9886880a1ed88ad8d2a9ccfbd`.
The separate cleanup proof SHA256 is
`80fdfb5d8fa9362e0e45339c7e3bdcf08b74175391a3788a2751c552094fa462`. Do not delete
source refs, evidence, products or required symbols through generic cleanup.

All owned benchmark processes and task-scoped idle assertions stop after final
capture. Only the exact owned qualification variable is restored to its original
absence, with fresh lease and404 readback; force-serial and unrelated settings,
secrets and caches are untouched.

This qualification permits the implementation's guarded ordinary candidate
default for same-repository PRs and exact main pushes. Manual/privileged/fork
safeguards remain serial; `PENNEL_CI_FORCE_SERIAL=true` and exact scoped serial
controls retain the fallback. No new variable or native cache family is needed.
Publication/merge and current combined-source functional CI remain distinct;
this report does not claim that the implementation has merged or shipped.
