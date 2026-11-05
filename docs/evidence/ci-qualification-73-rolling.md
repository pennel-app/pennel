# Rolling-build CI qualification — 5 November 2026

Qualification is in progress. Ordinary validation remains serial. All current
checks stay before merge on standard free hosted runners; routine native CAS
remains off.

## Approved comparison rule

Lander explicitly [approved rolling runner image build numbers](https://github.com/pennel-app/pennel/issues/152#issuecomment-6664508244)
on 5 November 2026. The comparison may omit only observed `image_version`
values. Raw versions must still be present and retained. Image family and
architecture, exact frozen source/base/head/merge, configuration, observed
Rust/Node/npm/Python/Xcode/SDK identities and dependency/cache state continue
to match. The previous [exact-image checkpoint](ci-qualification-73.md) and
raw captures remain unchanged.

The initial objectives remain warm candidate medians of at most 5 minutes for
documentation, 20 minutes for desktop/frontend and 60 minutes for mobile or
shared core. Each category needs at least five prospectively selected complete
matched candidate/control pairs. Separate matched cold medians may regress by
no more than 10%. Every observation, delay, failure, cancellation, retry and
individual miss stays reported. Goals do not shorten validation timeouts or
establish p95, maximum or guaranteed dispatch deadlines.

Times are replay-request-to-last-physical-job-complete, including API fanout,
operator reconciliation, orchestration and waiting. Original push-to-dispatch
intervals are unknown. Per-job request-to-start and execution intervals remain
separate; overlapping jobs are never summed as cohort elapsed time. Rolling
fleet results establish no same-image causal speedup.

## Prospective source scope

The original Docs, Desktop and Mobile frozen sources remain eligible for
reassessment and their unrequested cases. Core uses a separate corrected source:

| Category | Frozen base | Frozen head | Source wave |
| --- | --- | --- | --- |
| Docs | `f9b6a3e5d3233f9900bd0d6218bc0128073c880a` | `5f60a9e4aaf843dd6da5eeab6ee7e9abc8f51302` | f9-original |
| Desktop | same | `c4b5c5122aebb319c6bc80240953978ca9ce8648` | f9-original |
| Mobile | same | `2fcf9f20fa7c1b749a04e2da2290140be16201c2` | f9-original |
| Core | `6f3dd25783f23c0cb709ad8830db7dd8755e0b1c` | `40b9b1e19fdfb68b1db4d3b0fb650f135e03b419` | core-python-fixture-fixed |

The corrected Core base adds only the already-proven accepted-audio fixture
correction to the Python-pinned frozen recipe. Non-test session bytes and the
evaluation recipe match the implementation; the original semantic-no-op Core
probe is byte-identical. This removes the known Windows fixture race from the
remaining runs while preserving all assertions and product behavior.

The two original Core observations and one Python-only Core control remain in
the all-outcome audit as earlier source waves. Their exclusion from the active
qualification roster is prospective and based on input/correctness changes,
not observed timings. Different source populations cannot be pooled or chosen
by which is faster. The new Core wave declares five complete pairs in each
cache state before requesting any of them.

The active roster comprises 80 physical observations: 20 per category. The
all-outcome audit is expected to retain at least 83, including the three earlier
Core observations. Original manifests remain immutable; only never-requested
Core slots are superseded, with preserved transition snapshots. The remaining
50 observations are not replacements for misses or slower candidates.

## Results and retained evidence

The first bounded resume completed both cohorts successfully: corrected Core
serial control 01 took 2978.532322s; Mobile candidate 02 took 2533.910243s and
matched its existing serial control's source, toolchain and cache identity.
All selected jobs and required native phases passed without a workflow retry.
There are now 35 retained outcomes, 32 in the active roster, and 48 remaining
requests. The first-35 readback verified 204 unique physical jobs, 101 artifact
ZIPs and 384 retained files; all 348 previously hashed cohort files are unchanged.

| Category / cache | Matched pairs | Candidate matched median | Current result |
| --- | ---: | ---: | --- |
| Docs / warm | 5 | 24.617430s | passes 5-minute objective |
| Docs / cold | 5 | 22.709833s | passes relative gate against 24.167925s control |
| Desktop / warm | 3 | 947.120597s | fewer than five pairs |
| Desktop / cold | 0 | N/A | pending |
| Mobile / warm | 2 | 2464.2214485s | fewer than five pairs |
| Mobile / cold | 0 | N/A | pending |
| Core / warm | 0 | N/A | one control, no paired treatment yet |
| Core / cold | 0 | N/A | pending |

The original Docs candidate miss of 818.699025 seconds remains in its
five-observation warm population. Cold has no adopted absolute ceiling, so
derived cold `target_miss` labels are null/N/A. A reporting-only correction was
made at the controlled stop: reassessment of all 35 raw captures preserved every
gate, validity, count, median and clock, changing only ten cold labels. All 25
warm labels, including the Docs miss, remain unchanged. The corrected collector
passes 141 CI-helper tests; historical raw captures and assessments stay intact.

Rust cache compatibility is independently substantiated by three distinct
physical jobs (serial 126929415640, Release 126961003751, Debug 126961003664),
each with an exact restore of
`v0-rust-expo-mobile-lean-ios-generated-build-Darwin-arm64-fc985acc-9f6ae85c`.
The relevant-key-only API snapshot returned one archive across all refs: main
ID 9809973475, 564852135 bytes, version
`0011543faff0431349ce5ac946a048c3d801fd47a480e65f7ce7947eba0973f3`.
Both lanes have identical configured shared-key/workspace/default paths;
Release saves only on successful main pushes and Debug is restore-only. The
unique current archive association plus exact hits establishes lookup-version
compatibility. Additional bounded prefix reads returned zero bytes on timeout;
no job-resolved action SHA or runtime path printout was inferred. This cache
proof is separate from performance qualification. All three new native receipts
also confirm exact warm Rust hits, no bypass and the unchanged shared key.

The implementation has separately integrated main `8ad573f` (desktop refinements,
speech recovery and the exact Ubuntu-mirror fix), preserving its new checks and
unrelated local configuration. Benchmark refs remain frozen; final hosted
validation must cover the combined implementation source.

Old read-error snapshots, physical run/attempt identities, raw image versions,
cache audit and artifact checksums remain under `build/ci-qualification/`.

The scoped qualification variable is leased only to an exact benchmark PR/head.
Its original absent state will be restored after owned runs finish. No working
cache or evidence is deleted, no paid capacity is added, and the legacy manual
native-CAS contract and its stricter gates remain unchanged.
