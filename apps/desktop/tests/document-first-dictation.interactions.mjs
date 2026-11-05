const assert = (value, message) => { if (!value) throw Error(message); };
const visible = node => !!node?.getClientRects().length && !node.closest("[hidden]");
async function until(check, message) {
  for (let attempt = 0; attempt < 180; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw Error(message);
}
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use an owned synthetic fixture.");
  await until(() => document.querySelector(".dictation-home"), "Dictation home did not open.");
  return document.querySelector(".dictation-home");
}

export async function checkDictationQuickCopy() {
  const root = await owned(), client = window.__PENNEL_PLATFORM__;
  await until(() => root.querySelector("[data-recent-id]"), "Recent results did not load.");
  const row = root.querySelector("[data-recent-id]"), copy = row.querySelector(".recent-dictation-copy");
  assert(visible(copy), "Recent results need a direct Copy action.");
  const { api } = await import("/src/api.ts");
  const entry = (await api.history(200)).find(entry => entry.id === row.dataset.recentId);
  const original = client.invoke.bind(client), copies = [], calls = [];
  let fail = false, historyFail = false;
  client.invoke = async (command, args = {}) => {
    calls.push(command);
    if (command === "history" && historyFail) throw Error("Synthetic history unavailable.");
    if (command === "copy_text") {
      if (fail) throw Error("Synthetic clipboard unavailable.");
      copies.push(args.text);
    }
    return original(command, args);
  };
  try {
    copy.focus();
    assert(document.activeElement === copy, "Quick Copy cannot receive keyboard focus.");
    copy.click();
    await until(() => copies.length === 1 && !copy.disabled && visible(root.querySelector(".recent-copy-status")), "Copy was not acknowledged.");
    assert(copies[0] === entry.text, "Quick Copy changed the stored result.");
    assert(document.querySelector(".dictation-home") === root, "Copy navigated away from Dictation.");
    fail = true; copy.click();
    await until(() => root.querySelector(".recent-copy-status").classList.contains("error") && !copy.disabled, "Clipboard failure has no recoverable feedback.");
    fail = false; copy.click();
    await until(() => copies.length === 2 && !root.querySelector(".recent-copy-status").classList.contains("error"), "Copy cannot recover after clipboard failure.");
    assert(copies[1] === entry.text && !calls.some(command => /generate|transcribe|retry_history/.test(command)), "Copy reprocessed a stored result.");
    historyFail = true; window.__PENNEL_FIXTURE__.emit("history-changed", null);
    await until(() => visible(root.querySelector(".recent-load-status")), "History failure was not explained.");
    assert(row.isConnected && document.activeElement === copy, "A failed refresh discarded a known result or keyboard focus.");
    historyFail = false; window.__PENNEL_FIXTURE__.emit("history-changed", null);
    await until(() => !visible(root.querySelector(".recent-load-status")), "A recovered history read kept its old error visible.");
    assert(row.isConnected && document.activeElement === copy, "A successful refresh replaced the focused result.");
  } finally { client.invoke = original; }
}

export async function checkDictationReadinessActions() {
  const root = await owned();
  await until(() => root.dataset.dictationReadiness !== "checking", "Readiness did not settle.");
  const repair = root.querySelector(".dictation-home-repair"), practice = root.querySelector(".dictation-home-practice");
  const blocked = root.dataset.dictationReadiness === "blocked";
  assert(blocked ? visible(repair) && repair.classList.contains("primary") && practice.disabled
    : visible(practice) && practice.classList.contains("primary") && !practice.disabled,
  "The primary action does not match recording readiness.");
  if (!blocked) {
    assert(root.querySelector(".dictation-home-intro").textContent.length < 110, "Ready Dictation still repeats the full first-use instructions.");
    assert(!root.querySelector(".dictation-processing-details").open, "Routine processing detail is expanded before it is requested.");
  }
}
