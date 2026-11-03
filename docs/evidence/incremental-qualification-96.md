# Incremental qualification — #96

This phase supplies frozen prefix evaluation and exact-context eligibility for
[#96](https://github.com/pennel-app/pennel/issues/96), under
[#94](https://github.com/pennel-app/pennel/issues/94). It builds on the live-speaker
contract from #99. The application registry has **zero adopted qualifications**;
manual and background suggestions remain unavailable until their own evidence passes.

The [machine receipt](incremental-qualification-96.json) records the checked source,
commands, log hashes and gaps. The complete owned-fixture
[freeze and replay report](../../evaluations/decision-models/results/incremental-2026-11-03/README.md)
are checked in separately from historical measured-model results.

## Contract

- `MeetingQualificationRegistry::bundled().admit(...)` uses only the versioned,
  application-owned `assets/qualifications/meeting-assistance.json`. A connection test
  and caller-supplied candidate record cannot grant eligibility.
- Finished-meeting suggestions, manual live suggestions, source assessment and
  background live suggestions are distinct contexts. English, Dutch and mixed content
  are qualified separately; interface language does not choose content language.
- Eligibility binds the configured protocol, endpoint, model, expected model, input
  bound and timeout; actual model/revision/provider/artifact; task, prompt, schema,
  ordered labels, grouping, preprocessing, calibration and resource profile. Actual
  identity is rechecked after execution. Ambiguous deployments and changed identities
  fail closed.
- `PennelEngine::meeting_analysis_availability(request_json)` is an owned UniFFI status
  API. It reads configuration and the registry without provider probes, credential
  lookup, meeting content or inference. `qualify_meeting_analysis` returns an opaque
  Rust admission contract; it does not replace cancellation, resource or source guards.
- The source-assessment evidence and eligibility schema are shared with #56. Retrieval
  and the source-check UI remain owned by that issue. Readable answers continue to use
  the selected generative model.

## Evidence

The initial registry regressions failed on exact-context admission, independent
resource gates and identity validation. The finished implementation passes the
registry tests and the offline evaluation suite; detailed counts are in the receipt.

The corpus contains 30 disjoint meeting families, English/Dutch/mixed variants and
eight finalized cutoffs per stream: **1,440 prefixes and 4,500 cases**. Family-linked
translations, authored ASR perturbations and prefixes remain in the same split.
The fresh offline run and replay are byte-identical, with semantic report SHA-256
`1353118da9078e934fd700c00ffccb8d51f09aed63efcbcf68f66ec44bacb895`.
All 12 fixture candidate records remain unqualified and unadopted.

The existing four cached artifact manifests were hash-verified offline. No model was
loaded, downloaded or called. Artifact sizes measure files, not inference memory.
Available Q8 files do not establish reference parity. The 287 Python checks cover
prefix isolation, held-out separation, changed/duplicate/unlinked replies, exact source
anchors, owner/deadline/status scoring, generated-claim provenance and refusal of
oversized contexts. Deterministic source-bound Rust evaluation fixtures remain stable.

## Acceptance remaining

Fixture scores are harness evidence. Actual prefix-model predictions, generated-answer
claim review, natural recorded ASR transfer, candidate recall, reranking, deadline
normalization and model/resource measurements remain unavailable. No local warm p95,
CPU/RSS, executed inference usage, typing/Stop impact or acoustic quality pass is claimed.
Live activation additionally needs foreground-impact acceptance; background activation
also needs separate resource acceptance. #97 stays gated.

The additive core ABI is generated and typechecked against matching Swift declarations.
Native app, clean/repeated Prebuild, hosted iPhone tests and simulator/device Release
results from upstream #99 are separate evidence, not exact-head #96 native acceptance.
The integrated downstream #57 candidate repeats the affected mobile pipeline. CI,
merge, real-call, signed-device, installed Windows/Linux and deployment acceptance
remain separate gates. This phase adds no UI surface or screenshot claim.
