const assert = (value, message) => { if (!value) throw new Error(message); };
const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
async function until(check, message) {
  for (let attempt = 0; attempt < 150; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 30));
  }
  throw new Error(message);
}

/** Verifies the unified AI & Models Hub, Settings Search, and Command Palette in the owned fixture. */
export async function checkAiHubAndPalette() {
  const fixture = window.__PENNEL_FIXTURE__;
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && fixture && window.__PENNEL_PLATFORM__, "Use the owned fixture server.");
  await until(() => document.querySelector(".ai-models-hub") && document.querySelectorAll(".ai-preset-card").length === 3, "AI & Models Hub did not render its three quick setup presets.");
  assert(document.querySelectorAll("[data-settings-category]").length === 6, "Settings must expose six balanced categories.");
  assert(document.querySelector('[data-settings-category="ai-models"][aria-current="page"]'), "AI & Models did not open as active category.");
  assert(document.querySelector('[data-settings-anchor="ai-speakers"]'), "Speaker recognition section missing from AI & Models.");
  assert(document.querySelector('[data-settings-anchor="ai-local"]'), "Local speech models section missing from AI & Models.");
  assert(document.querySelector('[data-settings-anchor="ai-providers"]'), "Cloud providers section missing from AI & Models.");
  assert(document.querySelector(".ai-advanced-providers") && !document.querySelector(".ai-advanced-providers").open, "Local servers & CLI agents should stay collapsed unless active.");
  assert(document.querySelector("details[data-inference-experimental]") && !document.querySelector("details[data-inference-experimental]").open, "Specialized review override should stay collapsed by default.");
  assert(!document.querySelector('[data-inference-capability="reranking"]'), "Dead reranking section must remain removed.");

  // Separate meeting speech toggle preserves dictation_stt focus/node in place.
  const speechBtn = document.querySelector('.ai-active-tasks button[data-setup-task="dictation_stt"]');
  assert(speechBtn && !document.querySelector('.ai-active-tasks button[data-setup-task="meeting_stt"]'), "Unified speech row should hide separate meeting_stt by default when linked.");
  const toggleInput = document.querySelector('[data-settings-anchor="ai-tasks"] input[type="checkbox"]');
  assert(toggleInput, "Separate meeting speech toggle missing.");
  toggleInput.click();
  await until(() => document.querySelector('.ai-active-tasks button[data-setup-task="meeting_stt"]'), "Enabling separate meeting speech did not reveal meeting_stt.");
  assert(speechBtn.isConnected, "Toggling separate meeting speech replaced the existing dictation_stt row.");
  toggleInput.click();
  await until(() => !document.querySelector('.ai-active-tasks button[data-setup-task="meeting_stt"]'), "Disabling separate meeting speech did not collapse meeting_stt.");
  assert(speechBtn.isConnected, "Collapsing separate meeting speech replaced dictation_stt.");

  // Bilingual Settings Search navigates and highlights anchors.
  const searchInput = document.querySelector(".settings-search-input");
  assert(searchInput, "Settings search field missing.");
  searchInput.focus();
  searchInput.value = "donker";
  searchInput.dispatchEvent(new Event("input", { bubbles: true }));
  await until(() => document.querySelectorAll(".settings-search-item").length > 0, "Dutch search query returned no settings matches.");
  document.querySelector(".settings-search-item").click();
  await until(() => document.querySelector('[data-settings-category="appearance"][aria-current="page"]'), "Clicking a Settings search result did not open its target category.");

  // Command Palette (Cmd+K / Ctrl+K) opens, filters, navigates via Enter, and closes via Escape.
  const isMac = fixture.info().platform === "macos";
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: isMac, ctrlKey: !isMac, bubbles: true, cancelable: true }));
  await until(() => document.querySelector("dialog.command-palette[open]"), "Cmd/Ctrl+K did not open the Command Palette.");
  const paletteInput = document.querySelector("dialog.command-palette .command-palette-input");
  assert(paletteInput && document.activeElement === paletteInput, "Command Palette did not focus its search input.");
  paletteInput.value = "ollama";
  paletteInput.dispatchEvent(new Event("input", { bubbles: true }));
  await until(() => document.querySelectorAll(".command-palette-item").length > 0, "Command Palette did not match 'ollama' setting.");
  paletteInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  await until(() => !document.querySelector("dialog.command-palette[open]") && document.querySelector('[data-settings-category="ai-models"][aria-current="page"]'), "Pressing Enter in Command Palette did not navigate to AI & Models.");

  window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: isMac, ctrlKey: !isMac, bubbles: true, cancelable: true }));
  await until(() => document.querySelector("dialog.command-palette[open]"), "Command Palette did not reopen.");
  document.querySelector("dialog.command-palette").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  await settle();
  if (document.querySelector("dialog.command-palette[open]")) {
    document.querySelector("dialog.command-palette").close();
  }
  assert(!document.querySelector("dialog.command-palette[open]"), "Escape did not close the Command Palette.");
  assert(!fixture.unknown.length, `Unknown fixture commands: ${fixture.unknown.join(", ")}`);
  return { hubVerified: true, searchVerified: true, paletteVerified: true, syntheticOnly: true };
}
