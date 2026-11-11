const assert = (value, message) => { if (!value) throw Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 320; attempt++) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw Error(message);
}
const edit = (input, value) => {
  input.value = value;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
};

export async function checkMeetingTitleRefresh() {
  assert(window.__PENNEL_FIXTURE__ && /^(127\.0\.0\.1|localhost)$/.test(location.hostname), "Use the owned fixture.");
  await until(() => document.querySelector(".live-workspace .live-title"), "The meeting did not open.");
  const root = document.querySelector(".live-workspace"), title = root.querySelector(".live-title");
  const notes = root.querySelector(".live-notes"), client = window.__PENNEL_PLATFORM__;
  const invoke = client.invoke.bind(client), original = title.value, id = root.dataset.meetingId;
  let generated = "Planning van de volgende release", read = false;
  client.invoke = async (command, args) => {
    const value = await invoke(command, args);
    if (command === "meeting_detail" && args.id === id) {
      read = true;
      return { ...value, info: { ...value.info, title: generated } };
    }
    if (command === "rename_meeting" && args.id === id) generated = args.title;
    return value;
  };
  try {
    const { getRetainedMeetingWorkspace } = await import("/src/live-workspace.ts");
    assert(original !== generated, "The fixture does not contain a title update.");
    const retained = notes.value;
    await until(() => read && title.value === generated, "The generated meeting title did not reach the retained header.");
    assert(root === getRetainedMeetingWorkspace(id) && root.querySelector(".live-notes") === notes && notes.value === retained,
      "A title update replaced the retained meeting or notes.");
    title.focus(); edit(title, "Mijn eigen meetingnaam");
    await until(() => root.querySelector(".live-save-status")?.dataset.saveState === "saved" || generated === "Mijn eigen meetingnaam",
      "The manual title edit was not saved.");
    assert(title.value === "Mijn eigen meetingnaam", "A background refresh overwrote the active title edit.");
    return { generatedTitleRefreshed: true, retainedWorkspaceAndNotes: true, manualRenameAvailable: true, syntheticOnly: true };
  } finally { client.invoke = invoke; }
}

export async function checkConversationTitleRefresh() {
  assert(window.__PENNEL_FIXTURE__ && /^(127\.0\.0\.1|localhost)$/.test(location.hostname), "Use the owned fixture.");
  await until(() => document.querySelector(".conversation-panel .conversation-input:not(:disabled)"), "Ask did not open.");
  const root = document.querySelector(".conversation-panel"), picker = root.querySelector(".conversation-select");
  const input = root.querySelector(".conversation-input"), history = root.querySelector(".chat-history");
  const client = window.__PENNEL_PLATFORM__, invoke = client.invoke.bind(client);
  const generated = "Beslissingen over de release";
  client.invoke = async (command, args) => {
    const value = await invoke(command, args);
    if (command !== "conversation_snapshot") return value;
    const id = value.conversation.id;
    return { ...value, conversation: { ...value.conversation, title: generated },
      conversations: value.conversations.map(item => item.id === id ? { ...item, title: generated } : item) };
  };
  try {
    input.focus();
    edit(input, "Een bewaard concept blijft staan.");
    input.setSelectionRange(4, 12, "backward");
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    await until(() => picker.textContent === generated, "The generated conversation title was not refreshed.");
    assert(root.querySelector(".conversation-input") === input && root.querySelector(".chat-history") === history
      && document.activeElement === input && input.value === "Een bewaard concept blijft staan."
      && input.selectionStart === 4 && input.selectionEnd === 12 && input.selectionDirection === "backward",
      "The title refresh lost the question draft, composition, focus or selection.");
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    return { generatedTitleRefreshed: true, draftAndCompositionPreserved: true, syntheticOnly: true };
  } finally { client.invoke = invoke; }
}
