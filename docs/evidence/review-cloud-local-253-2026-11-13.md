# Managed Review processing — 13 November 2026

Owner: [#253](https://github.com/pennel-app/pennel/issues/253). Model qualification remains tracked separately in [#96](https://github.com/pennel-app/pennel/issues/96).

## Implemented boundaries

Settings offers Pennel Cloud, On this computer and the existing advanced provider setup in English and Dutch. Choosing a source or opening Review performs no inference. Explicit Save changes only the Review assignment, with account/provider/configuration race checks; it does not enable automatic analysis or alter text/speech assignments.

Cloud uses the existing signed-in device session and a separately qualified `pennel-review-v3` / `supported_passages_v3` profile. Availability requires both current service metadata and a matching adopted qualification record. The exact destination is Pennel Cloud → OpenRouter → Perplexity. The existing direct OpenRouter C7 record remains independent; it cannot approve the Cloud route.

The managed local implementation uses the retained Decider Flash v1.2 Q8_0 artifact, with explicit acquisition/cancellation/removal and no Cloud fallback. The same open model file is hashed and supplied to the native loader. Actual serving identity binds its loaded artifact and pinned runtime/protocol implementation. The packaged runtime targets Apple Silicon Macs with macOS 14.4 or later; other desktop builds show the same option with an unsupported-build explanation. Mobile has no new local Review UI or runtime capability.

Model calls run in an owned worker with bounded requests, cancellation, deadlines and stop/reap ownership. An independently reviewed fix makes queued cancellation respect the waiting request's deadline and makes cold-start unload reap a child before returning. Three controlled-worker regressions cover these races.

## Local candidate result

The retained local candidate **failed development checks and remains unavailable**. No local qualification record was created and no new model was downloaded.

Three seen-development 16-window jobs using independent questions each proposed 14 items: 12 correct, two unsupported and three expected items missed. Whole native-worker jobs took 59.21, 61.28 and 59.62 seconds; startup took 7.49 seconds. These are development observations, not held-out measurements or a p95 claim. They do not satisfy the unchanged quality or 5-second local latency gates.

The measured work included 40,776 input tokens, 128 independent question sequences and 16 native decode calls per job. Actual loaded-buffer metadata showed Metal allocations; no evidence established a safe settings-only route to passing. Failed runs and original evaluation deadlines are retained. The fresh managed held-out corpus was not used for local inference.

## Qualification tooling

The Cloud and local harnesses call the shipped Rust Engine and shared decoder. They freeze actual request templates, source anchors, configuration, model identity expectations and compiled source fingerprints. Both retain the original quality, exact-source, calibration and resource gates. Bounded nonblocking JSONL supervision covers stalled input, partial replies and expired deadlines. Failures do not retry or switch providers.

The new corpus contains 290 invented Dutch fixtures with separate development, calibration and held-out families. Actual Cloud exports cover 288 prompt/slot/question contexts; the smallest calibration context has 11 observations, two classes and 11 families. Short explicit calibration/held-out selections accommodate access-token leases; a selection never claims full-phase completion, and final fitting/scoring still requires every frozen fixture.

Access-only evaluation leases travel through owned FIFOs and anonymous pipes, remain in memory, and cannot refresh or fall back to normal credentials in a child client. Production capability/identity preflight made zero provider inference requests. Service deployment and genuine Cloud qualification are recorded separately from this source/build evidence.

## Raw Cloud candidate: failed, retained

The first Cloud policy completed 433 real provider requests: two development, 180 calibration, 181 held-out and 70 resource requests. The pinned provider reported `perplexity/pplx-decider-v1.2-20261016` / `Perplexity`; its weights are not independently verified. All requests used invented text. Normal monthly quota stayed unchanged through the separate, explicitly approved test-device allowance.

The raw policy failed the unchanged zero-unsupported gate: 1,021 correct items from 1,027 proposals against 1,044 expected items, six unsupported, 23 missed and zero duplicates. An independent replay and source audit found semantic kind errors, with no gold, UTF-8, source-map, owner or due-field defect. No Cloud approval record was emitted or adopted. The failed result and original measured source remain retained; the passing direct OpenRouter C7 record is independent.

Five cold and thirty warm client measurements completed. Warm whole-job p95 was 6,593.97ms, measured individual-process peak RSS 74,989,568 bytes and cumulative CPU 21,844.768ms. Provider CPU/RSS and server warmth were unmeasured. The maximum-512-window development inference was not run after this quality failure. Total cumulative exposure, including historical conservative bounds, was US$0.34262418 with no new unresolved charges.

## Distinct calibrated-support policy

The next policy applies the original precommitted support temperature and cutoff parameters as a reject-only Rust gate. It preserves raw states, choices, probabilities and model-input wires; it never promotes a raw absent/unclear answer. Rejected or uncertain support becomes unassessed coverage. Missing or mismatched calibration, route, identity or context cannot fall back to raw support. The direct C7 path remains unchanged.

The immutable asset is `assets/qualifications/meeting-support-cloud-v1.json`, bound to the original fitted lock at `0d4dcfa69c07b84769729e4dee97c42a83fe7436`. Reuse verifies all 96 original calibration jobs, 180 actual request/response pairs, exact original fit and every input context. It retains the original run-manifest hash; reuse counts as zero new inference. The typed-result audit reproduced 8,832 rows with zero changed probability values. Numerical, typed-JSON, route-binding and foreign-cohort registry regressions passed.

This policy has a separate frozen protocol and 72 fresh held-out fixtures across nine new topic families, authored without reading earlier outputs/errors. The old held-out set supplies development diagnostics only. Every original quality threshold, source requirement, deadline and resource-sample count remains unchanged. New real held-out and resource results are required before adoption; the first live development check returned 14 correct items, one withheld and no unsupported item.

The new policy **passed every qualification gate**. It emitted 715 items against 776 expected: all 715 exactly matched the frozen judgments and literal sources, with zero unsupported or duplicate items and 61 misses. Raw/calibrated component macro F1 was 0.922168/0.919726; raw/calibrated support macro F1 was 0.996591/0.953107. These retain the original thresholds and count abstentions as coverage loss.

New inference comprised two development, 135 fresh held-out and 70 fresh resource requests. The original 180 calibration calls remain explicitly reused evidence, not new inference. Five fresh and thirty retained-coordinator measurements gave whole-job warm p95 5,549.26ms, individual-process peak RSS 92,684,288 bytes and cumulative CPU 33,873.718ms. Native GUI and provider CPU/RSS remain unmeasured. The maximum-512-window stress inference was not repeated within the bounded temporary test allowance; offline size/deadline refusal checks do not establish that larger performance result.

The exact emitted record is `finished-nl-bbf47b3e584713bc29e0`, bound to the actual calibration asset, production destination and measured Mac resource cohort. Its scope is manual finished Dutch analysis. Natural-ASR, human pilot, other resource cohorts and automatic foreground/background processing remain unapproved. No local-model record was added; the original direct C7 record is preserved.

## Verification before route qualification

- Core release library build and default tests: 1,373 passing, eight explicit ignores; keyboard: 17 passing.
- Core evaluation tests: 31 passing; CLI checks: three passing; direct C7 admission preserved.
- Evaluation Python suite: 402 passing, including the shared I/O regressions.
- Desktop fixture checks: 538 frontend and 426 backend tests, TypeScript/Vite, theme checks and strict backend Clippy passed at the implementation boundary.
- Mobile typecheck, 17 plugin, seven widget, 160 UI and 116 MobileKit tests passed; Mac platform tests: 241 passing.
- Owned mocked setup fixtures include matched English/Dutch light/dark before/after images, 340px enlarged-type geometry and advanced-provider regressions. Mock readiness is not service or model acceptance.

The new-policy default Core suite passed 1,381 tests (eight explicit ignores); its evaluation Python suite passed 433 tests, and the desktop backend passed 426 tests using the existing matching native SDK. Final signed package checks, regenerated final mobile libraries/builds and merge evidence remain separate. Existing physical-device, signed-upgrade, keyboard, automatic-analysis and broader #96 acceptance gates remain open.
