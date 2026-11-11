const assert = (value, message) => { if (!value) throw new Error(message); };
const pause = (ms = 25) => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let attempt = 0; attempt < 250; attempt++) { if (await check()) return; await pause(); }
  throw new Error(message);
}
async function owned() {
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && window.__PENNEL_FIXTURE__, "Use the owned synthetic fixture.");
  await until(() => document.querySelector(".meeting-export-action:not([hidden])"), "A finished meeting is required for export review.");
  return { root: document.querySelector(".live-workspace"), client: window.__PENNEL_PLATFORM__, fixture: window.__PENNEL_FIXTURE__ };
}
function record(name, result) { window.__PENNEL_LIVE_ACCEPTANCE__ ??= {}; window.__PENNEL_LIVE_ACCEPTANCE__[name] = result; return result; }
const settled = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
function visibleBounds(node) {
  const bounds = { top: 0, left: 0, right: innerWidth, bottom: innerHeight };
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    const style = getComputedStyle(parent), rect = parent.getBoundingClientRect();
    if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) {
      bounds.left = Math.max(bounds.left, rect.left + parent.clientLeft);
      bounds.right = Math.min(bounds.right, rect.left + parent.clientLeft + parent.clientWidth);
    }
    if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) {
      bounds.top = Math.max(bounds.top, rect.top + parent.clientTop);
      bounds.bottom = Math.min(bounds.bottom, rect.top + parent.clientTop + parent.clientHeight);
    }
  }
  return bounds;
}
async function checkReachable(node, label, focus = true) {
  assert(node && !node.closest("[hidden]") && !node.disabled, `${label} is unavailable.`);
  if (focus) { assert(node.tabIndex >= 0, `${label} is not keyboard reachable.`); node.focus({ preventScroll: true }); }
  node.scrollIntoView({ block: "nearest", inline: "nearest" }); await settled();
  const rect = node.getBoundingClientRect(), bounds = visibleBounds(node);
  assert(rect.width > 0 && rect.height > 0 && rect.top >= bounds.top - 1 && rect.bottom <= bounds.bottom + 1
    && rect.left >= bounds.left - 1 && rect.right <= bounds.right + 1,
  `${label} is clipped after scrolling: ${JSON.stringify({ rect: { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right }, bounds })}.`);
  const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
  assert(hit && (hit === node || node.contains(hit)), `${label} is covered by another surface.`);
  if (focus) assert(document.activeElement === node, `${label} could not receive keyboard focus.`);
}
async function checkExportViewport(preview) {
  const review = preview.querySelector(".meeting-export-review"), formatted = preview.querySelector(".meeting-export-rendered");
  assert(review.clientHeight >= 64, `The export review has no usable reading viewport: ${review.clientHeight}px.`);
  for (const [selector, label] of [
    ['[data-export-scope="shareable"]', "Shareable scope"], ['[data-export-scope="personal"]', "Personal scope"],
    ['[data-export-view="formatted"]', "Formatted preview choice"], ['[data-export-view="markdown"]', "Markdown preview choice"],
  ]) await checkReachable(preview.querySelector(selector), label);
  const passage = formatted.querySelector(".markdown-heading, p");
  await checkReachable(passage, "Prepared document passage", false);
  for (const [selector, label] of [
    [".meeting-export-copy-formatted", "Copy formatted"], [".meeting-export-copy-markdown", "Copy Markdown"], [".meeting-export-save", "Save Markdown"],
  ]) await checkReachable(preview.querySelector(selector), label);
  const geometry = { readingHeight: review.clientHeight, actionsHeight: preview.querySelector(".meeting-export-actions").getBoundingClientRect().height };
  preview.scrollTop = 0; review.scrollTop = 0;
  const pane = preview.closest(".live-pane"); if (pane) pane.scrollTop = 0;
  await settled();
  return geometry;
}

