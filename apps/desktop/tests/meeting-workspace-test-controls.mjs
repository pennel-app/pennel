const assert = (value, message) => { if (!value) throw Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 240; attempt++) { if (check()) return; await new Promise(resolve => setTimeout(resolve, 25)); }
  throw Error(message);
}

export function selectMeetingPane(root, pane) {
  if (root.querySelector(".meeting-document") && (pane === "notes" || pane === "summary")) {
    const document = root.querySelector(".meeting-document"), section = root.querySelector(`[data-live-pane="${pane}"]`);
    if (document.hidden || root.classList.contains("live-overlay")) root.querySelector(".meeting-companion-close").click();
    assert(section && !section.hidden && !document.hidden, `The ${pane} document section is unavailable.`);
    document.scrollTop += section.getBoundingClientRect().top - document.getBoundingClientRect().top;
    return;
  }
  const button = root.querySelector(`[data-live-tab="${pane}"]`);
  assert(button && !button.hidden && button.getClientRects().length, `The ${pane} tab is not a visible meeting control.`);
  const panel = root.querySelector(`[data-live-pane="${pane}"]`);
  if (button.getAttribute("aria-selected") !== "true" || panel?.hidden) button.click();
}

export async function openMeetingMore(root, key) {
  const { t } = await import("/src/i18n.ts");
  if (root.classList.contains("live-overlay")) root.querySelector(".meeting-companion-close").click();
  const trigger = root.querySelector(".meeting-workspace-more");
  assert(trigger?.getClientRects().length, "The meeting More control is not visible.");
  trigger.click();
  const entry = () => [...document.querySelectorAll('.choice-list [role="menuitem"]')].find(button => button.textContent.trim() === t(key));
  await until(() => entry()?.getClientRects().length, `The More menu does not expose ${key}.`);
  entry().click();
}

export async function openMeetingTools(root, section) {
  const key = { people: "meetings.workspace_people", speakers: "meetings.workspace_speaker_controls", template: "meetings.summary_template_change",
    automation: "meetings.summary_automation_change", transcription: "meetings.retranscribe_audio", files: "meetings.workspace_file_actions" }[section];
  assert(key, `Unknown meeting tool ${section}.`);
  await openMeetingMore(root, key);
  await until(() => root.querySelector(`[data-meeting-tool="${section}"]`)?.getClientRects().length, `The ${section} tools are not visible.`);
}

/** Synthetic transaction during an actual contentDOM composition lifecycle; not physical IME evidence. */
export function replaceNotesSelection(editor, text) {
  const start = editor.selectionStart, end = editor.selectionEnd;
  editor.applyUserChange(editor.value.slice(0, start) + text + editor.value.slice(end), { start, end: start + text.length });
}
