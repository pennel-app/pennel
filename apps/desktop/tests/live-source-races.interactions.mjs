import { selectMeetingPane, openMeetingMore } from "./meeting-workspace-test-controls.mjs";
const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 200; attempt++) { if (await check()) return; await pause(); }
  throw new Error(message);
}

export async function checkLiveSourceRaces() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__?.workspace, "Use the owned synthetic fixture.");
  const fixture = window.__PENNEL_FIXTURE__;
  await until(() => document.querySelector(".live-workspace"), "Workspace did not mount.");
  const root = document.querySelector(".live-workspace");
  const select = pane => selectMeetingPane(root, pane);
  select("questions");
  const input = root.querySelector(".live-question-input");
  input.value = "Synthetic question for asynchronous source navigation";
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  root.querySelector(".live-question-submit").click();
  await until(() => fixture.workspace.requests().length, "Question was not admitted.");
  fixture.workspace.complete(fixture.workspace.requests().at(-1).request_id);
  await until(() => root.querySelector(".citation"), "Answer has no citation.");
  const citation = root.querySelector(".citation");
  const originalDigest = crypto.subtle.digest.bind(crypto.subtle);
  let release;
  let hashing = false;
  const blockHash = () => {
    hashing = false;
    const waiting = new Promise(resolve => { release = resolve; });
    crypto.subtle.digest = async (...args) => { hashing = true; await waiting; return originalDigest(...args); };
  };
  try {
    blockHash();
    citation.click();
    await until(() => hashing, "Source validation did not begin.");
    await openMeetingMore(root, "meetings.workspace_suggestions");
    release();
    await pause(100);
    assert(!root.querySelector('[data-live-pane="suggestions"]').hidden, "An older source jump overrode the latest pane selection.");
    assert(!root.querySelector(".live-source-highlight"), "An abandoned source jump highlighted content.");

    select("questions");
    blockHash();
    citation.click();
    await until(() => hashing, "Second source validation did not begin.");
    fixture.workspace.replaceTranscript([{ start_ms: 0, end_ms: 8000, speaker: "others", text: "Synthetic changed passage while source hashing was pending", failed: false, voice: 1 }]);
    await until(() => root.querySelector(".live-transcript").textContent.includes("while source hashing"), "Changed transcript did not render.");
    release();
    await until(() => !root.querySelector(".live-source-notice").hidden, "A source changed during validation has no recovery explanation.");
    assert(!root.querySelector(".live-source-highlight"), "Changed evidence was highlighted after delayed validation.");
    return { abandonedJumpSuppressed: true, sourceRevalidatedAfterAwait: true, syntheticOnly: true };
  } finally {
    release?.();
    crypto.subtle.digest = originalDigest;
  }
}