export async function checkFormattedExportPreview() {
  const { root, client, fixture } = await owned();
  const original = client.invoke.bind(client), operations = [];
  const shareable = {
    title: "Synthetic shared result",
    markdown: "# Synthetic shared result\n\n- Decision: **Keep the reviewed text**.\n\n<img src=x onerror=\"globalThis.__PENNEL_EXPORT_UNSAFE__=true\">",
    html: "<h1>Synthetic shared result</h1><p>Decision: <strong>Keep the reviewed text</strong>.</p><script>globalThis.__PENNEL_EXPORT_UNSAFE__=true</script>",
  };
  const personal = { title: "Synthetic personal result", markdown: "# Personal result\n\nPRIVATE_EXPORT_SENTINEL", html: "<h1>Personal result</h1><p>PRIVATE_EXPORT_SENTINEL</p>" };
  client.invoke = async (command, args = {}) => {
    operations.push({ command, args: structuredClone(args) });
    if (command === "preview_meeting_export") return structuredClone(args.scope === "personal" ? personal : shareable);
    if (command === "copy_text") return;
    if (command === "export_prepared_meeting") return true;
    return original(command, args);
  };
  try {
    root.querySelector(".meeting-export-action").click();
    const preview = document.querySelector(".meeting-export-preview"), body = preview.querySelector(".meeting-export-copy");
    assert(preview.closest("dialog")?.open, "Share review must open a modal dialog without replacing the meeting workspace.");
    const formatted = preview.querySelector(".meeting-export-rendered");
    await until(() => preview.dataset.exportView === "formatted" && formatted && !formatted.hidden && body.value === shareable.markdown,
      "Export review did not default to the formatted prepared document.");
    assert(body.hidden && formatted.querySelector("strong")?.textContent === "Keep the reviewed text", "Formatted review did not render Markdown structure.");
    assert(!formatted.querySelector("script,img,iframe,a") && !globalThis.__PENNEL_EXPORT_UNSAFE__, "Untrusted export content became active HTML.");
    assert(formatted.textContent.includes("<img") && !formatted.textContent.includes("PRIVATE_EXPORT_SENTINEL"), "The formatted document changed output scope or omitted literal text.");
    const preparedCalls = operations.filter(value => value.command === "preview_meeting_export").length;
    preview.querySelector('[data-export-view="markdown"]').click();
    assert(!body.hidden && formatted.hidden && body.value === shareable.markdown, "Markdown view did not preserve the exact prepared text.");
    body.focus(); body.setSelectionRange(2, 16); body.scrollTop = 12;
    const selection = { start: body.selectionStart, end: body.selectionEnd, scroll: body.scrollTop };
    preview.querySelector('[data-export-view="formatted"]').click();
    preview.querySelector('[data-export-view="markdown"]').click();
    assert(body.selectionStart === selection.start && body.selectionEnd === selection.end && body.scrollTop === selection.scroll,
      "Changing preview format replaced Markdown selection or scroll.");
    assert(operations.filter(value => value.command === "preview_meeting_export").length === preparedCalls, "Changing presentation prepared new output.");
    preview.querySelector(".meeting-export-copy-formatted").click();
    await until(() => operations.some(value => value.command === "copy_text"), "Formatted copy was not reachable.");
    const copied = operations.find(value => value.command === "copy_text").args;
    assert(copied.text === shareable.markdown && copied.html === shareable.html, "Formatted copy changed the reviewed immutable payload.");
    await until(() => !preview.querySelector(".meeting-export-save").disabled, "Export actions did not recover after copying.");
    preview.querySelector(".meeting-export-save").click();
    await until(() => operations.some(value => value.command === "export_prepared_meeting"), "Markdown export was not reachable.");
    const saved = operations.find(value => value.command === "export_prepared_meeting").args.output;
    assert(JSON.stringify(saved) === JSON.stringify(shareable), "Saving changed the reviewed immutable payload.");
    await until(() => !preview.querySelector('[data-export-scope="personal"]').disabled, "Output scope remained disabled after saving.");
    preview.querySelector('[data-export-scope="personal"]').click();
    await until(() => body.value === personal.markdown, "Explicit personal output was not prepared.");
    preview.querySelector('[data-export-view="formatted"]').click();
    assert(formatted.textContent.includes("PRIVATE_EXPORT_SENTINEL"), "Explicit personal preview did not show its own prepared output.");
    assert(!operations.some(value => /generate_.*summary/.test(value.command)), "Opening or formatting export silently invoked generation.");
    const footer = preview.querySelector(".meeting-export-actions");
    assert(footer && getComputedStyle(footer).flexShrink === "0", "Export actions do not have a stable footer.");
    const geometry = await checkExportViewport(preview);
    return record("formattedExportPreview", { formattedDefault: true, safeRendering: true, exactPayload: true, explicitScope: true, noImplicitGeneration: true, viewportReachable: true, geometry, syntheticOnly: true });
  } finally { client.invoke = original; delete globalThis.__PENNEL_EXPORT_UNSAFE__; }
}

