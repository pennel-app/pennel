# Decision-model evaluation

Measured results: [initial local comparison](../evaluations/decision-models/results/2026-10-28/README.md)
and [initial OpenRouter comparison](../evaluations/decision-models/results/openrouter-2026-10-28/README.md).
The [completed follow-up](../evaluations/decision-models/results/followup-2026-10-28/README.md)
adds Mercury's remaining timing slots, actual generated-output evidence review and an
independent suggestion experiment with feasible coverage and diverse calibration targets.
The reports identify bounded, calibrated research candidates and retain measured failures.
Production behavior remains unchanged; unavailable measurements and product acceptance remain explicit.

Issue [#53](https://github.com/pennel-app/pennel/issues/53) owns this suite. Its corpus and
retrieval receipts are shared with [#43](https://github.com/pennel-app/pennel/issues/43).
Production decision/rerank provider contracts remain in
[#54](https://github.com/pennel-app/pennel/issues/54). This suite does not enable a product
feature, change a default model or enroll a speaker.

## Reproduce offline

From the repository root, with Python 3.11+ and the existing Rust dependencies cached:

```sh
python3 evaluations/decision-models/run.py --offline
python3 -B -m unittest discover -s evaluations/decision-models/tests -v
```

The first command builds the evaluation-only Rust executable with `--no-default-features
--features evaluation`. Cargo runs offline; if crate dependencies are not cached, prepare
them explicitly with the normal project build first. No speech or speaker model is loaded.
CI prepares ordinary build dependencies, then runs the same fixture and mocked-transport
checks without an API key, model download or inference request.

Replay the published actual inference scores with no optional model dependencies:

```sh
python3 evaluations/decision-models/run.py --offline \
  --replay evaluations/decision-models/results/2026-10-28

python3 evaluations/decision-models/run.py --offline \
  --replay evaluations/decision-models/results/openrouter-2026-10-28
```

The compressed predictions are measured local inference, not canned mock replies. Replay
recomputes quality/calibration/ranking metrics; its resource observations retain the
original hardware and sample counts. A regression test compares the replay with the
recorded report, allowing only floating-point roundoff.
Plotting-only confidence knots within 12 decimal places are coalesced by the portability
test when different Python/libm versions split floating-point ties. Stored predictions,
calibration thresholds, coverage, decisions and acceptance gates are not changed.

Reports are written under `build/decision-model-evaluation/offline/`. The semantic digest
covers deterministic scores rather than timing noise. Model inputs and predictions are
synthetic artifacts; diagnostics contain identifiers and sanitized errors, never secrets.
The Rust bridge creates and removes its own temporary library. It never opens the installed
Pennel profile, settings, meeting directory or credential store.

## Corpus and comparisons

The corpus generator and its provenance are versioned with the suite. There are 90 meeting
groups, with 10 development, 10 calibration and 10 held-out groups for each of English,
Dutch and mixed language. Related meetings and label-order variants stay in the same group.
Exact evidence offsets are Unicode code-point offsets within a transcript segment, notes
or summary. The core indexes the actual Markdown mirror; source spans are mapped to its
packed passages without assuming the first timestamp is a claim's exact position.

The deterministic baselines call the real Rust implementation:

- Public search uses its existing grouped full-text behavior.
- Ask uses its own whole-meeting/retrieved/recent selection. Candidate recall is measured
  before reranking; an omitted source cannot be recovered by a reranker.
- Cleanup measures the existing overlap/length guard, separately from generated-output
  fidelity. Accepting a corrupt synthetic proposal is a guard failure, not evidence that
  a provider produced that proposal.
- Citation parsing verifies syntax and source membership. Factual support needs the
  separate evidence rubric; a valid citation is insufficient.
- Template selection and text-based speaker names currently have no inference baseline.
  Their current automatic-suggestion coverage is zero. Acoustic recognition is outside
  this text benchmark.

The generative controls reuse Pennel's actual Q&A, summary/map-reduce and cleanup messages.
They require an explicitly configured available chat model. Mock replies only verify
transport/scoring. No other generative model search or diffusion cleanup comparison is
part of this issue.

## Freeze, calibrate and evaluate

Prepare isolated inference environments and explicitly acquire verified model artifacts
using the [worker setup instructions](../evaluations/decision-models/workers/README.md).
The release executable used by the recorded live commands is built separately from the
debug executable used by the default offline command:

```sh
CARGO_TARGET_DIR=build/evaluation-core-target scripts/with-toolchain.sh cargo build \
  --offline --locked --manifest-path core/Cargo.toml --no-default-features \
  --features evaluation --bin pennel-eval --release
```

`evaluations/decision-models/protocol.json` contains the task criteria, resource budgets,
comparison method, calibration rules and grounding rubric. Do not tune these after viewing
held-out results. Pinning includes source/corpus/protocol hashes, candidates, prompts,
ordered labels, question grouping, model/tokenizer artifacts, precision and hardware.

Prepare an experiment without scoring held-out model outputs:

```sh
python3 evaluations/decision-models/run.py --prepare \
  --config evaluations/decision-models/configs/experiment.local.json \
  --frozen evaluations/decision-models/locks/experiment.json
```

Commit the frozen manifest before held-out inference. Development runs may be used to
repair adapters and prompts. Fit calibration only on the calibration split; persist the
calibration context and threshold with the model/route/precision/prompt/label/grouping
identity. A change invalidates that calibration. A new experiment needs a new freeze.
The prompt-family hash binds all authored questions through the frozen corpus hash, task,
subtask and primitive; ordered labels remain separate contexts. Per-case wire requests
retain their own identities. Correct `unknown`/`abstain` decisions remain classification
outcomes, but never count toward actual name/template suggestion coverage.

```sh
python3 evaluations/decision-models/run.py --live --split development \
  --config evaluations/decision-models/configs/experiment.local.json \
  --frozen evaluations/decision-models/locks/experiment.json \
  --output build/decision-model-evaluation/development
```

Use the same configuration and committed freeze for calibration and held-out runs.
Incomplete judgments, identity drift and missing measurements are explicit acceptance gaps.
No aggregate score across languages selects a universal winner.

```sh
python3 evaluations/decision-models/run.py --live --split calibration \
  --config evaluations/decision-models/configs/measured-2026-10-28.json \
  --frozen evaluations/decision-models/locks/2026-10-28.json \
  --binary build/evaluation-core-target/release/pennel-eval \
  --output build/decision-model-evaluation/calibration
python3 evaluations/decision-models/run.py \
  --calibrate build/decision-model-evaluation/calibration/predictions.json \
  --frozen evaluations/decision-models/locks/2026-10-28.json \
  --output build/decision-model-evaluation/calibration
python3 evaluations/decision-models/run.py --live --split heldout --measure \
  --config evaluations/decision-models/configs/measured-2026-10-28.json \
  --frozen evaluations/decision-models/locks/2026-10-28.json \
  --calibration build/decision-model-evaluation/calibration/calibration.json \
  --binary build/evaluation-core-target/release/pennel-eval \
  --output build/decision-model-evaluation/heldout
```

Those historical live commands use the recorded generation commit `4a3135b`. For later
checkouts or hardware, use the preparation step with a new manifest path, commit it, and
substitute that path in each command. Do not overwrite the original measurement record.

The checked-in measured configuration describes this experiment, including unavailable
routes. Its OpenRouter account/retention gate is deliberately unverified. Supplying a key
does not bypass that gate. Changing the configuration requires a new freeze. The small
generative example's `max_cases` is a global smoke-test cap; omit it for full task/language
coverage, as the measured configuration does.

## Hosted routes and billing

The hosted follow-up uses `configs/openrouter-2026-10-28.json` and timestamped public
catalog/endpoint/ZDR evidence under `evaluations/decision-models/evidence/`. It requests
provider ZDR and denies data collection on explicitly selected global routes. This does
not establish regional residency or inspect account logging/opt-in settings. Only
synthetic content is submitted. Evaluation credentials can reside in a dedicated macOS
Keychain entry (`dev.pennel.evaluation.openrouter`, account `OPENROUTER_API_KEY`) and are
injected into benchmark subprocesses; no secret belongs in the configuration or Git.

Quality requests use bounded concurrency of four with ordered receipts and at most four
in-flight calls. Question grouping remains one case per request. Cancellation prevents
new paid requests; already-sent calls remain covered by reservations and transport
timeouts. Latency-panel requests run serially and are reported separately from concurrent
quality-workload timings.

Run the complete calibration and held-out corpus before latency repeats. Admit the entire
fixed latency panel only if its conservative bound fits the actual reconciled remaining
US$2 budget; otherwise mark it unmeasured. This admission rule depends on spend, never
held-out quality. All development probes, calibration, testing and latency requests share
the persistent ledger. The selected endpoint's pricing supplies reservations and price
caps; a cheaper model-level catalog route cannot stand in for the pinned provider.

The Cloudflare Clef route additionally enforces a conservative 2,000-byte serialized-state
bound because its published state-truncation behavior is narrower than its advertised
context window. Oversized cases are unavailable rather than silently truncated.

OpenRouter is preferred when the requested version and required typed decision primitive
are available. The optional runner uses one explicitly selected `OPENROUTER_API_KEY`.
An environment key alone never enables inference. Copy an example into an ignored local
configuration, explicitly enable it, choose its retention/region policy, and name its
fixed model/upstream route. Keep the key out of JSON and Git.

The default API is `POST https://openrouter.ai/api/alpha/decisions`; explicit System One
mode uses `/api/v1/systemone`. Neither is chat completions. Preserve binary probabilities,
choice/ordinal distributions and confidence metadata. Discovery requests include
`output_modalities=decisions`; catalog presence is followed by capability/identity checks.

Pin `provider.only` to the exact endpoint tag and set `allow_fallbacks:false`. Do not use
latest aliases, automatic routers or model fallback arrays. Keep direct-provider runners
for explicitly configured missing capabilities or route comparisons; runtime failures do
not trigger a destination change. A regional rejection is not permission to use the global
domain. Provider/model identity and public checkpoint identity are separate: `liquid/d1`,
`liquid/d1.1` and `LiquidAI/d1-2B` are not interchangeable without exact evidence.

Record requested/actual model and provider, routing configuration, request/response ID,
usage and `usage.cost` for every attempt. Missing billed cost remains unknown. Account for
repeated state, grouped questions and any explicitly configured retry. The experiment
spend ceiling is US$2 total, not per provider or per split. Never present a catalog price
calculation as a returned billed charge.

OpenRouter end-to-end timing is its own series, separate from direct and local results.
Hosted client startup does not measure provider model load time or server cold start.
Provider-side memory and hardware remain unmeasured unless separately established.

Primary API references: [Decisions](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request),
[System One](https://openrouter.ai/docs/guides/community/typesafe-sdk),
[catalog](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties),
[routing](https://openrouter.ai/docs/guides/routing/provider-selection),
[regional routing](https://openrouter.ai/docs/guides/features/in-region-routing).

## Evidence and remaining gates

Local artifacts use pinned revisions and isolated worker environments. Explicit downloads
are bounded by 3 GiB in total; no ordinary test downloads them. Runtime/worker size and
measured owned-process RSS are distinct from weight metadata and hypothetical native app
size. The optional runtime has not been embedded into a product package.

The suite publishes per-language recommendations to adopt, retain or defer for ranking,
answerability/support, meeting events, templates, names and cleanup. Adoption requires the
frozen task and privacy/resource criteria. Synthetic benchmark results do not complete
packaged-app, physical-device or Cloud deployment acceptance. Keep #53 open while its
required evidence is incomplete.

The first run verified 2,770,266,421 downloaded bytes and measured 3,144 successful held-out
cases, with 192 over-context cases retained as unavailable in the denominators. No hosted
requests were made. Full selected-route retention/region verification, hosted usage/cost
measurements, generative answer/citation support, Q8 parity, other local candidates and
packaged/platform acceptance remain explicit gaps. The small synthetic corpus is not
evidence of broad production accuracy.

The hosted follow-up made 30,162 requests for US$0.313064388, including development,
calibration, testing, warmups and timing. All missing-cost service failures were reconciled
against current-key usage before paid work continued. Failed requests/samples remain
failures; only previously unsent work was continued. Mercury's serial timing is partial
after a gateway error. The [report](../evaluations/decision-models/results/openrouter-2026-10-28/README.md)
includes API timings separately from ranking workload/accounting time and records the
template coverage ceiling and owner/ambiguity calibration limitations. No frozen criteria
were changed after viewing held-out results.

## Follow-up experiments and replay

The original reports, criteria and predictions remain unchanged. The suggestion supplement
uses 36 translation-linked meeting groups (108 short meeting documents), with 12 groups
per split. Its template prevalence permits 66.7% correct suggestion coverage; fixed-name
suggestion prevalence permits 75%. Owners include both speakers and unknown. Each ordered
task/language context has 12 calibration groups and multiple classes before service
failures. Distinct topics, statements, actions, quantities and deadlines prevent text reuse
between splits. The 50% coverage, precision/F1 requirements and calibration minima are
unchanged. This controlled supplement is not evidence of natural-meeting generalization.

Replay its actual typed predictions and stored calibration without credentials:

```sh
python3 evaluations/decision-models/followup.py --offline \
  --replay evaluations/decision-models/results/followup-2026-10-28/supplement \
  --output build/decision-model-evaluation/replay-suggestions
```

The generative experiment uses the existing core prompts, request-body builder, source
selection, citation parser and cleanup guard. It explicitly requests the dated GPT-4.1
mini checkpoint through `azure/swedencentral`, verifies its catalog canonical identity and
public ZDR eligibility, and records the returned alias and Azure provider. An alias response
does not independently verify server weights. This separate chat baseline never replaces a
typed Decisions call. It measures an evaluation-selected model, not a user's saved provider.

Its frozen workload contains 42 questions, six unique dictation cleanups and six summaries
from two held-out meetings per language. Stored native-AI evidence reviews cover every
substantive claim, exact source/citation support, language fidelity and all six original
marked event statements per summary. Response, rubric, raw-input and Unicode source-span
hashes bind the reviews. The leader checked the disputed claims across languages. These
are inspectable AI adjudications, not independent human or real-meeting acceptance.

```sh
python3 evaluations/decision-models/generate.py \
  --replay evaluations/decision-models/results/followup-2026-10-28/generative \
  --output build/decision-model-evaluation/replay-generation
```

The repeated generative panel has five fresh-client samples per language plus 30 warm
samples for each Q&A, cleanup and summary cell: 294 requests including excluded warmups.
Its text repetitions add no semantic quality credit. Metadata setup is separate from the
OpenRouter request timer. Server cold state, hardware, precision, memory and artifact size
remain unknown; the owned Python command's memory measurement is a client measurement.

The supplement's serial panel uses the same updated prompts as its quality experiment.
All timing panels require the entire conservative remaining bound to fit the reconciled
shared ledger. Failed consumed slots remain failed; explicit continuations fill unsent
slots and include their excluded warmups. Partial ranking calls contribute their individual
billed attempts even when the outer result has no complete cost or wall time.

For a new live experiment, copy an explicit configuration, revalidate its exact route,
prepare a new freeze and commit it before testing. `followup.py --prepare/--live` handles
the suggestion experiment; `generate.py --prepare/--live --measure` handles the generative
experiment. Both preserve the canonical cumulative ledger rather than creating a new
US$2 allowance. New source or hardware needs a new freeze; old calibrations and held-out
predictions must not be relabelled. The report includes the exact historical commands,
frozen manifests, accounting observations and continuation scripts.

The measured Q&A baseline conflates Wi-Fi passwords with unrelated access-code privacy
in several answers. Summary checkboxes promote tentative or unassigned work into actions.
Mixed-language cleanup preserves the proposition but translates its English date phrase.
These failures are retained in the evidence and per-language recommendations. No prompt,
threshold, model default or production feature was changed in response to held-out results.

Recorded-data tests exposed Python-version-dependent floating-point summation at a frozen
temperature-scaling cutoff. Normalization now uses explicit compensated `math.fsum`.
Replay preserves every recorded Python 3.14 selection and gate on Python 3.11 without
changing fits, cutoffs or labels. Residual display-level roundoff is compared with the
same numerical tolerance as the original replay tests.

Cached GLiClass multilingual and GLiNER classifiers also ran on the same repaired corpus:
432 successful calibration cases and 432 successful held-out cases, with no new downloads
or hosted calls. Their owner/template gates fail; GLiClass candidate-name suggestions meet
Dutch/mixed point gates but English coverage fails. GLiNER has no complete distribution for
the calibrated path. These remain bounded research results with native packaging deferred.

```sh
python3 evaluations/decision-models/followup.py --offline \
  --replay evaluations/decision-models/results/followup-2026-10-28/supplement-local \
  --output build/decision-model-evaluation/replay-local-suggestions
```

## Incremental prefix qualification — #96

`evaluations/decision-models/incremental.py` is the separate, versioned prefix experiment.
It preserves every historical protocol and result. The owned corpus contains 30 split-disjoint
meeting families, English/Dutch/mixed translations and two linked text variants per meeting.
The ASR variant is an authored text perturbation, not a measurement of natural recorded ASR.
Eight finalized cutoffs per stream produce 4,500 scheduled cases. Related translations,
perturbations and prefixes remain in the same development/calibration/held-out family.

The four use cases are independent: finished-meeting suggestions, explicit live suggestions,
source assessment, and background live suggestions. Finished-meeting cases use the terminal
snapshot; the other cases use only sources available at each cutoff. Provisional hypotheses,
future notes, later answers and later retractions cannot become earlier evidence. Input requests
include the meeting/session, finalized source IDs, exact text/revisions/fingerprints, timestamps,
acknowledged notes and cutoff. The exported `decision_request` uses #54's typed choice contract;
a bounded classifier result alone does not establish complete-item extraction.

Replay owned fixtures without credentials, model loading, downloads or network requests:

```sh
python3 -B -m unittest discover -s evaluations/decision-models/tests -p 'test_incremental*.py'
python3 -B evaluations/decision-models/incremental.py --offline \
  --output build/decision-model-evaluation/incremental-fixtures
python3 -B evaluations/decision-models/incremental.py \
  --freeze build/decision-model-evaluation/incremental-fixtures/freeze.json \
  --predictions build/decision-model-evaluation/incremental-fixtures/predictions.jsonl \
  --output build/decision-model-evaluation/incremental-replay
```

The replay reproduces counts, fits, gates, report hashes and candidate-record bindings. Perfect
fixture predictions remain `offline_fixture` evidence with **zero adopted records**. Historical
point passes and successful connection samples cannot change that outcome. Diagnostics print
only counts/hashes/status; the synthetic source corpus belongs in explicit evaluation artifacts.

For a separately authorized inference experiment, provide an explicit configuration containing
`configured` (protocol, endpoint, model, expected model, upstream, input bound, timeout), `actual`
(expected model/revision/provider/artifact hash), precision, grouping, preprocessing and resource
profile. No credential values or secret-bearing endpoint query strings belong in this file.
Export calibration requests first. Feed the recorded JSONL results back with `--calibrate`; that
writes a new freeze containing the fitted artifact and calibration-input hash. Measured held-out
request export is refused until this stage has completed:

```sh
python3 -B evaluations/decision-models/incremental.py --configuration CONFIG.json \
  --requests calibration --output build/incremental-calibration
python3 -B evaluations/decision-models/incremental.py \
  --freeze build/incremental-calibration/freeze.json --predictions CALIBRATION.jsonl \
  --calibrate --output build/incremental-heldout
python3 -B evaluations/decision-models/incremental.py \
  --freeze build/incremental-heldout/freeze.json --requests heldout \
  --output build/incremental-heldout
```

These commands export or replay data; they execute no inference. Each prediction is linked to its
exact case/input/freeze hash, actual identity and execution-evidence class. Duplicate/unlinked
responses fail closed. Oversized requests are explicitly refused without truncation. A changed
model alias, route, expected model, input bound, timeout, label order, prompt, schema, preprocessing,
calibration artifact or resource profile needs a new exact-context freeze.

The report keeps event classification separate from complete source-quoted item quality. It
scores owners, verbatim due phrases, exact anchors, duplicates, missed items, final status,
retractions and retained user review state independently. Unknown fields stay null. Positive
end-to-end event credit requires a complete supported tuple; both component and end-to-end
macro-F1 must reach 0.90, with accepted-item precision at least 0.95 and supported owner/final
status. Source support and answerability retain separate 0.98 selective-precision / 0.50 coverage
gates. Deadline normalization, candidate recall and reranking remain separate, unmeasured stages.

The fixture claim is application-authored. Source-assessment adoption additionally requires
frozen actual generated-response text/claim bounds and exact review hashes supplied through an
explicit `--corpus` file; missing/pending judgments remain unavailable. Classifying a fixture
claim cannot become a whole-answer correctness claim. Calibration reuses the existing machinery:
at least ten observations, two classes and two meeting families per exact context, fitting on
calibration only and preserving genuine distributions rather than confidence/logit substitutes.

Latency uses the existing type-7 p50/p95 implementation and discloses sample counts. CPU, RSS,
executed requests, usage/cost and recording impact are unavailable unless actually measured.
Local warm p95 is bounded at five seconds for events and 750 ms for source assessment. Live
use requires separate foreground-impact acceptance; background additionally requires resource
acceptance. UI fixture timings do not supply those measurements.

`candidate-records.json` uses the shared schema-version-1 Rust qualification contract. It is an
input to review, never an adoption write: the app-owned registry remains empty until exact
inference, quality, calibration, identity and runtime gates are satisfied. The chosen generative
model remains responsible for readable answers. The new protocol authorizes no hosted spend,
artifact acquisition, retries, fallback, continuous transmission or production-default changes.

Existing cached artifacts can be audited independently, without loading or acquiring models:

```sh
python3 -B evaluations/decision-models/incremental.py \
  --verify-cached-model gliclass-multilingual --verify-cached-model gliner-multilingual \
  --verify-cached-model gte-reranker --verify-cached-model gliclass-v4-q8 \
  --output build/incremental-artifact-audit
```

An available Q8 graph still does not establish reference-parity quality. Keep unavailable
inference, natural-ASR transfer, generated-claim, native/resource and real-call evidence explicit.
