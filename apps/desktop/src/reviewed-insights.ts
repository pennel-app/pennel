import { api, listen, type InsightItem, type InsightSource } from "./api";
import { bindLocalization, l, t, type TranslationKey } from "./i18n.ts";
import { controlButton, formatClock, h, select } from "./dom";
import { VisibilityPoll } from "./window-lifecycle";
import { insightService } from "./insight-service";
import { InsightReviewModel, InsightUiError, type InsightNoteHost } from "../../../assets/contracts/insight-review-model.ts";
import { insightReceipt, insightText, insightsWritable, safeInsightDestination, sourceCandidates } from "../../../assets/contracts/meeting-insights.ts";
import type { Context } from "./main";

export interface InsightReviewHost {
  id: string; meetingUid: string; notes: InsightNoteHost;
  openSource(source: InsightSource, origin: HTMLElement, restore: () => void): Promise<boolean>;
}
interface ReviewView { root: HTMLElement; model: InsightReviewModel; bind(ctx: Context, host: InsightReviewHost): void; refresh(): Promise<void>; flush(): Promise<boolean> }
const retained = new Map<string, ReviewView>();

export function insightReasonKey(code: string | undefined): TranslationKey {
  const known = new Set(["unknown_language", "unconfigured", "unqualified", "configuration_changed", "identity_changed", "mutation_in_progress", "meeting_active", "meeting_busy", "source_unavailable", "no_transcript", "context_too_large", "cancelled", "deadline_exceeded", "failed"]);
  return known.has(code ?? "") ? `insights.reason_${code}` as TranslationKey : "insights.reason_unavailable";
}
export function insightErrorKey(error: unknown): TranslationKey {
  return error instanceof InsightUiError && ["notes_changed", "notes_not_saved", "edit_conflict", "wrong_meeting"].includes(error.code)
    ? `insights.error_${error.code}` as TranslationKey : "insights.operation_failed";
}

export function reviewedInsights(ctx: Context, host: InsightReviewHost): ReviewView {
  let view = retained.get(host.meetingUid);
  if (!view) { view = createReview(ctx, host); retained.set(host.meetingUid, view); }
  view.bind(ctx, host); return view;
}

