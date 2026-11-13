import assert from "node:assert/strict";
import test from "node:test";
import { cloudReviewDraft, reviewProcessingSource, validManagedReviewProfile } from "../src/review-processing-state.ts";
import { normalizedInferenceDraft, testedSuggestionDraft } from "../src/inference-state.ts";

const version = "perplexity/pplx-decider-v1.2-20261016";
const account = { id: "cloud-owner", kind: "pennel_cloud", name: "Pennel Cloud", base_url: "https://untrusted-account.example" };
const profile = { available: true, reason_code: "available", profile: "supported_passages_v3", model: "pennel-review-v3",
  configured_model: "perplexity/pplx-decider-v1.2", expected_model: version, upstream: "perplexity", endpoint: "https://api.pennel.app/v1/decisions",
  limits: { max_input_bytes: 65536, max_questions: 64, max_batch_windows: 8, request_timeout_secs: 35 } };
const preset = { provider_kind: "pennel_cloud", qualification_id: "owned-cloud-fixture", language_policy: "nl", identity_evidence: "provider_reported_version",
  weights_independently_verified: false, actual_model: version, actual_provider: "Perplexity",
  configured: { protocol: "pennel_cloud", model: "pennel-review-v3", expected_model: version, upstream: null,
    endpoint: profile.endpoint, max_input_bytes: 65536, timeout_secs: 90 } };

test("a Cloud Review draft requires a real adopted preset and the active session's exact route", () => {
  const expected = { provider_id: account.id, protocol: "pennel_cloud", model: "pennel-review-v3", expected_model: version,
    upstream: null, max_input_bytes: 65536, timeout_secs: 90 };
  const before = structuredClone([account, profile, preset]);
  assert.deepEqual(cloudReviewDraft(profile, [preset], account), expected);
  assert.deepEqual(cloudReviewDraft(profile, [preset], undefined), { ...expected, provider_id: "" }, "Signed-in users should not need to add an advanced connection.");
  assert.equal(cloudReviewDraft(profile, [], account), null, "Configuration metadata alone granted qualification.");
  assert.equal(cloudReviewDraft({ ...profile, endpoint: "https://api-staging.pennel.app/v1/decisions" }, [preset], account), null);
  assert.equal(cloudReviewDraft({ ...profile, expected_model: "perplexity/pplx-decider-v1.2-20261017" }, [preset], account), null);
  assert.equal(cloudReviewDraft({ ...profile, available: false, reason_code: "disabled" }, [preset], account), null);
  assert.equal(cloudReviewDraft(profile, [preset], { ...account, kind: "open_router" }), null);
  assert.equal(cloudReviewDraft(profile, [{ ...preset, configured: { ...preset.configured, expected_model: "different" } }], account), null);
  assert.deepEqual([account, profile, preset], before);
});

test("managed discovery rejects malformed or unsupported metadata without inferring identity", () => {
  assert.equal(validManagedReviewProfile(profile), true);
  for (const patch of [{ endpoint: "http://api.pennel.app/v1/decisions" }, { endpoint: "https://another.example/v1/decisions" },
    { profile: "unknown" }, { model: "pennel-auto" }, { configured_model: "other" }, { upstream: "other" },
    { limits: { ...profile.limits, max_questions: 128 } }, { limits: { ...profile.limits, max_batch_windows: 9 } },
    { limits: { ...profile.limits, request_timeout_secs: 90 } }]) assert.equal(validManagedReviewProfile({ ...profile, ...patch }), false);
  assert.equal(validManagedReviewProfile(null), false);
});

test("qualified Cloud setup reuses the selected account without trusting its editable URL", () => {
  assert.equal(testedSuggestionDraft(preset, account)?.provider_id, account.id);
  assert.equal(testedSuggestionDraft({ ...preset, configured: { ...preset.configured, endpoint: "https://another.example/v1/decisions" } }, account), null);
  assert.equal(testedSuggestionDraft({ ...preset, provider_kind: "open_router" }, account), null);
});

test("managed Review saving keeps the dated pin while legacy Cloud setup remains pennel-auto", () => {
  const config = cloudReviewDraft(profile, [preset], account);
  assert.deepEqual(normalizedInferenceDraft(config), config);
  assert.deepEqual(normalizedInferenceDraft({ ...config, model: " old-draft ", expected_model: "old" }),
    { ...config, model: "pennel-auto", expected_model: null });
});

test("source labels follow explicit task ownership rather than changing an old connection", () => {
  const config = cloudReviewDraft(profile, [preset], account);
  assert.equal(reviewProcessingSource(config, [account]), "cloud");
  assert.equal(reviewProcessingSource({ ...config, model: "pennel-auto" }, [account]), "advanced");
  assert.equal(reviewProcessingSource(null, [account]), null);
  assert.equal(reviewProcessingSource({ ...config, protocol: "local_decider", provider_id: "local" }, [{ id: "local", kind: "local" }]), "local");
});
