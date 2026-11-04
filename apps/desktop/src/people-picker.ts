import { api } from "./api";
import { askText, controlButton, errorText, h, textField } from "./dom";
import { bindLocalization, l, t, type TranslationKey } from "./i18n";
import { noteSelectionBytes, participantGroups, participantHint, samePeopleSelection, PeopleLoadEpoch, type ChosenContact, type MeetingPerson,
  type PeopleAction, type PeopleSelection, type PeopleSnapshot, type PeopleUpdateResult } from "./people-state";
import "./people.css";

export interface PeopleHost {
  id(): string;
  sessionId?(): string | undefined;
  platform: string;
  valid(): boolean;
  changed?(snapshot: PeopleSnapshot): void;
  report(error: unknown): void;
  notes?: {
    current(): PeopleSelection;
    flush(): Promise<void>;
    saved(result: PeopleUpdateResult, before: PeopleSelection): void;
  };
}
type Target = { kind: "manage" } | { kind: "speaker"; utteranceIds: string[] } | { kind: "insert" } | { kind: "mention"; mentionId: string };
let dialogNumber = 0;
const responseKeys: Record<string, TranslationKey> = { pending: "people.response_pending", accepted: "people.response_accepted", declined: "people.response_declined",
  tentative: "people.response_tentative", delegated: "people.response_delegated", completed: "people.response_completed", in_process: "people.response_in_process" };

function contactFields(contact: ChosenContact) {
  return { display_name: contact.display_name, email: contact.email,
    source: { source_id: contact.source, account_id: null, record_id: contact.source_id, revision: contact.source_revision } };
}

