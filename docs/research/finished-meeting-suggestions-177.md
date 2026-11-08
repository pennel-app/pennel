# Existing routes for finished-meeting Suggestions

Research resolution for [Which existing model routes can support trustworthy
finished-meeting suggestions?](https://github.com/pennel-app/pennel/issues/177), under
[Make finished meetings dependable and actionable](https://github.com/pennel-app/pennel/issues/176).
Source: Pennel
[`a005b8e0d39161aea69d2a30b5638078eef2a46d`](https://github.com/pennel-app/pennel/commit/a005b8e0d39161aea69d2a30b5638078eef2a46d).
External documentation and public metadata checked **8 November 2026**.

**No route is ready for adoption.** The useful shortlist is the official local
Decider runtime, pinned hosted standard Decider, and direct Mercury Decide. These
are candidates for separately authorized experiments, not a route choice or
activation. The [bundled registry][registry] contains zero adopted records. This
research made no inference calls, downloads, purchases, account changes or provider
changes. Mac, manually requested analysis after Stop, and `en`, `nl`, `mixed` are
the scope; each language needs its own evidence. Trustworthy selective coverage is
useful, with omissions explained and unsupported owners/dates unknown.

## What the app actually requests

The [production request][insights] is `meeting_suggestions_v1`, grouping
`independent-event-passages-spans-typed-prior-with-neighbours-v3`. It independently
classifies **actions, decisions, open questions and changed commitments**, with
tentative, confirmed, retracted, unresolved, changed and unknown states, plus
none/negated outcomes. Restricting activation to two kinds would change this
contract, its prompts/schema and qualification; it is not an existing free toggle.

Each finalized utterance is SOURCE 0, with its immediate previous/next utterances
as context. More than 512 windows is refused. Typed notes, captions and inferred
speaker identity are not extraction inputs. Only protocol `mercury` adds
`event_spans` (24 kind/state fields) and `details` (owner, verbatim due phrase and
typed prior references). Other protocols produce passage-level suggestions,
without separately extracted owners/dates/prior evidence. Unsupported or ambiguous
details remain unknown. Exact quoting prevents invented strings; it does **not**
prove correct event interpretation. Distant retractions outside this three-utterance
window are a concrete coverage risk. [Decoding and provenance][decode]

The [transport][transport] speaks typed `state`/`questions`, not chat completions.
The default serialized-input ceiling is 16,000 bytes; LocalDecider additionally
caps the body at 32,768, and the general maximum is 65,536. These include question
overhead and are not tokenizer limits. Oversized input is refused, never truncated.
The [finished job][deadline] has one absolute configured deadline (maximum 90
seconds), so fast individual requests do not establish acceptable whole-meeting
completion. A generic OpenAI-compatible generator is not automatically compatible.

## Compatibility and bounded shortlist

| Candidate | Existing Pennel call path | Capability and present gap |
| --- | --- | --- |
| **Local first:** official Decider for Mac, `pplx-decider-flash-v1.2` | `local_decider`; Custom account; `http://127.0.0.1:8080/v1/decisions`; no key | Documented typed choices; current app sends four questions. Vendor Mac examples specify one question/request: verify unchanged grouping before assuming compatibility. Passage scope; no native exact-span/detail request. No Pennel inference or resource measurements for this runtime. |
| **Hosted comparison:** `perplexity/pplx-decider-v1.2`, upstream `perplexity` | `open_router_decisions`; `https://openrouter.ai/api/alpha/decisions` | Existing typed choice path; passage scope. Historical expected response was `perplexity/pplx-decider-v1.2-20261016`, not proof of today's identity. Revalidate exact model/revision/provider, policy and complete-item quality. |
| **Extraction comparison:** direct `mercury-decide` | `mercury`; Custom account; `https://api.inceptionlabs.ai/v1/decisions` | Existing choice-plus-extract path matches current documented request shape. Exact event/detail spans are possible, but production six-question behavior, identity, semantic quality and resources are unmeasured. Historical OpenRouter Mercury results are a different route. |

Call-path compatibility here means an implemented transport can form the request
after explicit configuration; it does not mean the runtime is installed, a service
was called, or Suggestions can bypass admission. [Protocol capabilities][protocols]

Perplexity documents [Decider for Mac][mac] for Apple Silicon M1+, macOS 13+,
16 GB memory and about 6 GB free disk; first launch downloads about 5.8 GB. Inference
needs no API key or usage billing, but setup may request a Hugging Face read token:
fully account-free acquisition is not established. These are **vendor requirements**,
not measured minimums. Its exact installed runtime version, precision,
weight/tokenizer hashes, memory, load time and local context capacity remain missing.

The [Flash weights card][flash] marks Apache-2.0 and describes a specialized
noncausal, per-question readout. Its 262,144-position limit and approximately 32k
training length describe the reference model, not a measured Mac-app context budget.
The official [GGUF card][gguf] explicitly says stock `llama-server` does not implement
its protocol v3; its 13-question Metal reference comparison is vendor evidence,
not Pennel parity or meeting quality. Do not replace the official runtime with
Ollama, MLX or an arbitrary GGUF server by assumption. Runtime redistribution
licensing has not been reviewed.

The [hosted Decider documentation][pplx-api] describes typed choices and fixed
grouping effects; changing grouping can change probabilities. A versioned model
name can also move when retired, and direct API aliases may merely be echoed in
responses. A model string is insufficient identity evidence. Public
[OpenRouter endpoint metadata][decider-endpoints], read at approximately 12:28 UTC,
returned `perplexity` and `makora/fp16`; pinning the former preserves the historical
upstream rather than silently switching. This GET establishes neither retention
nor inference success. Existing router requests deny data collection, require ZDR
and disable fallbacks; account logging and processing region still require review.
[Saved routing rules][desktop-connections]

Inception currently [documents extract][mercury-extract] on the direct Decisions
endpoint, mixed with other question types, with 1–26 fields, exact spans, offsets
and `not_stated`. This supports a contract experiment, not a semantic guarantee.
Its [direct model table][mercury-models] leaves Mercury Decide's context window
unspecified; do not transfer OpenRouter's model-page context to this route. The
[OpenRouter Mercury endpoints GET][mercury-endpoints] returned HTTP 200 with an
empty endpoint list today. Historical availability therefore cannot justify
selecting that router route now. Direct retention, region, service limits and
revision metadata remain unverified; no hosted price or latency estimate is used.

Pennel Cloud's `pennel-auto` is an existing authenticated managed decision path,
but it cannot select the direct Mercury extraction protocol and has no adopted
record here. Its actual backend/deployment qualification is separate; selecting
Cloud cannot inherit another route's identity. [Managed configuration][protocols]

## What was actually measured

The [28 October local run][local-results] measured CPU FP32 workers on an M4 Pro,
48 GiB, macOS 27.0.1. GLiClass event-kind F1 was 0.057 EN / 0.070 NL / 0.089 mixed;
GLiNER was also below 0.1. GLiClass warm event p95 was 57.6/62.7/64.9 ms and peak
owned RSS 1.539 GiB: speed passed that panel while quality failed. These were
optional Python workers, not a packaged Pennel decision server. The [worker
contract][workers] also leaves GLiNER without complete probability distributions,
GTE ranking-only, and Q8 GLiClass without reference-parity qualification. They are
not stronger immediate Suggestions candidates.

The [hosted run][hosted-results] contains real typed API predictions, separate
calibration and preserved failures. Bounded event-kind point passes did not
establish complete extraction: owner calibration had nine observations and one
observed class per context. The [repaired supplement][followup] measured standard
Decider owner component F1/precision of 1.0 in all three languages, on 12 authored
meeting groups per cell; Mercury's calibrated owner F1 was 0.8485 EN / 0.8413 NL /
0.9302 mixed. These are different prompts/tasks/routes, not current Suggestions
qualification. The separate GPT-4.1-mini summary experiment promoted tentative or
unassigned statements into actions; it cannot substitute for typed extraction.

The [incremental qualification replay][incremental-evidence] is authored fixture evidence: 30 meeting
families, 1,440 prefixes, 4,500 cases, zero adopted records. Its ASR perturbations
are text authored by the application, not natural recorded speech. Crucially,
[`incremental.py`][incremental-code] exports its own three component questions and
v1 schema, while production uses the four-kind v3 request above. Reuse its split,
calibration and scoring infrastructure, but first bind an export/runner to the
**actual production request**; relabelling existing predictions cannot bridge this
prompt/schema/grouping gap.

## Evidence required before adoption

The [existing protocol][qualification-protocol] and [Rust admission gate][qualification]
require:

1. Frozen calibration before held-out inference, split by meeting family; separate
   EN/NL/mixed contexts; at least ten calibration observations, two classes, two
   meeting groups and nonempty held-out evidence. Preserve genuine distributions,
   abstentions, refusals and service failures.
2. Component **and complete-item** macro-F1 ≥0.90, accepted-item precision ≥0.95,
   supported owners/final status, exact source tuples, no duplicate/unsupported
   proposals, and preserved user review state. Measure missed events and unknown
   details; a classifier label alone earns no complete-item pass. Deadline
   normalization remains a separate unmeasured stage.
3. Actual recorded inference; complete source judgments; five cold and thirty warm
   samples, executed-request counts, CPU/RSS and warm p95. Local event p95 must be
   ≤5,000 ms; Rust bounds hosted p95 by the configured timeout. Record whole-job
   latency and cancellation on Mac too. Source assessment's separate 0.98
   precision/0.50 coverage and 750-ms local gates do not qualify Suggestions.
4. Exact protocol/endpoint/model/expected-model/upstream/input-limit/timeout,
   actual model/revision/provider/artifact hash, task, prompt/schema/ordered-label
   hashes, grouping, preprocessing, calibration and OS/resource profile. Preserve
   the [production fingerprints][fingerprints]. Runtime [identity validation][identity]
   rechecks returned model/revision/provider; the [transport][identity-parser] reads
   revision only from `model_sha`/`model_revision`. The cited direct response examples do
   not establish all those fields: missing identity must remain a blocker, never
   be filled from a requested alias.
5. Reviewed application-owned adoption, then packaged Mac and natural recorded-ASR
   usefulness/recovery acceptance. Finished analysis does not require the distinct
   foreground/background live gates; it also does not satisfy them.

Three next questions are sharp enough for bounded follow-up tickets:

- **Local Flash:** can the official runtime accept the unchanged four-question
  production request, expose sufficient verifiable identity, and satisfy all-kind
  tuple quality within Mac resource/deadline bounds? Single-question documentation
  is a test question, not an observed incompatibility.
- **Hosted standard Decider:** does passage fallback pass complete-item gates
  separately for EN/NL/mixed, including multi-event passages and distant
  retractions, while visibly leaving unsupported details unknown?
- **Direct Mercury:** can the six-question request (24 event-span and six detail
  fields) correctly separate
  coexisting events, Unicode spans, tentative/retracted commitments and ambiguous
  owners/due phrases, with qualifying identity and selected-route privacy evidence?

Route selection, exact measured failure/coverage bounds and candidate-specific
acquisition/spend limits still need Lander's decision. Historical 3-GiB/US$2
experiment allowances are not renewed authorization. Output implementation stays
with [Add reviewable meeting actions, decisions and open questions with transcript
sources](https://github.com/pennel-app/pennel/issues/57), qualification with
[Qualify decision models for incremental meeting assistance](https://github.com/pennel-app/pennel/issues/96),
and setup/recovery with [Guide Notetaker setup and recovery](https://github.com/pennel-app/pennel/issues/115).
Existing capture and UX acceptance owners remain responsible for their evidence.

[registry]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/assets/qualifications/meeting-assistance.json
[insights]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/meeting_insight_engine.rs#L23-L130
[decode]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/meeting_insight_engine.rs#L230-L538
[transport]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/inference.rs#L762-L858
[deadline]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/meeting_insight_engine.rs#L1126-L1129
[protocols]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/inference.rs#L446-L655
[mac]: https://docs.perplexity.ai/docs/decisions/decider-for-mac
[flash]: https://huggingface.co/perplexity-ai/pplx-decider-flash-v1.2
[gguf]: https://huggingface.co/perplexity-ai/pplx-decider-flash-gguf-v1.2
[pplx-api]: https://docs.perplexity.ai/docs/decisions/quickstart
[decider-endpoints]: https://openrouter.ai/api/v1/models/perplexity/pplx-decider-v1.2/endpoints
[desktop-connections]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/docs/desktop.md#L186-L213
[mercury-extract]: https://docs.inceptionlabs.ai/capabilities/extract
[mercury-models]: https://docs.inceptionlabs.ai/get-started/models
[mercury-endpoints]: https://openrouter.ai/api/v1/models/inception/mercury-decide/endpoints
[local-results]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/evaluations/decision-models/results/2026-10-28/README.md
[workers]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/evaluations/decision-models/workers/README.md#L29-L37
[hosted-results]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/evaluations/decision-models/results/openrouter-2026-10-28/README.md
[followup]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/evaluations/decision-models/results/followup-2026-10-28/README.md
[incremental-evidence]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/docs/evidence/incremental-qualification-96.md
[incremental-code]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/evaluations/decision-models/incremental.py#L84-L109
[qualification-protocol]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/evaluations/decision-models/protocol-incremental-v1.json#L61-L94
[qualification]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/meeting_qualification.rs#L225-L299
[fingerprints]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/meeting_insight_engine.rs#L571-L613
[identity]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/meeting_qualification.rs#L372-L387
[identity-parser]: https://github.com/pennel-app/pennel/blob/a005b8e0d39161aea69d2a30b5638078eef2a46d/core/src/inference.rs#L1404-L1469
