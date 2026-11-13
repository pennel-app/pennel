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

## Verification before route qualification

- Core release library build and default tests: 1,373 passing, eight explicit ignores; keyboard: 17 passing.
- Core evaluation tests: 31 passing; CLI checks: three passing; direct C7 admission preserved.
- Evaluation Python suite: 402 passing, including the shared I/O regressions.
- Desktop fixture checks: 538 frontend and 426 backend tests, TypeScript/Vite, theme checks and strict backend Clippy passed at the implementation boundary.
- Mobile typecheck, 17 plugin, seven widget, 160 UI and 116 MobileKit tests passed; Mac platform tests: 241 passing.
- Owned mocked setup fixtures include matched English/Dutch light/dark before/after images, 340px enlarged-type geometry and advanced-provider regressions. Mock readiness is not service or model acceptance.

Final route qualification, registry adoption, final signed package checks, regenerated mobile libraries/builds and merge evidence must be added before claiming those results. Existing physical-device, signed-upgrade, keyboard, automatic-analysis and broader #96 acceptance gates remain separate.
