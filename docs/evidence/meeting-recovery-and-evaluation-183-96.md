# Meeting recovery and production-bound evaluation

8 November 2026. Prepared implementation and local verification for [Expose failed-chunk transcription recovery after reopening a saved meeting](https://github.com/pennel-app/pennel/issues/183) and [Qualify decision models for incremental meeting assistance](https://github.com/pennel-app/pennel/issues/96), following the [agreed brief](../finished-meeting-first-work.md). Review and merge remain pending. The [machine receipt](meeting-recovery-and-evaluation-183-96.json) contains exact source/artifact hashes, counts, scope and limits.

## Implemented behavior

Saved meetings now carry a coherent failed-recovery plan with failed/retryable counts, availability and an owned reason. Detail reads share the execution selector's path/provenance/header rules without reading PCM. Explicit retry rechecks full audio integrity; an unsafe chunk stays retained while independent healthy chunks can recover. Provider failures still stop service work, preserving actionable guidance and committed progress.

The existing meeting document shows persistent incomplete-transcript guidance and **Retry failed transcription** in English/Dutch. It saves typed-note edits before retry, refreshes after zero/partial/error outcomes and preserves reading/draft/meeting ownership. **Retranscribe recording** remains distinct. Missing or unknown recovery availability disables the targeted action.

The feature-gated offline `pennel-eval` bridge exports and replays the shipped Suggestions windows, wire bodies, raw parser, identity comparison and candidate decoding. Authored fixtures and owned temporary stores are accepted; real profiles/stores, credentials, bridge networking and admission/registry mutation are excluded. The Python runner consumes the Rust exports unchanged, freezes context and split information, and enforces local-only transport, absolute deadlines and complete attempt retention. Historical results remain unchanged; raw component scoring is explicitly descriptive and uncalibrated.

Implementation commits are `0894ab8` (recovery), `adc7cb4` (evaluation) and `e23a17b` (test isolation). The latter changes only `cfg(test)` fixtures. The signed native Mac candidate was built at `adc7cb4`; its production source is equivalent to the verified code head `e23a17b`.

## Verification

| Check | Result |
| --- | --- |
| Core default | 1,086 passed; seven existing ignored |
| Core lean with evaluation | 1,087 passed; five existing ignored |
| Rust typing/keyboard core | 17 passed |
| Desktop frontend | 421 tests; TypeScript/Vite build passed |
| Desktop backend | 270 passed |
| Mac platform | 146 passed |
| Evaluation Python | 300 passed, including 13 finished-pilot regressions |
| Formatting, strict default/evaluation Clippy, theme and packaging | Passed |
| Owned headless rendering | Eight matched before/after images, four interaction cases, twelve unavailable/minimum-size renders; EN/NL and light/dark; no overflow at 760×520 |
| Mobile compatibility | Typecheck; 17 plugin, seven widget, 159 UI and 116 MobileKit tests; clean/repeated Prebuild, locked pods and six ownership checks passed |
| Generated iOS and native products | Fresh base/candidate and device/simulator binding bytes match; both unsigned ARM64 Release builds and 144 runtime/service + 36 keyboard native tests passed |

Mobile final-source preparation at `e23a17b` reproduced the tested compiled-library and binding bytes. Original manifests/products were preserved; harmless XCFramework entry ordering was compared by keyed entries. The fresh owned simulator is shut down and existing simulators are unchanged.

Regression-first checks reproduced the missing export, missing recovery status, corrupt-first-chunk ordering, absolute-deadline and evidence-retention failures before their fixes. Full parallel Rust runs also exposed transport/manual-cancellation fixture interference from unrelated speech work. Test-only foreground setup now reaches those fixtures' own gates; deterministic negative/manual-cancel controls retain coverage and production priorities remain unchanged. Failed runs and review audit records remain retained locally.

## Packaged Mac recovery

The personally signed **Pennel Capture Preview** passed strict/deep verification with team `B538Q2K76C`; binary SHA-256 is `c66a06cb1a0c8c9286b88ed50573e97ddf4a6dcdf57e80f299d82cac30712f55`.

Public core APIs created a new isolated profile from generated PCM and a keyless loopback speech fixture, with two failed/eligible passages and a separate missing-audio case. This establishes valid retained-audio provenance without microphone capture or invented metadata. Actual native UI actions showed the warning, saved an unsaved note before retry, preserved one recovered passage through a later HTTP 503, saved another note edit and retried exactly the remaining audio hash. A second relaunch preserved meeting-file hashes and plans without new recognition requests. The missing-audio action stayed disabled with an explanation. Eleven fixture requests were retained: eight during seeding and three from the two explicit UI retries.

This is packaged UI/core recovery evidence using synthetic ingestion/recognition, not patched real-microphone, ASR-quality or human-usability acceptance. The app and owned fixture service are stopped; profiles, audio and reversible backups stay local.

The dedicated capture identity separates data and credentials from ordinary Preview/production. Its WebKit/TCC/Keychain identity remains shared across capture runs; no existing capture storage was cleared. A final post-quit accessibility observation timed out after the app had exited; the owned PID was separately confirmed gone.

## Bounded local Decider outcome

The notarized official **Decider for Mac 0.1.0** was installed for the trial. Its default text-model GGUF was pinned to published revision `2514d5f0b7fbbe1ee1bfec9ffb5801a60289b4cc` and SHA-256 `ffb27ade8976765b0e6943a0b7f5f3fd0f7042d45ad90256a76d608bcd3942c2`. Text-only mode used that same official default artifact; no replacement model, question splitting, alias workaround or hosted fallback was used. Sources: [official runtime setup](https://docs.perplexity.ai/docs/decisions/decider-for-mac), [published model files](https://huggingface.co/perplexity-ai/pplx-decider-flash-gguf-v1.2/tree/2514d5f0b7fbbe1ee1bfec9ffb5801a60289b4cc).

The first probe was sent before model loading completed and returned HTTP 503, “Loading model.” That complete attempt/binary/freeze is preserved separately. Readiness was corrected to require the official **model-ready event**, rather than an open port. No payload or production contract was changed.

After loading, the unchanged four-question production request returned **HTTP 200** with all four answer IDs. Its JSON omitted the required model revision and provider identity. Production parsing therefore returned **identity_unavailable**, and the pilot stopped after the first loaded English window. Remaining Dutch/mixed fixtures were not attempted. Identity was not filled from an alias, local filename or artifact manifest.

Retained Decider/runtime/model artifacts total about **5.24 GB**, below **8,000,000,000 bytes**. Observed loading took 5.29 seconds; evaluation stopped about 2.14 seconds after the ready event was observed, within the 1,800-second cap. Hosted spend was **zero**. Scoped CPU/RSS observations include the owned vendor process lifetime and are not a cold/warm or aggregate application qualification. Both POST probes, including the premature 503, remain recorded. The owned runtime is stopped; model and evidence artifacts remain available locally.

The qualification registry is unchanged and empty. This confirms bounded wire acceptance while identifying a serving-identity blocker; it establishes no language-quality, calibrated, complete-item or production-adoption result. Full qualification, actual ASR/call, physical-device, signed-upgrade, accessibility, distribution and deployment acceptance remain with their existing owners.
