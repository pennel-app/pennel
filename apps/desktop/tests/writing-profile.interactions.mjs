const assert = (condition, message) => { if (!condition) throw Error(message); };
async function until(check, message) {
  for (let attempt = 0; attempt < 160; attempt++) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 25)); }
  throw Error(message);
}

/** Uses the owned synthetic desktop seam; no recording, real account or model is accessed. */
export async function checkWritingProfile({ production = false } = {}) {
  const client = window.__PENNEL_PLATFORM__, fixture = window.__PENNEL_FIXTURE__;
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && client && fixture, "Use the owned writing-profile fixture.");
  const panel = () => document.querySelector(".writing-profile-settings");
  const control = key => [...panel()?.querySelectorAll("[data-writing-control]") || []].find(node => node.dataset.writingControl === key);
  const snapshot = () => client.invoke("writing_profile_status");
  await until(() => control("trait:en:directness"), "Writing profile did not finish loading.");
  if (production) {
    assert((await client.invoke("app_info")).preview === false, "Use the production desktop fixture.");
    assert(control("learning").getBoundingClientRect().height > 0 && control("sharing").getBoundingClientRect().height > 0, "Production learning or sharing controls are hidden.");
    assert(panel().querySelector(".writing-profile-url")?.textContent === "https://api.pennel.app/mcp/style", "Production profile setup did not expose the production connector.");
  }
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

/** Verifies first-use consent in the ordinary desktop before any source evidence exists. */
export async function checkProductionWritingProfileConsent() {
  const client = window.__PENNEL_PLATFORM__, fixture = window.__PENNEL_FIXTURE__;
  assert(/^(127\.0\.0\.1|localhost)$/.test(location.hostname) && client && fixture, "Use the owned writing-profile fixture.");
  assert((await client.invoke("app_info")).preview === false, "Use the production desktop fixture.");
  const panel = () => document.querySelector(".writing-profile-settings");
  const control = key => panel()?.querySelector(`[data-writing-control="${key}"] input`);
  const snapshot = () => client.invoke("writing_profile_status");
  await until(() => control("learning") && control("sharing"), "Production writing-profile controls did not load.");
  const initial = await snapshot();
  assert(!initial.learning_enabled && !initial.sharing_enabled && initial.started_at_ms === null && !initial.sources.length, "Production learning or sharing was enabled before consent.");
  assert(!control("learning").checked && !control("sharing").checked, "Production controls did not show the opt-in defaults.");
  assert(!fixture.calls.includes("set_writing_profile_learning") && !fixture.calls.includes("set_writing_profile_sharing"), "Rendering production settings changed profile consent.");

  control("learning").click();
  await until(() => document.querySelector("dialog[open]"), "Enabling production learning did not request consent.");
  document.querySelector("dialog[open] .dialog-actions button").click();
  await until(() => !document.querySelector("dialog[open]") && !control("learning").checked, "Cancelling learning left its control enabled.");
  assert(!(await snapshot()).learning_enabled && !fixture.calls.includes("set_writing_profile_learning"), "Cancelling the disclosure started learning.");

  control("learning").click();
  await until(() => document.querySelector("dialog[open]"), "Learning consent did not reopen.");
  document.querySelector("dialog[open] .dialog-actions button.danger").click();
  await until(async () => (await snapshot()).learning_enabled && !control("learning").disabled, "Confirmed production learning did not enable.");
  assert(!control("sharing").checked && !(await client.invoke("writing_profile_connection")).enabled, "Learning consent also enabled sharing.");

  control("sharing").click();
  await until(async () => (await client.invoke("writing_profile_connection")).enabled && !control("sharing").disabled, "Production profile sharing did not enable independently.");
  const connection = await client.invoke("writing_profile_connection");
  assert(connection.designated && connection.mcp_url === "https://api.pennel.app/mcp/style", "Production profile sharing used the wrong source or connector.");
  panel().querySelector(".writing-profile-url").parentElement.querySelector("button").click();
  await until(() => fixture.copiedTexts().includes(connection.mcp_url), "The production connector URL could not be copied.");
  control("sharing").click();
  await until(async () => !(await client.invoke("writing_profile_connection")).enabled, "Production profile sharing did not turn off.");
  assert((await snapshot()).learning_enabled, "Turning off sharing also disabled learning.");
  assert(!fixture.calls.some(command => ["set_claude_access", "delete_meeting", "delete_history", "start_meeting", "toggle_dictation"].includes(command)), "Production profile controls changed meeting access or original speech.");
  assert(!fixture.unknown.length, `Unknown fixture commands: ${fixture.unknown.join(", ")}`);
  return { productionControlsVisible: true, optInDefaults: true, firstUseConsent: true, sharingIndependent: true, productionConnectorCopied: true };
}
