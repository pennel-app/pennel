import { api, type MeetingSummaryDocument, type SummarySource } from "./api";
import type { Context } from "./main";
import { bindLocalization, l, t } from "./i18n";
import { controlButton, errorText, h, markdown } from "./dom";
import { languageModelConfigured } from "./presentation";
import { SummaryDraft } from "./meeting-summary-state";
import { summaryDestination } from "./meeting-summary-automation";
import { meetingPostprocess } from "./meeting-postprocess-state";
import { sourceRange } from "./source-navigation";
import { meetingSummaryReading } from "./meeting-summary-reading";
import { actionState, attachedSummaryRecord } from "./meeting-summary-reading-state";
import { LatestRequest } from "./async";

export function meetingSummaryEditor(initial: Context, host: {
  id(): string;
  finished(): boolean;
  flushNotes(): Promise<void>;
  template(): string;
  processing?(): boolean;
  canReview?(): boolean;
  meetingUid?(): string;
  changed?(): void;
  sourceAvailable?(source: SummarySource): boolean;
  openSource?(source: SummarySource, invoker: HTMLElement): Promise<void> | void;
  openTemplate?(): void;
  transcriptProvenance?(id: string): Promise<{ provider_kind: string; model: string; requested_language: string | null; generated_at: string } | null>;
}) {
  let ctx = initial, composing = false, generating = false, directCreating = false, loaded = false, loading = false, editing = false, renderedValue: string | null = null, revealPending = false;
  let progressCleanup: (() => void) | null = null;
  let timer: number | undefined, saving: Promise<void> | null = null, reading: Promise<void> | null = null;
  const draft = new SummaryDraft();
  const summaryReads = new LatestRequest();
  let stopSelectionWatch: (() => void) | null = null;
  const editor = h("textarea", { class: "meeting-summary-editor", hidden: true, "aria-label": l("meetings.summary_editor_label"),
    placeholder: l("meetings.summary_editor_placeholder"), spellcheck: "true" });
  const placeholders = h("div", { class: "summary-placeholders", hidden: true, role: "status", "aria-label": l("meetings.summary_preparing") }, ...[1, 2, 3, 4].map(() => h("div", { "aria-hidden": "true" })));
  const rendered = h("div", { class: "meeting-summary-rendered", tabindex: 0, "aria-label": l("meetings.workspace_summary") });
  const reader = meetingSummaryReading({ id: host.id, sourceAvailable: source => host.sourceAvailable?.(source) === true,
    openSource: (source, invoker) => host.openSource?.(source, invoker), openTemplate: host.openTemplate,
    transcriptProvenance: host.transcriptProvenance, updateAction, report });
  rendered.append(reader.root);
  const editSummary = controlButton({ class: "btn compact meeting-summary-edit", "aria-pressed": "false", onclick: () => {
    if (editing) {
      const revision = draft.editRevision;
      void flush().then(() => { if (revision === draft.editRevision && !composing && !draft.dirty) editing = false; update(); }).catch(report);
    }
    else { editing = true; update(); editor.focus({ preventScroll: true }); }
  } }, l("meetings.summary_edit"));
  const status = h("span", { class: "help meeting-summary-status", role: "status", "aria-live": "polite" });
  const explanation = h("p", { class: "help meeting-summary-explanation" }, l("meetings.summary_private_help"));
  const destination = h("p", { class: "help meeting-summary-destination" });
  const details = h("details", { class: "meeting-summary-details" },
    h("summary", {}, l("meetings.summary_details")), explanation, destination);
  const setupReason = h("p", { class: "help meeting-summary-setup-reason" }, l("meetings.summary_setup_short"));
  const error = h("p", { class: "error meeting-summary-error", hidden: true, role: "status" });
  const outdated = h("p", { class: "help", hidden: true }, l("meetings.summary_outdated_editor"));
  const markReviewed = controlButton({ class: "btn compact meeting-summary-mark-reviewed", hidden: true, onclick: () => { void markAsReviewed().catch(report); } }, l("meetings.summary_mark_reviewed"));
  const outdatedNotice = h("div", { class: "meeting-summary-outdated", hidden: true }, outdated, markReviewed);
  const generate = controlButton({ class: "btn primary meeting-summary-generate", onclick: () => { void generateDraft(); } }, l("meetings.summary_generate"));
  const setup = h("button", { class: "text-button", onclick: () => ctx.openSetup("llm") }, l("meetings.set_up_summaries"));
  const retry = controlButton({ class: "btn compact meeting-summary-retry", onclick: () => { void flush().catch(report); } }, l("meetings.summary_retry_save"));
  const recover = controlButton({ class: "btn compact meeting-summary-recover", onclick: () => { void refresh(true).catch(report); } }, l("meetings.summary_recover"));
  const recovery = h("div", { class: "meeting-summary-recovery", hidden: true },
    h("p", { class: "help" }, l("meetings.summary_changed_saved")), h("div", { class: "row" },
      controlButton({ class: "btn compact", onclick: () => {
        if (!draft.keepEditAgainstRemote()) return; update(); void flush().catch(report);
      } }, l("meetings.summary_keep_edit")),
      controlButton({ class: "btn compact", onclick: () => {
        if (composing || !draft.useRemote()) return; editor.value = draft.value; update();
      } }, l("meetings.summary_use_saved"))));
  const candidateBody = h("div", { class: "meeting-summary-candidate-body markdown", tabindex: 0 });
  const candidateReason = h("p", { class: "help" });
  const accept = controlButton({ class: "btn primary meeting-summary-accept", onclick: () => { void acceptCandidate().catch(report); } }, l("meetings.summary_candidate_accept"));
  const candidate = h("section", { class: "meeting-summary-candidate", hidden: true }, h("h3", {}, l("meetings.summary_candidate_title")), candidateReason, candidateBody,
    h("div", { class: "row" }, accept, controlButton({ class: "btn", onclick: () => { draft.candidate = null; update(); } }, l("meetings.summary_candidate_discard"))));
  const historyList = h("div", { class: "meeting-summary-history-list" });
  const history = h("details", { class: "meeting-summary-history" }, h("summary", {}, l("meetings.summary_history")), historyList);
  history.addEventListener("toggle", () => { if (history.open) void loadHistory().catch(report); });
  const root = h("section", { class: "meeting-summary-surface", tabindex: -1, "aria-label": l("meetings.workspace_summary") },
    h("div", { class: "meeting-summary-heading" }, h("h2", {}, l("meetings.workspace_summary")), h("span", { class: "grow" }), status, reader.lengthControl, reader.template, editSummary),
    h("div", { class: "meeting-summary-context" }, details, h("div", { class: "meeting-summary-setup row" }, setupReason, setup)),
    outdatedNotice, recovery, error, h("div", { class: "row meeting-summary-recovery-actions" }, retry, recover), editor, placeholders, rendered,
    h("div", { class: "row meeting-summary-actions" }, generate), candidate, history);

  function progress() { const uid = draft.document?.meeting_uid ?? host.meetingUid?.(); return uid ? meetingPostprocess.get(uid, "summary") : null; }
  function working() { return generating || progress()?.state === "started"; }
  function blockedReason(): string {
    if (!host.finished()) return t("meetings.summary_finished_only");
    if (!loaded) return t("meetings.summary_loading");
    if (composing) return t("meetings.workspace_composing");
    if (draft.remote) return t("meetings.summary_changed_saved");
    if (draft.pending) return t("meetings.workspace_saving");
    if (working() || host.processing?.()) return t("meetings.summary_preparing");
    return languageModelConfigured(ctx.info) ? "" : t("meetings.summary_setup_short");
  }
  function report(failure: unknown) { draft.error = errorText(failure); update(); }
  function update() {
    draft.applyPresentation(presentationProtected());
    const ready = languageModelConfigured(ctx.info), finished = host.finished(), active = working(), remote = progress();
    const remoteError = !generating && !draft.value.trim() && remote?.state === "failed" ? t("meetings.summary_processing_failed") : "";
    if (active && !draft.value.trim()) revealPending = true;
    if (!active && (draft.error || remote?.state === "failed" || remote?.state === "cancelled")) revealPending = false;
    root.dataset.summaryState = active ? "generating" : draft.pending ? "saving" : draft.error || remoteError ? "error" : draft.dirty ? "unsaved" : loaded ? "saved" : "loading";
    status.textContent = active ? t(generating ? "meetings.summary_generating" : "meetings.summary_preparing") : draft.pending ? t("meetings.workspace_saving")
      : draft.error || remoteError ? t("meetings.summary_not_saved") : draft.dirty ? t("meetings.workspace_unsaved")
      : loaded ? draft.value.trim() || draft.document?.edited ? t("meetings.workspace_saved") : "" : t("meetings.summary_loading");
    editor.disabled = !finished || !loaded || directCreating;
    editor.hidden = !editing; rendered.hidden = editing || active && !draft.value.trim();
    rendered.dataset.empty = String(!draft.value.trim());
    placeholders.hidden = !active || !!draft.value.trim() || editing;
    editSummary.disabled = !finished || !loaded || composing || draft.pending || directCreating;
    editSummary.textContent = t(editing ? "meetings.summary_done" : "meetings.structured.edit");
    editSummary.setAttribute("aria-label", t(editing ? "meetings.summary_done" : "meetings.summary_edit"));
    editSummary.setAttribute("aria-pressed", String(editing));
    if (!editing && renderedValue !== draft.value) {
      rendered.classList.toggle("summary-reveal", revealPending && !!draft.value.trim());
      if (draft.value.trim()) revealPending = false;
      renderedValue = draft.value;
    }
    reader.update(draft.document ? { ...draft.document, edited: draft.document.edited ?? false, outdated: draft.document.outdated ?? false,
      markdown: draft.value, record: !draft.dirty && draft.value === draft.document.markdown ? draft.document.record : null } : null,
      editing || !finished || !loaded || composing || draft.pending || active || !!draft.remote || !!host.processing?.());
    reader.lengthControl.hidden ||= editing;
    generate.disabled = !finished || !ready || active || draft.pending || !loaded || composing || !!draft.remote || !!host.processing?.();
    generate.textContent = t(generating ? "meetings.summary_generating" : draft.document?.markdown ? "meetings.summary_generate_again" : "meetings.summary_generate");
    generate.title = !finished ? t("meetings.summary_finished_only") : host.processing?.() ? t("meetings.working") : !ready ? t("meetings.summary_setup_help") : "";
    setup.hidden = ready;
    setupReason.hidden = ready;
    destination.textContent = !finished ? t("meetings.summary_finished_only") : !ready ? t("meetings.summary_setup_help") : summaryDestination(ctx);
    error.textContent = draft.error ?? remoteError; error.hidden = !error.textContent;
    outdated.hidden = !draft.document?.outdated && !draft.remote?.outdated;
    markReviewed.hidden = !draft.document?.outdated || !draft.document.markdown.trim();
    markReviewed.disabled = !draft.canMarkReviewed || composing || active || loading || !finished || !loaded || !!host.processing?.() || host.canReview?.() === false;
    outdatedNotice.hidden = outdated.hidden && markReviewed.hidden;
    retry.hidden = !draft.error || !!draft.remote || !draft.dirty; retry.disabled = draft.pending || composing;
    recover.hidden = !draft.error || !!draft.remote; recover.disabled = loading || draft.pending;
    root.querySelector<HTMLElement>(".meeting-summary-recovery-actions")!.hidden = retry.hidden && recover.hidden;
    recovery.hidden = !draft.remote;
    candidate.hidden = !draft.candidate;
    accept.disabled = !draft.canAccept || composing;
    candidateReason.textContent = t(draft.canAccept ? "meetings.summary_candidate_help" : "meetings.summary_candidate_stale");
    history.hidden = !finished;
    host.changed?.();
  }
  function schedule() { clearTimeout(timer); if (!composing) timer = window.setTimeout(() => { void flush().catch(report); }, 600); }
  editor.addEventListener("input", () => { draft.edit(editor.value); update(); schedule(); });
  editor.addEventListener("compositionstart", () => { composing = true; clearTimeout(timer); update(); });
  editor.addEventListener("compositionend", () => { composing = false; draft.edit(editor.value); update(); schedule(); });
  editor.addEventListener("blur", () => { if (!composing) void flush().catch(report); });
  rendered.addEventListener("focusout", () => { queueMicrotask(applyDeferredPresentation); });
  function presentationProtected() { return composing || editing || documentActive() || renderedSelection() || reader.interacting(); }
  function applyDeferredPresentation() { if (root.isConnected && draft.applyPresentation(presentationProtected())) update(); }
  function watchPresentation(next: Context) {
    stopSelectionWatch?.();
    document.addEventListener("selectionchange", applyDeferredPresentation);
    const stop = () => document.removeEventListener("selectionchange", applyDeferredPresentation);
    stopSelectionWatch = stop;
    next.onDispose(() => { if (stopSelectionWatch === stop) { stop(); stopSelectionWatch = null; } });
  }
  async function flush(): Promise<void> {
    clearTimeout(timer);
    if (composing) throw new Error(t("meetings.workspace_composing"));
    if (saving) { await saving; if (draft.dirty) return flush(); return; }
    if (!draft.dirty || !draft.document || !host.finished()) return;
    if (draft.remote) throw new Error(t("meetings.summary_changed_saved"));
    const target = draft.value, base = draft.document;
    invalidateReads(); draft.pending = true; update();
    saving = api.saveMeetingSummary(host.id(), base.revision, base.source_revision, target).then(validated).then(document => {
      invalidateReads(); draft.acknowledge(document, target);
    }).catch(failure => { report(failure); throw failure; }).finally(() => { saving = null; draft.pending = false; update(); });
    await saving;
  }
  function invalidateReads() { summaryReads.cancel(); reading = null; loading = false; }
  async function refresh(force = false) {
    if (reading) return reading;
    if (!host.finished() || draft.pending || (!force && draft.dirty)) return;
    loading = true; update();
    const id = host.id();
    const request: Promise<void> = summaryReads.run(() => api.meetingSummary(id).then(validated), document => {
      if (host.id() !== id) return;
      loaded = true;
      const changed = draft.receive(document, presentationProtected());
      if (changed && editor.value !== draft.value) editor.value = draft.value;
    }).then(() => undefined).catch(failure => {
      if (host.id() !== id) return;
      report(failure); throw failure;
    }).finally(() => { if (reading === request) { reading = null; loading = false; update(); } });
    reading = request; await request;
  }
  async function validated(document: MeetingSummaryDocument): Promise<MeetingSummaryDocument> { return { ...document, record: await attachedSummaryRecord(document) }; }
  function documentActive() { return document.activeElement === editor || rendered.contains(document.activeElement); }
  function renderedSelection() {
    const selection = window.getSelection();
    return !!selection && !selection.isCollapsed && Array.from({ length: selection.rangeCount }, (_, index) => selection.getRangeAt(index)).some(range => range.intersectsNode(rendered));
  }
  async function generateDraft(allowDirect = true) {
    if (generate.disabled) return;
    generating = true; draft.error = null;
    const uid = draft.document?.meeting_uid ?? host.meetingUid?.(); if (uid) meetingPostprocess.clearTerminal(uid, "summary");
    update();
    try {
      await host.flushNotes(); invalidateReads(); await flush(); await refresh();
      const revision = draft.editRevision, before = draft.document;
      if (allowDirect && before?.edited === false && !before.markdown.trim() && !draft.dirty && !draft.remote && !editing && !composing) {
        directCreating = true; update();
        invalidateReads(); await api.summarizeMeeting(host.id(), host.template(), before);
        invalidateReads(); await refresh(true);
      } else {
        const generated = await api.generateMeetingSummary(host.id(), host.template());
        const result = { ...generated, record: await attachedSummaryRecord(generated) };
        draft.stage(result, revision);
        candidateBody.replaceChildren(markdown(result.markdown));
      }
    } catch (failure) { report(failure); }
    finally { generating = false; directCreating = false; update(); }
  }
  async function dismissPrompt() {
    if (!loaded || composing || draft.pending || draft.remote || working()) return;
    await host.flushNotes(); invalidateReads(); await flush(); await refresh();
    const before = draft.document; if (!before) throw new Error(t("meetings.summary_loading"));
    invalidateReads(); draft.pending = true; update();
    saving = api.dismissMeetingSummaryPrompt(host.id(), before.revision, before.source_revision).then(validated).then(document => {
      invalidateReads();
      if (!draft.acknowledgeMetadata(document)) throw new Error(t("meetings.summary_changed_saved"));
    }).finally(() => { saving = null; draft.pending = false; update(); });
    await saving;
  }
  async function acceptCandidate() {
    if (!draft.canAccept || composing || !draft.candidate) return;
    const selected = draft.candidate, revision = draft.editRevision;
    invalidateReads(); draft.pending = true; update();
    saving = api.acceptMeetingSummary(host.id(), selected.revision, selected.source_revision, selected.markdown, selected.record).then(validated).then(result => {
      if (draft.editRevision === revision) { draft.value = result.markdown; editor.value = draft.value; }
      invalidateReads(); draft.acknowledge(result, result.markdown); draft.candidate = null;
    }).finally(() => { saving = null; draft.pending = false; update(); });
    await saving;
  }
  async function updateAction(base: MeetingSummaryDocument, index: number, change: { done?: boolean; owner?: string | null }) {
    const current = draft.document;
    if (!current || current.meeting_uid !== base.meeting_uid || current.revision !== base.revision || current.source_revision !== base.source_revision
      || editing || composing || draft.dirty || draft.pending || draft.remote || draft.deferredPresentation || working() || !host.finished() || host.processing?.()) throw new Error(t("meetings.summary_changed_saved"));
    const action = current.record?.structure?.actions[index];
    if (!action) throw new Error(t("meetings.summary_changed_saved"));
    const state = actionState(action, change), revision = draft.editRevision;
    invalidateReads(); draft.pending = true; draft.error = null; update();
    saving = api.updateMeetingSummaryAction(host.id(), current.revision, current.source_revision, index, state.done, state.owner).then(validated).then(result => {
      if (revision === draft.editRevision) { draft.value = result.markdown; editor.value = draft.value; }
      invalidateReads(); draft.acknowledge(result, result.markdown);
    }).finally(() => { saving = null; draft.pending = false; update(); });
    await saving;
  }
  async function markAsReviewed() {
    if (markReviewed.disabled || !draft.document) return;
    const reviewed = draft.document;
    invalidateReads(); draft.pending = true; update();
    saving = api.acceptMeetingSummary(host.id(), reviewed.revision, reviewed.source_revision, reviewed.markdown).then(validated).then(result => {
      invalidateReads(); draft.acknowledge(result, reviewed.markdown);
    }).finally(() => { saving = null; draft.pending = false; update(); });
    await saving;
  }
  async function loadHistory() {
    const entries = await api.meetingSummaryHistory(host.id());
    historyList.replaceChildren(...entries.map(entry => h("details", { class: "meeting-summary-version" },
      h("summary", {}, entry.created_at), markdown(entry.markdown), controlButton({ class: "btn compact", onclick: async () => {
        if (composing || draft.pending) return;
        try {
          await flush(); await refresh();
          if (!draft.document || draft.remote) return;
          const revision = draft.editRevision;
          invalidateReads(); draft.pending = true; update();
          saving = api.restoreMeetingSummary(host.id(), entry.revision, draft.document.revision, draft.document.source_revision).then(validated).then(result => {
            if (revision === draft.editRevision) { draft.value = result.markdown; editor.value = draft.value; ++draft.editRevision; }
            draft.candidate = null; invalidateReads(); draft.acknowledge(result, result.markdown);
          }).finally(() => { saving = null; draft.pending = false; update(); });
          await saving; await loadHistory();
        } catch (failure) { report(failure); }
      } }, l("meetings.summary_restore")))));
    if (!entries.length) historyList.append(h("p", { class: "help" }, l("meetings.summary_history_empty")));
  }
  watchPresentation(initial); bindLocalization(root, update); update();
  return { root, editor, provenance: reader.provenance, flush, refresh, readiness: update, working, blockedReason,
    canGenerate: () => !generate.disabled,
    canDismiss: () => loaded && !composing && !draft.pending && !draft.remote && !working(),
    generate: () => generateDraft(true), dismissPrompt,
    bind(next: Context) {
      ctx = next; watchPresentation(next); next.onInfoChange(update); progressCleanup?.();
      const cleanup = meetingPostprocess.subscribe(() => { update(); const state = progress(); if (state && state.state !== "started" && host.finished()) void refresh().catch(report); });
      progressCleanup = cleanup;
      next.onDispose(() => { cleanup(); if (progressCleanup === cleanup) progressCleanup = null; });
      update();
    },
    editing: () => editing,
    restorePresentation(wasEditing: boolean) { if (!draft.dirty && !composing) { editing = wasEditing; update(); } },
    focusSource(text: string) {
      const range = sourceRange(editor.value, text);
      if (!range) return false;
      editing = true; update();
      editor.focus({ preventScroll: true }); editor.setSelectionRange(range.start, range.end); return true;
    } };
}