/** Both note editors use the same explicit, owner-only participant actions. */
export async function openPeoplePicker(host: PeopleHost, initialTarget: Target = { kind: "manage" }): Promise<void> {
  const originalId = host.id(), originalSession = host.sessionId?.();
  const before = initialTarget.kind === "insert" ? host.notes?.current() : undefined;
  if (before?.composing) { host.report(new Error(t("meetings.workspace_composing"))); return; }
  try { if (before) await host.notes!.flush(); } catch (error) { host.report(error); return; }
  if (!host.valid() || host.id() !== originalId || host.sessionId?.() !== originalSession) return;
  if (before && !samePeopleSelection(before, host.notes!.current())) { host.report(new Error(t("people.selection_changed"))); return; }
  const invoker = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const key = `people-picker-${++dialogNumber}`;
  const dialog = h("dialog", { class: "editor-dialog people-picker", "aria-modal": "true", "aria-labelledby": `${key}-title`, "aria-describedby": `${key}-description` });
  const heading = h("h2", { id: `${key}-title` });
  const explanation = h("p", { class: "help", id: `${key}-description` }, l("people.description"));
  const search = textField({ type: "search", "aria-label": l("people.search"), placeholder: l("people.search") });
  const people = h("div", { class: "people-results", "aria-label": l("people.title") });
  const notice = h("p", { class: "help", role: "status" }, l("people.loading"));
  const commitStatus = h("p", { class: "help people-commit-status", role: "status", "aria-live": "polite", hidden: true }, l("people.inserting"));
  const retry = controlButton({ class: "btn compact", hidden: true, onclick: () => { void load(); } }, l("common.retry"));
  const recovery = h("p", { class: "help", role: "status", hidden: true });
  const detail = h("div", { class: "people-selected" });
  const speakerLinks = h("section", { class: "people-speaker-links" });
  const mentions = h("section", { class: "people-mentions" });
  const visibleName = textField({ "aria-label": l("people.visible_name"), autocomplete: "off" });
  const visibleField = h("label", { class: "people-visible-field", hidden: true }, h("span", {}, l("people.visible_name")), visibleName,
    h("span", { class: "help" }, l("people.visible_help")));
  const apply = controlButton({ class: "btn primary", disabled: true, hidden: initialTarget.kind === "manage", onclick: () => { void applyPerson(); } });
  const unlink = controlButton({ class: "btn", hidden: true, onclick: () => { void unlinkTarget(); } }, l("people.unlink"));
  const chooseContact = controlButton({ class: "btn", disabled: host.platform !== "macos", onclick: () => { void choose(); } }, l("people.choose_contact"));
  const create = controlButton({ class: "btn", onclick: () => { void manual(); } }, l("people.manual"));
  const cancel = controlButton({ class: "btn", onclick: () => close() }, l("common.cancel"));
  let closed = false, loading = false, committingNotes = false, manualName = "";
  const loads = new PeopleLoadEpoch();
  let snapshot: PeopleSnapshot | null = null;
  let selected: string | null = null;
  let target = initialTarget;
  const valid = () => !closed && host.valid() && host.id() === originalId && host.sessionId?.() === originalSession;
  const close = () => {
    if (committingNotes || closed) return;
    closed = true; loads.invalidate(); dialog.close(); dialog.remove(); if (invoker?.isConnected) invoker.focus({ preventScroll: true });
  };
  const failure = (error: unknown) => { if (valid()) { notice.textContent = errorText(error); notice.className = "error"; notice.hidden = false; retry.hidden = false; } };
  const busy = (value: boolean) => {
    loading = value; create.disabled = value; chooseContact.disabled = value || host.platform !== "macos"; search.disabled = value;
    visibleName.disabled = value; unlink.disabled = value; retry.disabled = value;
    cancel.disabled = committingNotes; commitStatus.hidden = !committingNotes; dialog.setAttribute("aria-busy", String(committingNotes)); updateApply();
  };
  function updateApply() {
    const inserting = target.kind === "insert";
    visibleField.hidden = !inserting || !selected;
    apply.hidden = target.kind === "manage";
    apply.disabled = loading || !selected || inserting && !visibleName.value.trim();
    apply.textContent = t(committingNotes ? "people.inserting" : inserting ? "people.insert" : target.kind === "mention" ? "people.relink" : "people.link");
    heading.textContent = t(target.kind === "speaker" ? "people.link_title" : inserting ? "people.mention_title" : "people.title");
    const link = target.kind === "speaker" ? snapshot?.speaker_links.find(link => link.utterance_ids.some(id => target.kind === "speaker" && target.utteranceIds.includes(id))) : null;
    unlink.hidden = !link && target.kind !== "mention";
  }
  function choosePerson(person: MeetingPerson) {
    if (loading) return;
    selected = person.id;
    // A contact-only name does not become user-authored Markdown by selecting a private link.
    visibleName.value = person.origin === "contact" ? "" : person.display_name;
    render();
    if (target.kind === "insert") visibleName.focus();
  }
  function render() {
    if (!snapshot || !valid()) return;
    const focusedPerson = document.activeElement instanceof HTMLElement && people.contains(document.activeElement)
      ? document.activeElement.closest<HTMLElement>("[data-person-id]")?.dataset.personId : undefined;
    recovery.hidden = !snapshot.recovery_status || snapshot.recovery_status === "ready";
    recovery.textContent = recovery.hidden ? "" : t(snapshot.recovery_status === "confirmation_pending" ? "people.confirmation_pending"
      : snapshot.recovery_status === "not_written" ? "people.recovery_not_written" : "people.recovery_review");
    if (snapshot.recovery_status === "confirmation_pending") retry.hidden = false;
    const groups = participantGroups(snapshot.people, search.value);
    people.replaceChildren(...groups.map(group => h("section", { class: "people-group" }, h("h3", {}, l(group.kind === "meeting" ? "people.meeting_group" : "people.contacts_group")),
      ...group.people.map(person => {
        const hint = participantHint(person, snapshot!.people, snapshot!.calendar);
        return controlButton({ class: `people-choice${selected === person.id ? " selected" : ""}`, disabled: loading, "aria-pressed": String(selected === person.id), "data-person-id": person.id,
          onclick: () => choosePerson(person) }, h("span", { class: "people-name" }, person.private_alias || person.display_name || l("people.unnamed")),
          person.email ? h("span", { class: "help" }, person.email) : null,
          hint.ordinal ? h("span", { class: "help" }, l("people.same_name", { number: hint.ordinal })) : null,
          hint.is_self ? h("span", { class: "help" }, l("people.calendar_self")) : null,
          hint.response && responseKeys[hint.response] ? h("span", { class: "help" }, l(responseKeys[hint.response])) : null,
          person.source_status === "unavailable" ? h("span", { class: "help" }, l("people.source_unavailable")) : null);
      }))));
    if (!groups.length) people.append(h("p", { class: "help" }, l("people.empty")));
    const person = snapshot.people.find(person => person.id === selected);
    detail.replaceChildren();
    if (person) {
      detail.append(h("h3", {}, l("people.private_details")), h("p", {}, person.display_name || l("people.unnamed")));
      if (person.origin === "contact") detail.append(h("p", { class: "help" }, l(person.source_status === "unavailable" ? "people.source_unavailable" : "people.contact_snapshot")),
        controlButton({ class: "btn compact", disabled: host.platform !== "macos" || loading, onclick: () => { void choose(person.id); } }, l("people.choose_again")));
      detail.append(h("div", { class: "row people-person-actions" },
        controlButton({ class: "btn compact", disabled: loading, onclick: () => { void alias(person); } }, l("people.alias")),
        controlButton({ class: "btn compact", disabled: loading, onclick: () => { void renamePerson(person); } }, l("people.rename_person"))));
      detail.append(controlButton({ class: "text-button", disabled: loading, onclick: () => { void mutate({ action: "remove_person", person_id: person.id }).then(() => { selected = null; render(); }).catch(failure); } }, l("people.remove")));
    }
    speakerLinks.replaceChildren();
    if (snapshot.speaker_links.length) {
      speakerLinks.append(h("h3", {}, l("people.speaker_links")));
      for (const link of snapshot.speaker_links) {
        const person = snapshot.people.find(person => person.id === link.person_id);
        speakerLinks.append(h("div", { class: "people-mention-row", "data-speaker-link-id": link.id },
          h("span", {}, person?.private_alias || person?.display_name || l("people.unnamed")),
          h("span", { class: "help" }, l(link.state === "needs_review" ? "people.review" : "people.linked")),
          controlButton({ class: "btn compact", disabled: loading, onclick: () => { void mutate({ action: "unlink_speaker", link_id: link.id }).catch(failure); } }, l("people.unlink"))));
      }
      if (snapshot.speaker_links.some(link => link.state === "needs_review")) speakerLinks.append(h("p", { class: "help" }, l("people.speaker_relink_help")));
    }
    mentions.replaceChildren(h("h3", {}, l("people.mentions")));
    if (!snapshot.mentions.length) mentions.append(h("p", { class: "help" }, l("people.no_mentions")));
    for (const mention of snapshot.mentions) mentions.append(h("div", { class: "people-mention-row" }, h("span", {}, `@${mention.visible_name}`),
      mention.state === "unresolved" ? h("span", { class: "help" }, l("people.unresolved")) : null,
      controlButton({ class: "btn compact", disabled: loading, onclick: () => { target = { kind: "mention", mentionId: mention.id }; selected = mention.person_id; render(); search.focus(); } }, l("people.relink")),
      controlButton({ class: "btn compact", disabled: loading, onclick: () => { void mutate({ action: "unlink_mention", mention_id: mention.id }).catch(failure); } }, l("people.unlink"))));
    updateApply();
    if (focusedPerson) [...people.querySelectorAll<HTMLElement>("[data-person-id]")].find(element => element.dataset.personId === focusedPerson)?.focus({ preventScroll: true });
  }
  async function mutate(action: PeopleAction): Promise<PeopleUpdateResult | null> {
    if (!valid() || !snapshot || loading) return null;
    const revision = snapshot.revision;
    const canonicalInsert = action.action === "insert_mention";
    committingNotes = canonicalInsert;
    busy(true); render();
    try {
      const result = await api.updateMeetingPeople(originalId, { ...action, expected_revision: revision }, originalSession);
      // The core may have saved Markdown even if navigation or finalization detached the picker.
      // Acknowledge only its captured draft before dropping obsolete presentation results.
      if (canonicalInsert && result.notes !== undefined && before) host.notes?.saved(result, before);
      if (!valid()) return null;
      snapshot = result.snapshot;
      host.changed?.(snapshot);
      notice.hidden = true; retry.hidden = true;
      render();
      return result;
    } finally {
      committingNotes = false; busy(false); render();
      if (!valid()) close();
    }
  }
  async function manual() {
    const name = await askText(l("people.manual_title"), manualName, l("people.manual_help"));
    if (!name || !valid()) return;
    manualName = name;
    const previous = new Set(snapshot?.people.map(person => person.id));
    try {
      const result = await mutate({ action: "create_person", display_name: name, origin: "manual" });
      const added = result?.snapshot.people.find(person => !previous.has(person.id));
      if (added) { manualName = ""; choosePerson(added); }
    } catch (error) { failure(error); }
  }
  async function choose(personId?: string) {
    if (!valid() || loading || host.platform !== "macos") return;
    busy(true);
    const epoch = loads.begin(originalId, originalSession);
    try {
      const result = await api.pickMeetingContact();
      if (!valid() || !loads.accepts(epoch, host.id(), host.sessionId?.())) return;
      if (result.status === "unavailable") { failure(new Error(result.reason || t("people.contact_unavailable"))); return; }
      if (result.status !== "selected" || !result.contact) return;
      busy(false);
      const previous = new Set(snapshot?.people.map(person => person.id));
      const saved = await mutate(personId ? { action: "replace_source", person_id: personId, ...contactFields(result.contact) }
        : { action: "create_person", origin: "contact", ...contactFields(result.contact) });
      const person = saved?.snapshot.people.find(person => personId ? person.id === personId : !previous.has(person.id));
      if (person) choosePerson(person);
    } catch (error) { failure(error); }
    finally { if (valid() && loads.accepts(epoch, host.id(), host.sessionId?.())) busy(false); }
  }
  async function alias(person: MeetingPerson) {
    const name = await askText(l("people.alias"), person.private_alias ?? "", l("people.alias_help"));
    if (name === null || !valid()) return;
    try { await mutate({ action: "set_private_alias", person_id: person.id, private_alias: name || null }); }
    catch (error) { failure(error); }
  }
  async function renamePerson(person: MeetingPerson) {
    const name = await askText(l("people.rename_person"), person.display_name, l("people.person_name_help"));
    if (!name || !valid()) return;
    try { await mutate({ action: "rename_person", person_id: person.id, display_name: name }); }
    catch (error) { failure(error); }
  }
  async function applyPerson() {
    if (!snapshot || !selected || loading || !valid()) return;
    try {
      if (target.kind === "speaker") {
        const link = snapshot.speaker_links.find(link => link.utterance_ids.some(id => target.kind === "speaker" && target.utteranceIds.includes(id)));
        await mutate({ action: "link_speaker", person_id: selected, utterance_ids: target.utteranceIds, expected_source_revision: snapshot.source_revision, ...link ? { link_id: link.id } : {} });
      } else if (target.kind === "mention") await mutate({ action: "relink_mention", mention_id: target.mentionId, person_id: selected, expected_notes_revision: snapshot.notes_revision });
      else if (target.kind === "insert" && before && host.notes) {
        if (!samePeopleSelection(before, host.notes.current())) throw new Error(t("people.selection_changed"));
        const range = noteSelectionBytes(before.text, before.selection_start, before.selection_end);
        if (!range) throw new Error(t("people.selection_changed"));
        const result = await mutate({ action: "insert_mention", person_id: selected, visible_name: visibleName.value.trim(), notes: before.text, ...range, expected_notes_revision: snapshot.notes_revision });
        if (result && valid()) {
          if (result.snapshot.recovery_status === "confirmation_pending") { target = { kind: "manage" }; render(); return; }
        }
      }
      if (valid()) close();
    } catch (error) { failure(error); }
  }
  async function unlinkTarget() {
    if (!snapshot) return;
    try {
      if (target.kind === "mention") await mutate({ action: "unlink_mention", mention_id: target.mentionId });
      else if (target.kind === "speaker") {
        const link = snapshot.speaker_links.find(link => link.utterance_ids.some(id => target.kind === "speaker" && target.utteranceIds.includes(id)));
        if (link) await mutate({ action: "unlink_speaker", link_id: link.id });
      }
      if (valid()) close();
    } catch (error) { failure(error); }
  }
  search.addEventListener("input", render);
  visibleName.addEventListener("input", updateApply);
  dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
  bindLocalization(dialog, () => { updateApply(); render(); });
  const content = h("div", { class: "people-picker-content" }, explanation, search, notice, commitStatus, retry, recovery, people, h("div", { class: "row people-add-actions" }, create, chooseContact),
    ...host.platform !== "macos" ? [h("p", { class: "help" }, l("people.contact_unavailable"))] : [], detail, visibleField, speakerLinks, mentions,
  );
  dialog.append(heading, content, h("div", { class: "dialog-actions row" }, unlink, h("span", { class: "grow" }), cancel, apply));
  document.body.append(dialog); dialog.showModal(); search.focus(); updateApply();
  async function load() {
    const epoch = loads.begin(originalId, originalSession);
    try {
      busy(true); retry.hidden = true;
      notice.className = "help"; notice.textContent = t("people.loading"); notice.hidden = false;
      const result = await api.meetingPeople(originalId);
      if (!valid() || !loads.accepts(epoch, host.id(), host.sessionId?.())) return;
      snapshot = result;
      if (target.kind === "speaker") selected = snapshot.speaker_links.find(link => link.utterance_ids.some(id => target.kind === "speaker" && target.utteranceIds.includes(id)))?.person_id ?? selected;
      if (!snapshot.people.some(person => person.id === selected)) selected = null;
      notice.hidden = true; render();
    } catch (error) { failure(error); }
    finally { if (valid() && loads.accepts(epoch, host.id(), host.sessionId?.())) { busy(false); render(); } }
  }
  await load();
}

export function peopleNoteControls(host: PeopleHost): HTMLElement {
  const mention = controlButton({ class: "btn compact people-insert", onclick: () => { void openPeoplePicker(host, { kind: "insert" }).catch(host.report); } }, l("people.mention"));
  mention.addEventListener("pointerdown", event => event.preventDefault());
  return h("div", { class: "row people-note-controls" }, mention,
    controlButton({ class: "btn compact people-manage", onclick: () => { void openPeoplePicker(host).catch(host.report); } }, l("people.title")));
}