export async function checkEnlargedFormattedExportPreview() {
  await checkFormattedExportPreview();
  const sizes = [...document.querySelectorAll("#app *, dialog[open] *")].filter(node => node instanceof HTMLElement)
    .map(node => [node, parseFloat(getComputedStyle(node).fontSize)]);
  for (const [node, size] of sizes) { node.style.fontSize = `${size * 2}px`; node.style.lineHeight = "1.4"; }
  await settled();
  const geometry = await checkExportViewport(document.querySelector(".meeting-export-preview"));
  return record("enlargedFormattedExportPreview", { enlargedText: true, viewportReachable: true, geometry, syntheticOnly: true });
}

export async function checkExportPreviewLatePreparation() {
  const { root, client } = await owned();
  const original = client.invoke.bind(client);
  let rejectEarlier, requests = 0;
  const earlier = new Promise((_, reject) => { rejectEarlier = reject; });
  const latest = { title: "Latest output", markdown: "# Latest output\n\nThe reviewed current document.", html: "<h1>Latest output</h1><p>The reviewed current document.</p>" };
  client.invoke = async (command, args = {}) => {
    if (command === "preview_meeting_export") { requests++; return requests === 1 ? earlier : structuredClone(latest); }
    return original(command, args);
  };
  try {
    root.querySelector(".meeting-export-action").click();
    await until(() => requests === 1, "The first preparation did not start.");
    root.querySelector(".meeting-export-action").click();
    const preview = document.querySelector(".meeting-export-preview"), body = preview.querySelector(".meeting-export-copy");
    await until(() => body.value === latest.markdown && !preview.querySelector(".meeting-export-copy-markdown").disabled, "The newer preparation did not finish.");
    rejectEarlier(new Error("Synthetic stale preparation failure"));
    await pause(50);
    assert(body.value === latest.markdown && !preview.textContent.includes("Synthetic stale preparation failure")
      && !preview.querySelector(".meeting-export-copy-markdown").disabled, "A late preparation failure replaced newer reviewed output.");
    return record("exportPreviewLatePreparation", { staleFailureIgnored: true, latestPreserved: true, syntheticOnly: true });
  } finally { rejectEarlier?.(new Error("Synthetic cleanup")); client.invoke = original; }
}

