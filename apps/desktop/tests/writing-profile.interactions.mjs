const assert = (condition, message) => { if (!condition) throw Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 160; attempt++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); }
  throw Error(message);
}

/** Uses the owned synthetic desktop seam; no recording, real account or model is accessed. */
export async function checkWritingProfile() {
  const client = window.__PENNEL_PLATFORM__, fixture = window.__PENNEL_FIXTURE__;
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && client && fixture, "Use the owned writing-profile fixture.");
  const panel = () => document.querySelector(".writing-profile-settings");
  const control = key => [...panel().querySelectorAll("[data-writing-control]")].find(node => node.dataset.writingControl === key);
  const snapshot = () => client.invoke("writing_profile_status");
  await until(() => control("trait:en:directness"), "Writing profile did not finish loading.");
  assert((await snapshot()).languages.every(language => language.status === "ready"), "Prepared language evidence did not start ready.");

  const choose = async (value, index) => {
    const choice = control("trait:en:directness");
    choice.focus(); choice.click();
    await until(() => document.querySelectorAll('.choice-list [role="option"]').length === 4, "Trait choices did not open.");
    document.querySelectorAll('.choice-list [role="option"]')[index].click();
    await until(() => control("trait:en:directness")?.value === value && !panel().querySelector(':disabled[data-writing-control="refresh"]'), "Trait change did not save.");
  };
  await choose("indirect", 3);
  assert(document.activeElement === control("trait:en:directness"), "Saving a trait lost keyboard focus.");
  assert((await snapshot()).languages.find(language => language.language === "en").traits.find(trait => trait.key === "directness").overridden, "The correction was not an explicit override.");

  control("refresh").focus(); control("refresh").click();
  await until(() => !control("refresh").disabled, "Refresh did not finish.");
  assert((await snapshot()).languages.find(language => language.language === "en").traits.find(trait => trait.key === "directness").value === "indirect", "Refresh replaced a user correction.");

  panel().querySelector(".writing-profile-sources").open = true;
  const first = (await snapshot()).sources[0];
  control(`source:${first.id}`).querySelector("input").click();
  await until(async () => (await snapshot()).sources[0].excluded && !control("refresh").disabled, "Source exclusion did not save.");
  assert((await snapshot()).languages.find(language => language.language === "en").status === "insufficient_evidence", "Exclusion left stale ready evidence available.");

  control("learning").querySelector("input").click();
  await until(async () => !(await snapshot()).learning_enabled && !control("learning").querySelector("input").disabled, "Learning did not pause.");
  assert((await snapshot()).sources.length === 6, "Pause erased source evidence.");
  control(`source:${first.id}`).querySelector("input").click();
  await until(async () => !(await snapshot()).sources[0].excluded, "A paused profile could not remove a source exclusion.");

  control("sharing").querySelector("input").click();
  await until(async () => !(await client.invoke("writing_profile_connection")).enabled, "Sharing did not switch off independently.");
  assert(!fixture.calls.includes("set_claude_access"), "Style controls changed meeting access.");

  control("delete").click();
  await until(() => document.querySelector("dialog[open]"), "Profile deletion did not request confirmation.");
  document.querySelector("dialog[open] .dialog-actions button").click();
  assert((await snapshot()).sources.length === 6, "Cancelling deletion removed evidence.");
  control("delete").click();
  await until(() => document.querySelector("dialog[open]"), "Profile deletion confirmation did not reopen.");
  document.querySelector("dialog[open] .dialog-actions button.danger").click();
  await until(async () => !(await snapshot()).sources.length && !control("learning").querySelector("input").disabled, "Confirmed deletion did not clear the profile.");
  const deleted = await snapshot();
  assert(!deleted.learning_enabled && !deleted.sharing_enabled, "Deletion did not stop learning and sharing.");
  assert(!fixture.calls.some(command => ["delete_meeting", "delete_history", "start_meeting", "toggle_dictation"].includes(command)), "Writing-profile controls changed original speech or started capture.");
  assert(!fixture.unknown.length, `Unknown fixture commands: ${fixture.unknown.join(", ")}`);
  return { correctedTrait: true, keyboardFocusPreserved: true, sourceInvalidation: true, pauseRetainsEvidence: true, sharingIndependent: true, confirmedDeletion: true };
}
