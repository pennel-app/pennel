# Qualify Review using an explicitly pinned provider-reported model version

Status: accepted by the owner on 11 November 2026 in the Review functional-fix follow-up to #96.

## Context

The actual local and hosted Decision preflights returned useful typed responses but omitted a separate serving revision. The hosted route reported the dated model `perplexity/pplx-decider-v1.2-20261016` and provider `Perplexity`. Requiring another revision field made every tested route unavailable; a public checkpoint digest did not prove the hosted serving weights. The owner selected exact provider-reported model/version and route pinning, explicitly without independently verified remote weights.

This revisits the exact-identity and per-window transport assumptions used by [ADR 0004](0004-automatic-suggestion-analysis.md). The subsequent C7 trial passed for the exact manual Dutch/resource context and its candidate was adopted. Separate automatic-consent and foreground-impact requirements remain in force; native positive generation and final registry-sensitive build verification are recorded separately from qualification.

## Decision

Add an explicit `provider_reported_version` qualification identity kind, initially limited to manual finished Dutch suggestions through the official OpenRouter Decisions endpoint and pinned Perplexity route. Bind the requested model, exact expected and returned dated model, returned provider, endpoint, route, input/time bounds and the full production prompt/schema/grouping/preprocessing/resource context. A changed or unversioned returned model or a changed provider/route fails closed.

Keep raw missing revision and artifact information missing. Do not copy configuration, a catalog name or a checkpoint manifest into response metadata. Presets expose `weights_independently_verified: false`. The existing default identity kind retains its separate serving-revision/artifact requirements, including for the local HTTP runtime. Old serialized contexts omit the new default kind and retain their hashes.

Identity eligibility alone does not enable Review. Require real production-bound calibration, disjoint held-out complete-item/source quality and client-resource evidence before adding a registry record. Preserve the existing quality thresholds and source checks. V3 uses raw state choices plus an independent SOURCE-0 kind-support choice: emit only an eligible state whose kind is explicitly supported. An absent kind is a clear negative; unclear support or a provider abstention remains unassessed. A wholly unassessed job reports insufficient evidence. Production execution and raw replay use the same assessment helper.

State, support and complete-item results are measured separately. Support rows cannot inflate the original four-component state/item F1, and unassessed slots receive no successful item credit. Raw and diagnostically calibrated support F1 must additionally meet 0.90. The original state/item F1, precision, zero-unsupported/duplicate, complete-job, source and resource gates remain. Fitted calibration is diagnostic evidence and must not be presented as an active runtime filter or calibrated UI confidence; it cannot revive a raw support abstention.

Bounded OpenRouter choice batches may group up to eight independent source windows in one physical request, inside actual serialized byte limits. Each passage retains its own adjacent context, unique answer mapping and exact sources. V3 requests four state and four support answers per window. Existing whole-job limits, source/configuration/credential cancellation guards, foreground priority and all-or-nothing commit remain. New prompt/schema/grouping/preprocessing hashes and export/replay v3 invalidate earlier contexts. Other transports and extraction routes keep singleton requests and their existing question behavior.

Manual qualification does not authorize automatic processing. The UI offers a manual request with its actual destination; the automatic checkbox stays unavailable until the additional foreground-impact policy passes. The tested setup remains a draft until the owner saves it, and reuses an explicitly selected compatible account and its OS-stored credential.

## Evidence and limits

The historical raw replies/freezes remain under `pr-assets/pr-250`, and incomplete A and failed B evidence remain retained. B's exact-source cards still contained 69 unsupported semantic items and failed precision; they did not receive approval. Seen-B cases are development regressions only, with no C7 quality credit. Fresh C7 calibration/held-out material and its actual production requests were frozen before inference, and the fitted lock was committed before held-out execution. The independent audit reproduced all C7 report fields and verified all 430 physical requests.

Qualification uses only invented Dutch material and the original cumulative US$2 hosted ceiling; no private meetings, new model/runtime downloads, automatic retries or fallback. C7's adopted record is exact-context manual Dutch evidence, not universal model approval. Provider-reported identity does not establish immutable remote weights, regional residency or provider account retention settings. Natural-ASR, human, other-language and automatic/background acceptance remain separate.

Primary contract: [OpenRouter Decisions](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request). [C7 qualification and delivery evidence](../evidence/review-provider-version-96-2026-11-12.md) records the adopted candidate and remaining native/final-build checks. Broader acceptance remains open under #96.