export async function checkExportDialogLifecycle() {
  const { root, client } = await owned();
  const locale = await import("/src/i18n.ts");
  const language = locale.getInterfaceLanguage();
  const invoke = client.invoke.bind(client), invoker = root.querySelector(".meeting-export-action");
  const editor = root.querySelector(".live-notes");
  editor?.setSelectionRange(2, Math.min(10, editor.value.length));
  const selection = editor ? [editor.value, editor.selectionStart, editor.selectionEnd] : null;
  let resolveOld, requests = 0;
  const old = new Promise(resolve => { resolveOld = resolve; });
  const latest = { title: "Current immutable preview", markdown: "# Current immutable preview\n\nLiteral <owner-data>.", html: "<h1>Current immutable preview</h1><p>Literal &lt;owner-data&gt;.</p>" };
  client.invoke = (command, args) => command === "preview_meeting_export"
    ? (++requests === 1 ? old : Promise.resolve(structuredClone(latest))) : invoke(command, args);
  try {
    invoker.focus(); invoker.click();
    await until(() => requests === 1 && document.querySelector("dialog[open] .meeting-export-preview"), "Share did not open its own dialog.");
    document.querySelector(".meeting-export-cancel").click();
    assert(!document.querySelector("dialog[open]") && document.activeElement === invoker, "Cancel did not restore the original Share action.");
    if (selection) assert(editor.value === selection[0] && editor.selectionStart === selection[1] && editor.selectionEnd === selection[2], "Share changed the retained note selection.");
    locale.setInterfaceLanguage(language === "en" ? "nl" : "en");
    invoker.click();
    await until(() => document.querySelector(".meeting-export-copy")?.value === latest.markdown, "Reopened Share did not prepare the current payload.");
    const preview = document.querySelector("dialog[open] .meeting-export-preview");
    assert(preview.querySelector('[data-export-scope="shareable"]').textContent === locale.t("meetings.export_shareable"), "Reopened dialog kept detached labels in the previous language.");
    resolveOld({ title: "STALE", markdown: "STALE_PRIVATE_OUTPUT", html: "<p>STALE_PRIVATE_OUTPUT</p>" });
    await pause(50);
    assert(preview.querySelector(".meeting-export-copy").value === latest.markdown && !preview.textContent.includes("STALE_PRIVATE_OUTPUT"), "A closed preparation replaced the reopened preview.");
    assert(document.querySelectorAll("dialog[open]").length === 1, "Share created competing dialogs.");
    preview.querySelector(".meeting-export-cancel").click();
    return record("exportDialogLifecycle", { modal: true, focusAndSelection: true, closedPreparationIgnored: true, relocalizedReopen: true, syntheticOnly: true });
  } finally {
    resolveOld?.(latest); document.querySelector(".meeting-export-cancel")?.click();
    client.invoke = invoke; locale.setInterfaceLanguage(language);
  }
}

/** Layout-only review evidence, shared verbatim by before and after renders. */
export async function renderExportReference() {
  const { root, client, fixture } = await owned();
  const original = client.invoke.bind(client);
  const payload = { title: "Synthetic planning", markdown: "# Summary\n\nThe team confirmed next week's planning and agreed who prepares what.\n\n## Decisions\n\n- The planning is confirmed for Monday.\n- The client call moves to Thursday afternoon.\n\n## Action items\n\n- Share the agenda before Friday.\n- Confirm the next appointment.\n- Book a room for the client call.", html: "<h1>Summary</h1><p>Synthetic review payload.</p>" };
  client.invoke = async (command, args) => command === "preview_meeting_export"
    ? { ...payload, markdown: payload.markdown + (args.scope === "personal" ? "\n\n## Your notes\n\nPrivate synthetic preparation notes." : "") }
    : original(command, args);
  try {
    root.querySelector('.meeting-export-action').click();
    const preview = document.querySelector('.meeting-export-preview');
    await until(() => !preview.querySelector('.meeting-export-copy-markdown').disabled, "Review payload did not load.");
    if (fixture.options.export_reference_personal) {
      preview.querySelector('[data-export-scope="personal"]').click();
      await until(() => preview.querySelector('.meeting-export-copy').value.includes('Private synthetic preparation'), "Personal review payload did not load.");
    }
    if (fixture.options.export_reference_enlarged) {
      const sizes=[...document.querySelectorAll('#app *, dialog[open] *')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);
      for(const [node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}
    }
    await settled();
  } finally { client.invoke = original; }
}
