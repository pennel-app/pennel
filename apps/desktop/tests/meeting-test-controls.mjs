const pause = () => new Promise(resolve => setTimeout(resolve, 25));

/** Exercise the visible contextual route instead of clicking a control in a hidden pane. */
export async function openMeetingTool(key) {
  const { t } = await import("/src/i18n.ts");
  const more = document.querySelector(".meeting-workspace-more");
  if (!more || !more.getClientRects().length) throw Error("The meeting More action is unavailable.");
  more.click();
  for (let attempt = 0; attempt < 200; attempt++) {
    const option = [...document.querySelectorAll('[role="menuitem"]')].find(item => item.textContent.trim() === t(key));
    if (option) { option.click(); await pause(); return; }
    await pause();
  }
  throw Error(`The meeting tool ${key} is unavailable.`);
}
