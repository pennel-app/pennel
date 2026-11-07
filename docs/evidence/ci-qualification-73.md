# CI scheduling qualification — 5 November 2026

## Implemented design; qualification held, serial retained

[Draft implementation: isolated mobile validation on free runners](https://github.com/pennel-app/pennel/pull/155) retains the serial default. Functional source `9a59cea0181b096fc4a45b564891a4304eeab064` includes the test-fixture correction described below. The subsequent assessor guard prevents pooling different frozen sources. No default rollout or merge has occurred.

- [x] Two isolated mobile VMs: sequential Release simulator/device builds, and independent Debug runtime/keyboard preparation and tests.
- [x] Complete mobile result verifies selected jobs, phases, source, workflow run and attempt; missing/failed/cancelled work cannot pass.
- [x] One frontend production build per desktop OS in candidate mode, with backend compilation checks after packaging and inspection/uploads after all checks.
- [x] Existing Rust cache family and successful-main producer retained; Debug restores only. Scoped cold bypasses configured Rust/npm caches without deleting them.
- [x] Serial default/fallback, legacy manual compiler-cache contract, affected-platform selectors and desktop release serialization retained.
- [ ] Five comparable complete candidate/control pairs per category and separate cold controls pass the adopted timing gates.
- [ ] Enable ordinary candidate scheduling only after that qualification passes.

### Result and measurement boundary

All **33 final physical benchmark cohorts passed their checks**. Qualification is nevertheless **not passed under exact runner-image-build matching**. Docs has one comparable warm pair and one comparable cold pair (five required); Desktop, Mobile and Core have zero comparable pairs. No category is qualified. Input mismatches remain reported; the proposed [rolling-image policy](https://github.com/pennel-app/pennel/issues/73#issuecomment-6645503178) has not been adopted.

Times below measure **replay-request-to-last-physical-job-complete**, including request fanout, waiting and orchestration. The original push-to-dispatch interval is unknown; these are not proved push-to-complete times. Every rerun used a fresh saved request boundary and full applicable workflows, not failed-only retries. Overlapping execution is never summed as cohort elapsed time. Exact ready-runner boundaries are unavailable, so request-to-start includes orchestration and dependency waiting as well as scheduling.

### Raw descriptive timings — not qualified medians

| Category / source / dependency state | Candidate count; median (seconds) | Serial count; median (seconds) | Candidate range | Individual candidate target misses |
| --- | ---: | ---: | ---: | ---: |
| docs / f9-original / warm | 5; 24.617 | 5; 22.082 | 20.985–818.699 | 1 |
| docs / f9-original / cold | 5; 22.710 | 5; 24.168 | 22.302–42.846 | 0 |
| desktop / f9-original / warm | 3; 947.121 | 4; 940.749 | 909.571–956.924 | 0 |
| mobile / f9-original / warm | 1; 2394.533 | 2; 2944.260 | 2394.533–2394.533 | 0 |
| core / f9-original / warm | 1; 2383.763 | 1; 3410.133 | 2383.763–2383.763 | 0 |
| core / core-python-pinned / warm | 0; — | 1; 2980.257 | — | — |

The 818.699025-second Docs candidate remains an individual 5-minute target miss. Its final desktop selection job started 810.699025 seconds after the saved request and executed for 8 seconds. The API fanout/operator reconciliation delay is included, not subtracted or replaced. Cold runs have a separate relative-regression gate, not an adopted absolute completion ceiling.

For the Mobile candidate, Release started at +50.532654s and executed for 2332s; Debug started at +49.532654s and executed for 1691s. The final result started at +2385.532654s and executed for 9s; complete elapsed is 2394.532654s. This preserves waiting/execution detail without adding overlapping jobs. Per-job intervals and source/image/toolchain/cache identities for every cohort remain in the retained capture files.

### Frozen sources and why comparisons were rejected

| Source wave | PR and category | Frozen base | Frozen head |
| --- | --- | --- | --- |
| f9-original | [Documentation](https://github.com/pennel-app/pennel/pull/158) | `f9b6a3e5d3233f9900bd0d6218bc0128073c880a` | `5f60a9e4aaf843dd6da5eeab6ee7e9abc8f51302` |
| f9-original | [Desktop](https://github.com/pennel-app/pennel/pull/159) | same | `c4b5c5122aebb319c6bc80240953978ca9ce8648` |
| f9-original | [Mobile](https://github.com/pennel-app/pennel/pull/160) | same | `2fcf9f20fa7c1b749a04e2da2290140be16201c2` |
| f9-original | [Original Core](https://github.com/pennel-app/pennel/pull/161) | same | `b449649773861ebe49dbba560abe69b448a8bbb5` |
| core-python-pinned | [Pinned Core](https://github.com/pennel-app/pennel/pull/162) | `56242632309fc8bfb2688625e7d41a84c6006003` | `79d0ee1c2997d6f1c9ca7c5fe482ee255dc4dfe6` |

Each benchmark diff is a semantic no-op confined to its intended category. Benchmark PRs remain drafts and must not merge. Selection/status VMs received different Linux image builds even within a single cohort. Desktop also received different Windows image builds. The first Mobile pair matched native Mac/Xcode/SDK/Rust/dependency identities but differed on Linux helper image builds; exact complete-cohort identity therefore rejects it. Original Core also changed Linux Python from 3.11.18 to 3.11.17.

The separate Core source wave pins observed Python to Linux 3.11.18 and Mac/Windows 3.11.9 while retaining every evaluation check. Its first control observed those versions; it is not pooled with old-source observations. Both original Core captures remain byte-for-byte unchanged. The immutable benchmark refs predate the final test-fixture correction and do not establish current-production performance acceptance.

### Every final physical observation

All rows below have complete valid successful check/receipt evidence. Exact comparability is a separate gate. Run links specify the archived physical attempt; later attempts must never substitute for earlier captures.

| Cohort | Elapsed seconds | Individual target miss | Complete workflow attempts |
| --- | ---: | --- | --- |
| `docs-warm-serial-01` | 39.959324 | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/1) |
| `docs-warm-candidate-01` | 24.617430 | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/2) |
| `docs-warm-serial-02` | 21.075093 | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/3) |
| `docs-warm-candidate-02` | 24.647622 | no | [ci a4](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/4) |
| `docs-warm-serial-03` | 21.386119 | no | [ci a5](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/5) |
| `docs-warm-candidate-03` | 21.401353 | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/6) |
| `docs-warm-serial-04` | 22.081832 | no | [ci a7](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/7) |
| `docs-warm-candidate-04` | 818.699025 | YES | [ci a8](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/8); [desktop a8](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/8) |
| `docs-warm-serial-05` | 26.157352 | no | [ci a9](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/9); [desktop a9](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/9) |
| `docs-warm-candidate-05` | 20.984705 | no | [ci a10](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/10); [desktop a10](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/10) |
| `docs-cold-serial-01` | 22.186172 | no | [desktop a11](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/11); [ci a11](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/11) |
| `docs-cold-candidate-01` | 22.709833 | no | [desktop a12](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/12); [ci a12](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/12) |
| `docs-cold-serial-02` | 24.495098 | no | [desktop a13](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/13); [ci a13](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/13) |
| `docs-cold-candidate-02` | 22.429264 | no | [desktop a14](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/14); [ci a14](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/14) |
| `docs-cold-serial-03` | 24.167925 | no | [desktop a15](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/15); [ci a15](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/15) |
| `docs-cold-candidate-03` | 29.715616 | no | [ci a16](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/16); [desktop a16](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/16) |
| `docs-cold-serial-04` | 34.384079 | no | [desktop a17](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/17); [ci a17](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/17) |
| `docs-cold-candidate-04` | 22.301699 | no | [ci a18](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/18); [desktop a18](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/18) |
| `docs-cold-serial-05` | 23.117858 | no | [desktop a19](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/19); [ci a19](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/19) |
| `docs-cold-candidate-05` | 42.845771 | no | [ci a20](https://github.com/pennel-app/pennel/actions/runs/41927590629/attempts/20); [desktop a20](https://github.com/pennel-app/pennel/actions/runs/41927590754/attempts/20) |
| `desktop-warm-serial-01` | 1009.058538 | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/1) |
| `desktop-warm-candidate-01` | 947.120597 | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/2) |
| `desktop-warm-serial-02` | 991.073617 | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/3) |
| `desktop-warm-candidate-02` | 956.923672 | no | [ci a4](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/4); [desktop a4](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/4) |
| `desktop-warm-serial-03` | 890.424802 | no | [ci a5](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/5); [desktop a5](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/5) |
| `desktop-warm-candidate-03` | 909.571373 | no | [ci a6](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/6); [desktop a6](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/6) |
| `desktop-warm-serial-04` | 841.999141 | no | [ci a7](https://github.com/pennel-app/pennel/actions/runs/41929053477/attempts/7); [desktop a7](https://github.com/pennel-app/pennel/actions/runs/41929053533/attempts/7) |
| `mobile-warm-serial-01` | 3220.216195 | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/1) |
| `mobile-warm-candidate-01` | 2394.532654 | no | [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/2); [ci a2](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/2) |
| `mobile-warm-serial-02` | 2668.304505 | no | [ci a3](https://github.com/pennel-app/pennel/actions/runs/41931515911/attempts/3); [desktop a3](https://github.com/pennel-app/pennel/actions/runs/41931516175/attempts/3); [mobile a3](https://github.com/pennel-app/pennel/actions/runs/41931516246/attempts/3) |
| `core-warm-serial-01` | 3410.132955 | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41933782904/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/41933782800/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41933782776/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/41933782733/attempts/1) |
| `core-warm-candidate-01` | 2383.763170 | no | [ci a2](https://github.com/pennel-app/pennel/actions/runs/41933782904/attempts/2); [desktop a2](https://github.com/pennel-app/pennel/actions/runs/41933782776/attempts/2); [mobile a2](https://github.com/pennel-app/pennel/actions/runs/41933782733/attempts/2); [decision-evaluation a2](https://github.com/pennel-app/pennel/actions/runs/41933782800/attempts/2) |
| `corepy-warm-serial-01` | 2980.256575 | no | [ci a1](https://github.com/pennel-app/pennel/actions/runs/41951206735/attempts/1); [decision-evaluation a1](https://github.com/pennel-app/pennel/actions/runs/41951206784/attempts/1); [desktop a1](https://github.com/pennel-app/pennel/actions/runs/41951206736/attempts/1); [mobile a1](https://github.com/pennel-app/pennel/actions/runs/41951206739/attempts/1) |

### Functional validation and failures kept separate

At functional implementation source `9a59cea`, the CI-helper suite passes **129 tests**; actionlint (ShellCheck disabled) and whitespace checks pass. The following assessor audit found that internally matched pairs at different frozen sources could be pooled toward one five-pair gate. That future false-pass path is corrected separately, with a regression, without changing any raw capture or today’s held result. The assessor reports independent source strata; a mixed-source input cannot qualify through pooled counts or medians. Additional valid image-mismatched pairs remain reported as noncomparable, rather than becoming integrity failures. A later qualification must use its prospectively declared frozen wave and retain the separate all-outcome report. Full hosted functional validation at `9a59cea` passed [core/Mac platform](https://github.com/pennel-app/pennel/actions/runs/41953104606), [desktop on all three OSes](https://github.com/pennel-app/pennel/actions/runs/41953104355), [decision evaluation](https://github.com/pennel-app/pennel/actions/runs/41953104520) and [mobile including the complete result](https://github.com/pennel-app/pennel/actions/runs/41953104511). These functional checks are separate from the immutable benchmark cohorts.

A real Windows failure in [the previous ordinary source](https://github.com/pennel-app/pennel/actions/runs/41949341798/job/126989356682) exposed a session test-fixture race: optional nonblocking speaker capture may drop contended packets, but the fixture assumed acceptance. A controlled test-only queue hold reproduced the exact missing-voice assertion. Commit `9a59cea` synchronizes accepted PCM and real observer acknowledgements in the test module, retaining the foreground-deferral, voice/provisional, row/text/window and progress assertions. The following poisoned-lock failure was cascading. No runtime behavior, timeout, ignore or retry was added. Exact regression, full lean library (1049 passed / 0 failed / 5 existing ignored), strict Clippy and formatting pass; the [new hosted Windows core job](https://github.com/pennel-app/pennel/actions/runs/41953104355/job/127001532791) passes both formerly failing named tests and its full 1047-test default suite (6 existing ignored).

Earlier unacquired Mac runner failures happened before checkout and remain recorded as provider/scheduling failures, outside the 33 passing benchmark observations. Observer API/log-read failures were recovered by rereading the same immutable jobs. Their error snapshots and bounded read-recovery traces remain retained; successful checks were not rerun to manufacture benchmark results.

At the `9a59cea` functional checkpoint, the implementation branch had 20 ordinary workflow runs: 14 successful, two failed and four cancelled. The failures were [Mac runner acquisition at `1062030`](https://github.com/pennel-app/pennel/actions/runs/41924885528) and the Windows fixture failure linked above. The initial `799aff2` [core](https://github.com/pennel-app/pennel/actions/runs/41924152768), [desktop](https://github.com/pennel-app/pennel/actions/runs/41924152784), [mobile](https://github.com/pennel-app/pennel/actions/runs/41924152786) and [evaluation](https://github.com/pennel-app/pennel/actions/runs/41924152930) runs were superseded by the next implementation push under the existing per-ref policy. These ordinary iterations are separate from the prospectively declared benchmark cohorts. They are not fresh successful qualification observations.

After the source-stratum guard, all **132 CI helper tests** pass, including three regressions for distinct frozen sources, selecting a faster source after observing timings, and retaining a noncomparable extra pair. Read-only reassessment of all 33 captures preserves every prior qualification flag, matched/rejected pair, count and median. The additional diagnostics expose separate source populations; raw captures and historical reports remain unchanged.

### Retained state and next acceptance

All 15 owned benchmark workflow IDs are terminal. Admissions are held and the controller is stopped; no benchmark job was cancelled and no request clock was reset. The original 80-entry manifest and transition snapshot remain retained. Eighteen never-requested old Core slots were explicitly superseded by 20 pinned-Core slots: 100 raw declarations, 82 effective physical observations, 33 requested/completed and 49 not requested.

Under the exact-image policy, at least 73 more observations would be needed (106 total; 24 beyond the effective 82 plan), **even if every future pair matched and passed**. This is only a lower bound. The remaining declared heavier runs cannot repair the exhausted Docs matching prerequisite, so no further cohorts were admitted. Additional mismatches could increase the requirement. All comparable times, including misses and slower candidates, must remain in medians.

After verifying the exact owned lease and terminal jobs, restored only `PENNEL_CI_QUALIFICATION_CONFIG` to its original absence, verified by HTTP404 on GET. `PENNEL_CI_FORCE_SERIAL`, all other variables, caches, branches and evidence were untouched. Ordinary validation remains serial; native CAS stays off. The legacy native-CAS timing/storage gates and issue history remain independent and unmet. No paid capacity, merge, release, signing/credential change, device acceptance, p95/maximum or dispatch deadline is claimed.

Local retained evidence is under `build/ci-qualification/`: held-strict-assessment.json, held-source-stratified-assessment.json, held-all-outcomes.json, held-handoff.json, per-cohort captures, immutable manifests, observer recovery snapshots and qualification-variable-cleanup-proof.json. 89 artifact ZIPs were checksum-verified; 348 cohort files were hashed before the handoff. Do not delete logs/results/symbols or replace raw observations. This issue stays open/In review while merge and qualification remain outstanding.
