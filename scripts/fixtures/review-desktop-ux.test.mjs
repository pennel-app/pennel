import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { parseReviewArguments, reviewLinks, reviewFlows } from "../review-desktop-ux.mjs";

test("review options reject invalid flows, locale, appearance and ports", () => {
  assert.deepEqual(parseReviewArguments([]), { flow: "navigation", locale: "en", appearance: "light", port: 0, help: false });
  assert.equal(parseReviewArguments(["--flow", "setup", "--locale", "nl", "--appearance", "dark", "--port", "4310"]).port, 4310);
  for (const args of [["--flow", "release"], ["--locale", "fr"], ["--appearance", "both"], ["--port", "NaN"], ["--port", "-1"], ["--port", "65536"], ["--flow"], ["--unknown"]]) {
    assert.throws(() => parseReviewArguments(args), /review|flow|locale|appearance|port|Unknown|Missing/i);
  }
});

test("review links isolate every scenario and make reset explicitly profile-scoped", () => {
  const options = parseReviewArguments(["--flow", "meeting", "--locale", "nl", "--appearance", "dark"]);
  const first = reviewLinks("http://127.0.0.1:4310", options, "review-first");
  const second = reviewLinks("http://127.0.0.1:4310", options, "review-second");
  const profiles = new Set();
  for (const [index, item] of first.entries()) {
    const url = new URL(item.url), reset = new URL(item.reset);
    assert.equal(url.origin, "http://127.0.0.1:4310");
    assert.equal(url.searchParams.get("locale"), "nl");
    assert.equal(url.searchParams.get("theme"), "dark");
    assert.equal(url.searchParams.get("qa"), reviewFlows.meeting.scenarios[index].name);
    const profile = url.searchParams.get("profile");
    assert.ok(profile.startsWith("review-first-"));
    assert.equal(reset.searchParams.get("profile"), profile);
    assert.equal(reset.searchParams.get("review_reset"), "1");
    assert.notEqual(profile, new URL(second[index].url).searchParams.get("profile"));
    profiles.add(profile);
  }
  assert.equal(profiles.size, first.length);
  assert.throws(() => reviewLinks("https://example.com", options, "review-first"), /loopback/i);
  assert.throws(() => reviewLinks("http://127.0.0.1:4310", options, "../other-profile"), /session/i);
});

test("every review scenario is a human starting state with an owner fixture and no automatic task steps", async () => {
  const cases = JSON.parse(await readFile(new URL("./visual-cases.json", import.meta.url), "utf8"));
  for (const flow of Object.values(reviewFlows)) {
    assert.ok(flow.tasks.length > 0);
    assert.ok(flow.issue.title && flow.issue.url.startsWith("https://github.com/pennel-app/pennel/issues/"));
    for (const scenario of flow.scenarios) {
      const item = cases.find(item => item.name === scenario.name);
      assert.ok(item, `Missing ${scenario.name}`);
      assert.equal(item.owner_review, true);
      assert.equal(item.steps, undefined, `${scenario.name} must leave the tasks for the human`);
    }
  }
});
