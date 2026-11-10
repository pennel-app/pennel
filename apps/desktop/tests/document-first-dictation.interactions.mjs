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
    assert(copy.dataset.copied === 'true', 'Copy did not confirm in the result row.');
    await new Promise(resolve => setTimeout(resolve, 900));
    assert(copy.dataset.copied === 'true', 'Copy confirmation disappeared before its 1.2-second feedback interval.');
    await until(() => !copy.dataset.copied, 'Copy confirmation never returned to Copy.');
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
    historyFail = false; root.querySelector('.recent-load-status button').click();
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
    assert(root.querySelector(".dictation-home-intro").textContent.length < 170, "Ready Dictation still repeats the full first-use instructions.");
    assert(root.querySelectorAll(".home-processing-chip").length >= 2, "Speech and cleanup destinations must stay visible.");
  }
}

export async function checkHomePractice() {
  const root = await owned(), client = window.__PENNEL_PLATFORM__;
  const field = root.querySelector('.home-practice-field');
  assert(field, 'Home practice needs its own editable field.');
  const original = client.invoke.bind(client), calls = [];
  let failure = false, ownerId;
  client.invoke = async (command, args = {}) => {
    calls.push({command, args});
    if (failure && command === 'toggle_dictation_session') throw Error('Synthetic start failed; try again.');
    const result = await original(command, args);
    if (command === 'open_dictation_session') ownerId = result.session_id;
    return result;
  };
  try {
    const before = await original('app_info'), count = (await original('history', {limit:200})).length;
    root.querySelector('.dictation-home-practice').click();
    await until(() => document.activeElement === field, 'Try it here did not focus the home message.');
    assert(!calls.some(call => call.command === 'show_window' || call.command === 'toggle_dictation_session'), 'Try it here started capture or opened onboarding.');
    assert(calls.find(call => call.command === 'open_dictation_session')?.args.kind === 'home_practice', 'Home opened the wrong session kind.');
    field.value = 'Editable message:'; field.dispatchEvent(new Event('input', {bubbles:true}));
    failure = true; root.querySelector('.home-practice-start').click();
    await until(() => root.querySelector('.home-practice-status').textContent.includes('Synthetic start failed'), 'Start failure was swallowed.');
    failure = false; root.querySelector('.dictation-home-hands-free').click();
    await until(() => field.disabled && !root.querySelector('.home-practice-start').disabled, 'Hands-free did not start owned capture.');
    root.querySelector('[data-action="all-history"]').click();
    await until(() => root.querySelector('.home-practice-status').textContent.match(/Finish|Rond/), 'Busy navigation did not explain finishing or cancelling.');
    assert(root.isConnected && field.disabled, 'Busy navigation disposed the active home capture.');
    failure = true; root.querySelector('.home-practice-start').click();
    await until(() => root.querySelector('.home-practice-status').textContent.includes('Synthetic start failed'), 'Finish failure was swallowed.');
    assert(field.disabled && root.isConnected, 'Finish failure detached the active capture.');
    failure = false;
    root.querySelector('.home-practice-start').click();
    if (window.__PENNEL_FIXTURE__.options.owner_review) {
      await until(async () => (await original('get_dictation_session', {sessionId: ownerId})).phase === 'working', 'Finishing never reached processing.');
      assert(field.disabled && root.querySelector('.home-practice-start').disabled, 'Processing can be restarted or edited.');
      root.querySelector('[data-action="all-history"]').click();
      await until(() => root.querySelector('.home-practice-status').textContent.match(/Finish|Rond/), 'Processing navigation did not preserve its owner.');
    }
    await until(() => !field.disabled && field.value.includes('Editable message: '), 'Completed dictation did not retain the editable draft.');
    assert(field.value.length > 'Editable message: '.length, 'No recognized result reached the field.');
    assert((await original('history', {limit:200})).length === count + 1, 'Home result did not enter ordinary History.');
    await until(() => root.querySelector('.recent-dictation-new'), 'New home result did not enter Recent with feedback.');
    assert(JSON.stringify((await original('app_info')).settings.onboarding) === JSON.stringify(before.settings.onboarding), 'Home practice changed onboarding progress.');
    assert(!calls.some(call => ['copy_text','toggle_dictation','show_window','save_pad_text'].includes(call.command)), 'Home capture used an external, clipboard, onboarding or persistent pad route.');
    const value = field.value;
    root.querySelector('.dictation-home-hands-free').click();
    await until(() => field.disabled, 'Second capture did not begin.');
    root.querySelector('.home-practice-cancel').click();
    await until(() => !field.disabled, 'Cancel did not release home capture.');
    assert(field.value === value, 'Cancel discarded the retained message.');
    root.querySelector('[data-action="all-history"]').click();
    await until(() => !root.isConnected, 'Idle home could not leave.');
    document.querySelector('[data-workspace-section="dictation"]').click();
    await until(() => document.querySelector('.home-practice-field')?.value === value, 'Returning home lost its editable draft.');
  } finally { client.invoke = original; }
}

export async function checkHomeEmptyRecent() {
  const root = await owned(), client = window.__PENNEL_PLATFORM__, original = client.invoke.bind(client);
  client.invoke = (command, args) => command === 'history' ? Promise.resolve([]) : original(command, args);
  try {
    window.__PENNEL_FIXTURE__.emit('history-changed', null);
    await until(() => root.querySelector('.recent-dictations-empty'), 'An empty history has no explanation.');
    assert(!root.querySelector('[data-recent-id]'), 'An empty history kept stale rows.');
    assert(visible(root.querySelector('[data-action="all-history"]')), 'Empty Recent lost the History recovery route.');
  } finally { client.invoke = original; }
}
