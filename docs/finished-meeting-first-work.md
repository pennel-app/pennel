# First work for dependable, actionable Mac meetings

Agreed with Lander on 8 November 2026 through [Which bounded changes should make the first Mac workflow ready?](https://github.com/pennel-app/pennel/issues/179). This is an implementation brief; it records no completed product fix, installation, inference or model adoption.

## Decision and order

1. Implement [Expose failed-chunk transcription recovery after reopening a saved meeting](https://github.com/pennel-app/pennel/issues/183).
2. In parallel, extend [Qualify decision models for incremental meeting assistance](https://github.com/pennel-app/pennel/issues/96) with an offline evaluation path bound to the actual production Suggestions contract. The recovery fix can ship independently.
3. After production/evaluation parity passes, run the bounded official local Decider compatibility pilot below. Keep Suggestions gated pending full qualification.

Mac is the first acceptance surface. Analysis is manual after Stop. Dutch, English and mixed meeting content require separate evidence; interface language stays independent. Labelled, source-linked transcript passages are useful initially, including passages containing several events. Unsupported owners/dates remain unknown, and incomplete coverage stays visible. Preserve all four existing suggestion kinds and states.

The [native baseline](https://github.com/pennel-app/pennel/blob/0a48cb7604561ff5217d9ee1dbfa4baf17731255/docs/evidence/mac-recording-recovery-178.md) demonstrated preserved work and remaining-only recovery with synthetic recognition, and reproduced the missing recovery UI. The [route research](https://github.com/pennel-app/pennel/blob/0da530a1550c55070a4006c231fbcda25c7c6f26/docs/research/finished-meeting-suggestions-177.md) identified the current request/benchmark mismatch and candidate compatibility/identity gaps. Neither establishes ASR quality or a qualified model.

## Recovery fix

- Show incomplete-transcript guidance in the existing meeting document and Transcript companion after reopening. Reuse Glaze controls; keep English/Dutch desktop parity and ordinary recording/Stop behavior.
- Add a read-only failed-recovery plan to existing meeting detail: failed count, retryable count and owned status/reason. Unknown counts remain unknown. Reuse the same snapshot, provenance and retained-audio eligibility selector as failed-only retry; full archive availability or a failed flag alone cannot establish retryability. Keep filesystem work off the native UI thread.
- Enable **Retry failed transcription** only for currently eligible retained audio. Explain missing audio, unavailable metadata and unverified provenance. Keep **Retranscribe recording** as a distinct explicit operation.
- Flush typed-note edits before explicit retry and preserve drafts if saving fails. Invoke the existing failed-only command; revalidate authoritatively at execution. Refresh meeting state after success, error or partial progress while preserving the user's document context.
- Interpret the refreshed state: zero recovered with failures remaining is not completion, and a positive count does not establish that every failure recovered. Preserve successful passages, typed notes, source identity and existing selected-provider behavior. No automatic retry or provider fallback.

This adds status metadata and UI around the existing recovery engine. It introduces no replacement recorder, retry engine or meeting-file migration.

## Production-bound evaluation path

Extend the existing feature-gated Rust evaluation bridge and `pennel-eval`, using shared production preparation, wire encoding, raw-response parsing, identity comparison and insight decoding. Python orchestrates frozen experiments and scoring; it must consume these production-derived requests/results instead of reconstructing the contract.

Export authored finalized snapshots with their exact source mappings and qualification context. Replay regenerates those mappings from the same fixture and imports raw response bytes, including identity. Keep the bridge offline and fixture-only: no real profile paths, credentials, networking, real/user meeting-store mutation, registry writes or injected qualification handles. Owned temporary fixture stores remain available for source preparation and parity checks. Production admission continues through the bundled registry.

Create a versioned finished-meeting experiment alongside historical incremental results. Freeze the actual prompt/schema/labels/grouping/preprocessing and configured/actual model/resource context. Preserve family-level development/calibration/held-out separation. Do not relabel old component scores as production evidence, fabricate served-weight identity, change the task to actions-only or lower existing gates to fit a candidate.

## Bounded local compatibility pilot

The first candidate is official **Decider for Mac**, `pplx-decider-flash-v1.2`, through existing `local_decider` at `http://127.0.0.1:8080/v1/decisions` in an isolated evaluation configuration. Standard app-root inspection found no identified installation; older cached Python-worker artifacts are a different runtime/model. Recheck installed state before acquisition.

The [official setup documentation](https://docs.perplexity.ai/docs/decisions/decider-for-mac) describes Apple Silicon/macOS 13+, 16 GB memory, about 6 GB disk and a roughly 5.8 GB first-launch model download. It directs callers to send one question per request; unchanged four-question compatibility remains untested. The inspected M4 Pro/48 GiB host meets the documented hardware allowance; this is not measured runtime qualification.

Agreed limits and exit conditions:

- At most **8 GB (8,000,000,000 bytes)** of new retained runtime/model data, including retained installer data; **30 minutes elapsed evaluation time after the runtime is ready**, including failed attempts, waits and cancellation cleanup. Record download/load time separately. Hosted spend is **zero**.
- Use only invented development fixtures in `en`, `nl` and `mixed`. Keep calibration/held-out families out of pilot tuning. Start with one unchanged production four-question request and required returned identity; proceed to short finished-meeting snapshots only if those checks pass.
- Freeze current input limits and explicitly use the existing default **30-second whole-job deadline** across serial windows for this pilot; 90 seconds is the supported configuration ceiling, not the current default. Include preparation and all physical requests; do not reset the deadline per window, increase it silently or parallelize windows to make a candidate pass. Record actual request counts, latency and scoped CPU/RSS; unavailable measurements remain unavailable.
- This evaluation pilot establishes wire/parser/identity compatibility and observed bounded completion only. It does not prove admitted production scheduling, full quality, long-meeting throughput, calibration or resource acceptance.
- Record a blocker and stop this pilot if the unchanged contract, required identity, acquisition or resource bounds cannot be satisfied. Preserve existing credentials/services; new account or credential setup is outside this brief. Do not silently split questions, change grouping, substitute a runtime/model, fall back to hosted inference or activate Suggestions. Stop only runtime/requests started for this pilot when it ends, retaining their bounded model/evidence artifacts.

A passing pilot leads to full exact-context qualification under its existing owner. Actual inference, frozen calibration before held-out execution, component/event macro-F1 ≥0.90, end-to-end complete-item macro-F1 ≥0.90, accepted precision ≥0.95, supported owner/final status, per-language results, five cold/thirty warm samples and complete resource/source evidence remain required. Preserve local warm event p95 ≤5 seconds and measure whole-job completion separately. A passage-scoped output may be useful without claiming complete extraction; that does not waive qualification gates. Natural recorded-ASR and packaged/device/human acceptance remain explicit separate evidence.

## Verification and handoff

- Recovery regressions cover reopened failures; missing/unreadable audio; invalid/unverified metadata; valid failed audio without a full archive; zero and partial progress; preserved notes/context; and no resubmission of successful chunks. Repeat the isolated personally signed Preview recovery check and distinguish stored-audio fixtures from any new real capture.
- Compare production mock-transport bodies byte-for-byte with evaluation exports and replay the same raw responses through both decoders. Cover all four kinds, Mercury's existing extraction shape, Unicode/source offsets, duplicate/missing answers, oversized inputs/responses, changed identity and changed freeze/source hashes. An evaluation build must still refuse ordinary unqualified admission and make no bridge credential/provider calls.
- Run affected core, desktop and Mac-native checks under the current macOS-first policy, plus affected mobile generation/native checks when shared core or bindings require them. Visual PRs include matched before/after light/dark evidence on `pr-assets`. Pin the verified personal Apple identity/team for Preview signing.
- Deliver focused recovery and evaluation changes with their existing issue owners. Keep [Add reviewable meeting actions, decisions and open questions with transcript sources](https://github.com/pennel-app/pennel/issues/57) open for its remaining output qualification/acceptance. Live/background assistance, hosted comparison, iPhone adaptation and releases remain separately scoped.
