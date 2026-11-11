# Qualify Review using an explicitly pinned provider-reported model version

Status: accepted by the owner on 11 November 2026 in the Review functional-fix follow-up to #96.

## Context

The actual local and hosted Decision preflights returned useful typed responses but omitted a separate serving revision. The hosted route reported the dated model `perplexity/pplx-decider-v1.2-20261016` and provider `Perplexity`. Requiring another revision field made every tested route unavailable; a public checkpoint digest did not prove the hosted serving weights. The owner selected exact provider-reported model/version and route pinning, explicitly without independently verified remote weights.

This revisits the exact-identity and per-window transport assumptions used by [ADR 0004](0004-automatic-suggestion-analysis.md) for the newly qualified manual route. Its separate automatic-consent and foreground-impact requirements remain in force.

## Decision

Add an explicit `provider_reported_version` qualification identity kind, initially limited to manual finished Dutch suggestions through the official OpenRouter Decisions endpoint and pinned Perplexity route. Bind the requested model, exact expected and returned dated model, returned provider, endpoint, route, input/time bounds and the full production prompt/schema/grouping/preprocessing/resource context. A changed or unversioned returned model or a changed provider/route fails closed.

Keep raw missing revision and artifact information missing. Do not copy configuration, a catalog name or a checkpoint manifest into response metadata. Presets expose `weights_independently_verified: false`. The existing default identity kind retains its separate serving-revision/artifact requirements, including for the local HTTP runtime. Old serialized contexts omit the new default kind and retain their hashes.

Identity eligibility alone does not enable Review. Require real production-bound calibration, disjoint held-out complete-item/source quality and client-resource evidence before adding a registry record. Preserve the existing quality thresholds and source checks. The runtime uses raw closed-set decisions; fitted calibration is diagnostic evidence and must not be presented as an active runtime filter or calibrated UI confidence.

Bounded OpenRouter choice batches may group up to eight independent source windows in one physical request, inside actual serialized byte limits. Each passage retains its own adjacent context, unique answer mapping and exact sources. Existing whole-job limits, source/configuration/credential cancellation guards, foreground priority and all-or-nothing commit remain. New prompt/grouping hashes and export/replay v2 invalidate earlier contexts. Other transports and extraction routes keep singleton requests.

Manual qualification does not authorize automatic processing. The UI offers a manual request with its actual destination; the automatic checkbox stays unavailable until the additional foreground-impact policy passes. The tested setup remains a draft until the owner saves it, and reuses an explicitly selected compatible account and its OS-stored credential.

## Evidence and limits

The historical raw replies/freezes remain under `pr-assets/pr-250`. New qualification uses only invented Dutch material, a committed freeze and the existing cumulative US$2 hosted ceiling; no private meetings, automatic retries or fallback. Provider-reported identity does not establish immutable remote weights, regional residency or provider account retention settings. Natural-ASR, human and other-language/background acceptance remain separate.

Primary contract: [OpenRouter Decisions](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request). Current functional verification and any adopted record are documented under #96; an adopted record must never be implied before its actual gates pass.
