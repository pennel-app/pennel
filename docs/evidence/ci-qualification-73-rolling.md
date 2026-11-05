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

Reassessment and remaining complete cohorts are being collected. The original
Docs candidate miss of 818.699025 seconds remains in its five-observation warm
population. Old read-error snapshots, physical run/attempt identities, raw image
versions and artifact checksums remain retained under `build/ci-qualification/`.

The scoped qualification variable is leased only to an exact benchmark PR/head.
Its original absent state will be restored after owned runs finish. No working
cache or evidence is deleted, no paid capacity is added, and the legacy manual
native-CAS contract and its stricter gates remain unchanged.