function createReview(initialContext: Context, initialHost: InsightReviewHost): ReviewView {
  let ctx = initialContext, host = initialHost, bound: Context | null = null, binding = 0, focusEpoch = 0;
  let showDismissed = false;
  const model = new InsightReviewModel(host.id, host.meetingUid, insightService);
  const root = h("section", { class: "insight-review", "data-insights-meeting": host.meetingUid });
  const language = select([["unknown", l("insights.language_unknown")], ["en", l("insights.language_en")], ["nl", l("insights.language_nl")], ["mixed", l("insights.language_mixed")]], "unknown",
    value => { void model.choosePolicy(value as "unknown" | "en" | "nl" | "mixed"); }, "menu");
  language.setAttribute("aria-label", t("insights.language"));
  const find = controlButton({ class: "btn primary insight-find", onclick: () => { void model.analyze(); } }, l("insights.find"));
  const cancel = controlButton({ class: "btn insight-cancel", hidden: true, onclick: () => { void model.cancel(); } }, l("insights.cancel_analysis"));
  const refresh = controlButton({ class: "text-button", onclick: () => { void Promise.all([model.load(), model.choosePolicy(model.getSnapshot().policy)]); } }, l("insights.refresh"));
  const exportItems = controlButton({ class: "btn insight-export", onclick: async () => { const value = await model.export(); if (value !== undefined) { await api.copyText(value); notice.textContent = t("insights.copied"); } } }, l("insights.copy_markdown"));
  const destination = h("p", { class: "help insight-destination" });
  const availability = h("p", { class: "help insight-availability", role: "status" });
  const progress = h("p", { class: "help insight-progress", role: "status", hidden: true });
  const error = h("p", { class: "error insight-error", role: "status", hidden: true });
  const notice = h("p", { class: "help insight-notice", role: "status" });
  const empty = h("p", { class: "muted insight-empty" }, l("insights.empty"));
  const list = h("div", { class: "insight-list" });
  const dismissed = h("button", { class: "text-button insight-dismissed-toggle", onclick: () => { showDismissed = !showDismissed; render(); } });
  const rows = new Map<string, { element: HTMLElement; update(item: InsightItem): void }>();
  root.append(h("h2", {}, l("insights.title")), h("p", { class: "help" }, l("insights.description")),
    h("div", { class: "insight-toolbar" }, h("label", { class: "insight-language" }, l("insights.language"), language), find, cancel, refresh),
    destination, availability, progress, error, notice, h("div", { class: "insight-export-row" }, exportItems, dismissed), empty, list);
  const text = (element: HTMLElement, value: string) => { if (element.textContent !== value) element.textContent = value; };

  function createCard(initial: InsightItem) {
    let item = initial;
    const title = h("p", { class: "insight-kind" });
    const metadata = h("p", { class: "help insight-item-state" });
    const body = h("p", { class: "insight-item-text" });
    const editor = h("textarea", { class: "insight-edit", rows: 3, hidden: true, "aria-label": l("insights.edit_label") });
    const details = h("p", { class: "help insight-owner-due" });
    const cutoff = h("p", { class: "help insight-cutoff" });
    const sourceNotice = h("p", { class: "help insight-source-notice", hidden: true });
    const quotes = h("div", { class: "insight-sources" });
    const history = h("details", { class: "insight-source-history", hidden: true }, h("summary", {}, l("insights.source_history")));
    const historyQuotes = h("div", {}); history.append(historyQuotes);
    const insertion = h("p", { class: "help insight-insertion-state", hidden: true });
    const chooseSource = h("div", { class: "insight-reassociate", hidden: true });
    const accept = controlButton({ class: "btn compact insight-accept", onclick: () => { void model.review(item.id, "accepted"); } }, l("insights.accept"));
    const dismiss = controlButton({ class: "btn compact insight-dismiss", onclick: () => { void model.review(item.id, "dismissed"); } }, l("insights.dismiss"));
    const restore = controlButton({ class: "btn compact insight-restore", onclick: () => { void model.review(item.id, "proposed"); } }, l("insights.restore"));
    const edit = controlButton({ class: "btn compact insight-edit-action", onclick: async () => {
      const epoch = focusEpoch; await model.beginEdit(item.id);
      if (epoch === focusEpoch && root.isConnected && !editor.hidden) editor.focus({ preventScroll: true });
    } }, l("insights.edit"));
    const save = controlButton({ class: "btn compact insight-save-edit", hidden: true, onclick: () => { void model.saveEdit(item.id); } }, l("insights.save_edit"));
    const cancelEdit = controlButton({ class: "text-button insight-cancel-edit", hidden: true, onclick: () => { void model.cancelEdit(item.id); } }, l("common.cancel"));
    const add = controlButton({ class: "btn compact insight-add-notes", onclick: () => { void model.insert(item.id, host.notes); } }, l("insights.add_notes"));
    const check = controlButton({ class: "text-button insight-check-insertion", hidden: true, onclick: () => { void model.reconcile(); } }, l("insights.check_insertion"));
    const reassociate = controlButton({ class: "text-button insight-reassociate-action", hidden: true, onclick: async () => {
      chooseSource.hidden = false; chooseSource.replaceChildren(h("p", { class: "help" }, l("common.loading")));
      try {
        const sourceIndex = item.sources.findIndex(source => source.meeting_uid === host.meetingUid);
        const original = item.sources[sourceIndex];
        const detail = await api.meetingDetail(model.id);
        const candidates = original ? sourceCandidates(original, detail.transcript, detail.meeting_uid ?? host.meetingUid) : [];
        if (!candidates.length) { chooseSource.replaceChildren(h("p", { class: "help" }, l("insights.no_matching_source"))); return; }
        const itemRevision = item.revision;
        const chosen = select([["", l("insights.choose_source")], ...candidates.map((source, index) => [String(index), `${formatClock(source.start_ms / 1000)} · ${source.quote.slice(0, 90)} (${index + 1})`] as [string, string])], "", () => { use.disabled = chosen.value === ""; }, "menu");
        const use = controlButton({ class: "btn compact", disabled: true, onclick: async () => {
          const source = candidates[Number(chosen.value)]; if (!source || model.item(item.id)?.revision !== itemRevision) { text(sourceNotice, t("insights.error_edit_conflict")); sourceNotice.hidden = false; return; }
          await model.reassociate({ item_id: item.id, item_revision: itemRevision, source_index: sourceIndex, source,
            evidence_cutoff_ms: Math.max(item.evidence_cutoff_ms, source.end_ms) }); chooseSource.hidden = true;
        } }, l("insights.use_source"));
        chooseSource.replaceChildren(h("p", { class: "help" }, l("insights.reassociate_help")), chosen, use);
      } catch { chooseSource.replaceChildren(h("p", { class: "error" }, l("insights.operation_failed"))); }
    } }, l("insights.reassociate"));
    const element = h("article", { class: "insight-card", "data-insight-id": item.id }, title, metadata, body, editor, details, cutoff,
      quotes, history, sourceNotice, chooseSource, insertion, h("div", { class: "insight-actions" }, accept, edit, dismiss, restore, save, cancelEdit, add, check, reassociate));
    editor.addEventListener("input", () => model.edit(item.id, editor.value));
    editor.addEventListener("compositionstart", () => model.compose(item.id, true));
    editor.addEventListener("compositionend", () => { model.edit(item.id, editor.value); model.compose(item.id, false); });
    editor.addEventListener("blur", () => { if (!model.getSnapshot().edits.get(item.id)?.composing) void model.saveEdit(item.id, false); });
    let sourceStamp = "";
    const drawSources = (sources: InsightSource[], target: HTMLElement) => {
      target.replaceChildren(...sources.map(source => {
        const button = h("button", { class: "text-button insight-source-open", onclick: async () => {
          const position = root.scrollTop;
          let matched = false;
          try { matched = await host.openSource(source, button, () => { root.scrollTop = position; button.focus({ preventScroll: true }); }); } catch { /* Keep the frozen source visible. */ }
          if (!matched) { text(sourceNotice, t("insights.source_changed")); sourceNotice.hidden = false; }
        } }, `${t("insights.open_source")} · ${formatClock(source.start_ms / 1000)}`);
        return h("div", { class: "insight-source" }, button, h("blockquote", {}, source.quote));
      }));
    };
    return { element, update(next: InsightItem) {
      item = next; const state = model.getSnapshot(), draft = state.edits.get(item.id);
      const writable = insightsWritable(state.snapshot) && !state.busy.has("write");
      text(title, t(`insights.kind_${item.kind}`));
      text(metadata, `${t(`insights.review_${item.review_state}`)} · ${t(`insights.commitment_${item.commitment}`)}${item.edited_text !== null ? ` · ${t("insights.edited")}` : ""}`);
      text(body, insightText(item)); body.hidden = !!draft; editor.hidden = !draft;
      if (draft && !draft.composing && editor.value !== draft.text) editor.value = draft.text;
      text(details, `${t("insights.owner")}: ${item.owner?.text ?? t("insights.unknown")} · ${t("insights.due")}: ${item.due?.text ?? t("insights.unknown")}`);
      text(cutoff, t("insights.cutoff", { time: formatClock(item.evidence_cutoff_ms / 1000) }));
      const stamp = JSON.stringify([item.sources, item.source_history, t("insights.open_source")]);
      if (stamp !== sourceStamp) { sourceStamp = stamp; drawSources(item.sources, quotes); drawSources(item.source_history ?? [], historyQuotes); }
      history.hidden = !item.source_history?.length;
      if (item.source_state !== "current" || draft?.conflict) { text(sourceNotice, t(draft?.conflict ? "insights.error_edit_conflict" : "insights.source_changed")); sourceNotice.hidden = false; }
      accept.hidden = item.review_state !== "proposed" || !!draft; dismiss.hidden = item.review_state === "dismissed" || !!draft;
      restore.hidden = item.review_state !== "dismissed" || !!draft; edit.hidden = !!draft;
      for (const button of [accept, dismiss, restore, edit]) button.disabled = !writable;
      save.hidden = cancelEdit.hidden = !draft; save.disabled = !writable || !!draft?.composing || !draft?.text.trim(); cancelEdit.disabled = !writable;
      add.hidden = item.review_state !== "accepted" || !!draft;
      const receipt = insightReceipt(state.snapshot, item.id);
      add.disabled = !writable || item.source_state !== "current" || !!receipt && receipt.state !== "not_inserted";
      insertion.hidden = !receipt; if (receipt) text(insertion, t(`insights.insertion_${receipt.state}`));
      check.hidden = !receipt || !["pending", "needs_review"].includes(receipt.state); check.disabled = !writable;
      reassociate.hidden = item.source_state === "current" || !!draft; reassociate.disabled = !writable;
    } };
  }
  function render() {
    const state = model.getSnapshot(), snapshot = state.snapshot;
    language.value = state.policy; language.setAttribute("aria-label", t("insights.language"));
    find.disabled = !model.canAnalyze; cancel.hidden = !model.analysisBusy; cancel.disabled = state.busy.has("cancel");
    const destinationInfo = safeInsightDestination(state.availability?.endpoint);
    text(destination, t("insights.destination", { destination: destinationInfo.host ? destinationInfo.on_device ? t("insights.this_device", { host: destinationInfo.host }) : destinationInfo.host : t("insights.none"), model: state.availability?.model ?? t("insights.none") }));
    text(availability, t(state.edits.size ? "insights.finish_edit" : state.availability?.available ? "insights.qualified" : insightReasonKey(state.availability?.reason_code)));
    if (snapshot && !insightsWritable(snapshot)) text(availability, t(`insights.store_${snapshot.status}` as TranslationKey));
    error.hidden = !state.error; if (state.error) text(error, t(insightErrorKey(state.error)));
    progress.hidden = !state.job; if (state.job) text(progress, `${t(`insights.job_${state.job.state}`)}${state.job.physical_running && state.job.state === "cancelled" ? ` ${t("insights.worker_finishing")}` : ""}`);
      exportItems.disabled = !insightsWritable(snapshot) || !snapshot?.items.some(item => item.review_state === "accepted") || state.busy.has("export") || state.busy.has("write") || model.dirty;
    text(dismissed, t(showDismissed ? "insights.hide_dismissed" : "insights.show_dismissed"));
    const items = model.items().filter(item => showDismissed || item.review_state !== "dismissed" || state.edits.has(item.id));
    empty.hidden = items.length > 0; const keep = new Set(items.map(item => item.id));
    for (const [id, row] of rows) if (!keep.has(id)) { row.element.remove(); rows.delete(id); }
    let before = list.firstElementChild;
    for (const item of items) {
      let row = rows.get(item.id); if (!row) { row = createCard(item); rows.set(item.id, row); }
      row.update(item); if (before !== row.element) list.insertBefore(row.element, before); before = row.element.nextElementSibling;
    }
  }
  model.subscribe(render); bindLocalization(root, render); render();
  const view: ReviewView = {
    root, model, refresh: () => model.load(), flush: () => model.flush(),
    bind(nextContext, nextHost) {
      ctx = nextContext; host = nextHost; model.id = host.id;
      if (bound === ctx) { void model.load(); return; } bound = ctx; const mine = ++binding;
      const interacted = () => { ++focusEpoch; };
      for (const event of ["pointerdown", "keydown", "input"]) document.addEventListener(event, interacted, true);
      const poll = new VisibilityPoll(750, async () => { if (root.isConnected) await model.poll(); }, () => undefined);
      poll.setVisible(ctx.visible, false); ctx.onVisibilityChange(visible => { if (mine === binding) { poll.setVisible(visible); if (visible && root.isConnected) void model.load(); } });
      const events = listen<{ meeting_uid: string; revision: number; job_id?: string }>("meeting-insights-changed", ({ payload }) => {
        if (mine !== binding || payload.meeting_uid !== host.meetingUid || !root.isConnected) return;
        if (payload.job_id) { if (payload.revision > (model.getSnapshot().job?.revision ?? 0)) void model.poll(); }
        else if (payload.revision > (model.getSnapshot().snapshot?.revision ?? 0)) void model.load();
      });
      ctx.onFlush(async () => { if (!await model.flush()) throw new Error(t("insights.unsaved_edit")); });
      ctx.onDispose(() => { poll.dispose(); for (const event of ["pointerdown", "keydown", "input"]) document.removeEventListener(event, interacted, true); void events.then(stop => stop());
        if (mine === binding && !model.dirty && !model.analysisBusy) retained.delete(host.meetingUid); });
      void Promise.all([model.load(), model.choosePolicy(model.getSnapshot().policy)]);
    },
  };
  return view;
}
