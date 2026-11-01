import assert from "node:assert/strict";
import test from "node:test";
import { inferenceConfig, inferenceFingerprint, inferenceProtocolsForProvider, InferenceVerificationState } from "../src/inference-state.ts";

const config = { provider_id: "custom", protocol: "mercury", model: "decision-model", expected_model: null, upstream: null, max_input_bytes: 16000, timeout_secs: 30 };
const info = { settings: { providers: [{ id: "custom", kind: "custom", name: "My server", base_url: "https://example.com/v1" }], decision: config }, keys: { custom: true }, provider_revisions: { custom: 1 } };
const result = { verified: true, message: "Sample passed", identity: null };

test("old settings do not implicitly configure either inference capability", () => {
  assert.equal(inferenceConfig({ settings: { providers: [] } }, "decision"), null);
  assert.equal(inferenceConfig(info, "reranking"), null);
});

test("inference verification identity includes protocol, pinning, budgets, endpoint and credential revisions", () => {
  const fingerprint = inferenceFingerprint(info, "decision");
  for (const change of [
    { protocol: "perplexity" }, { model: "other" }, { upstream: "other" }, { expected_model: "versioned" }, { max_input_bytes: 12000 }, { timeout_secs: 15 },
  ]) assert.notEqual(inferenceFingerprint({ ...info, settings: { ...info.settings, decision: { ...config, ...change } } }, "decision"), fingerprint);
  assert.notEqual(inferenceFingerprint({ ...info, provider_revisions: { custom: 2 } }, "decision"), fingerprint);
  assert.notEqual(inferenceFingerprint({ ...info, keys: { custom: false } }, "decision"), fingerprint);
  assert.notEqual(inferenceFingerprint({ ...info, settings: { ...info.settings, providers: [{ ...info.settings.providers[0], base_url: "https://other.example/v1" }] } }, "decision"), fingerprint);
});

test("late verification cannot verify a changed configuration", () => {
  const state = new InferenceVerificationState();
  assert.equal(state.begin("old", "before"), true);
  assert.equal(state.begin("duplicate", "before"), false);
  assert.equal(state.finish("old", "after", result), false);
  assert.equal(state.get("after"), null);
  assert.equal(state.running, false);
});

test("Cloud verification is scoped to the signed-in account epoch", () => {
  const cloud = { ...info, cloud_signed_in: true, cloud_account_epoch: "account-1", settings: { ...info.settings,
    providers: [{ id: "cloud", kind: "pennel_cloud", name: "Pennel Cloud", base_url: "https://api.pennel.app/v1" }],
    decision: { ...config, provider_id: "cloud", protocol: "pennel_cloud", model: "pennel-auto" } } };
  const before = inferenceFingerprint(cloud, "decision");
  assert.equal(inferenceFingerprint({ ...cloud, provider_revisions: { cloud: 2 }, keys: { cloud: true } }, "decision"), before);
  assert.notEqual(inferenceFingerprint({ ...cloud, cloud_account_epoch: "account-2" }, "decision"), before);
  assert.notEqual(inferenceFingerprint({ ...cloud, cloud_signed_in: false }, "decision"), before);
  assert.equal(inferenceFingerprint({ ...info, cloud_account_epoch: "unrelated", cloud_signed_in: true }, "decision"), inferenceFingerprint(info, "decision"));
  const state = new InferenceVerificationState();
  state.begin("cloud-sample", before);
  assert.equal(state.finish("cloud-sample", inferenceFingerprint({ ...cloud, cloud_account_epoch: "account-2" }, "decision"), result), false);
  assert.equal(state.get(before), null);
});

test("managed Cloud eligibility comes from explicit capability and account metadata", () => {
  const cloud = { protocol: "pennel_cloud", decision: true, reranking: true, managed: true, allowed_provider_kinds: ["pennel_cloud"] };
  const tei = { protocol: "tei", decision: false, reranking: true, managed: false, allowed_provider_kinds: ["custom"] };
  const router = { protocol: "open_router_decisions", decision: true, reranking: false, managed: false, allowed_provider_kinds: ["open_router", "custom"] };
  const account = kind => ({ id: kind, kind, name: kind, base_url: "" });
  const all = [cloud, tei, router];
  for (const capability of ["decision", "reranking"]) assert.deepEqual(inferenceProtocolsForProvider(all, account("pennel_cloud"), capability), [cloud]);
  assert.deepEqual(inferenceProtocolsForProvider(all, account("custom"), "reranking"), [tei]);
  assert.deepEqual(inferenceProtocolsForProvider(all, account("open_router"), "decision"), [router]);
  assert.deepEqual(inferenceProtocolsForProvider(all, account("open_router"), "reranking"), []);
  assert.deepEqual(inferenceProtocolsForProvider([{ ...cloud, managed: false }], account("pennel_cloud"), "decision"), []);
  assert.deepEqual(inferenceProtocolsForProvider([{ ...cloud, allowed_provider_kinds: ["custom"] }], account("custom"), "decision"), []);
  assert.deepEqual(inferenceProtocolsForProvider([{ protocol: "pennel_cloud", decision: true }], account("pennel_cloud"), "decision"), []);
});

test("cancellation invalidates an old response without losing a newer request", () => {
  const state = new InferenceVerificationState();
  state.begin("old", "before");
  assert.equal(state.invalidate(), "old");
  state.begin("new", "after");
  assert.equal(state.finish("old", "before", result), false);
  assert.equal(state.running, true);
  assert.equal(state.finish("new", "after", result), true);
  assert.equal(state.get("after"), result);
  assert.equal(state.get("changed"), null);
});
