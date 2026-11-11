import { playMotion } from "./motion";
import { api, type MeetingSummaryDocument, type MeetingSummaryRecord, type MeetingSpeakerTimeline, type MeetingTranscriptProvenance, type SummaryAction, type SummarySource } from "./api";
import { askText, formatClock, h, markdown, select } from "./dom";
import { bindLocalization, formatDateTime, l, t, type TranslationKey } from "./i18n";
import { icon } from "./icons";
import { templateLabel } from "./meeting-labels";
import { laneSpans, SUMMARY_LENGTH_KEY, summaryLength, summarySections, type SummaryLength } from "./meeting-summary-reading-state";
import "./meeting-summary-reading.css";

export function meetingSummaryReading(host: {
  id(): string;
  sourceAvailable(source: SummarySource): boolean;
  openSource(source: SummarySource, invoker: HTMLElement): Promise<void> | void;
  openTemplate?(): void;
  updateAction(document: MeetingSummaryDocument, index: number, change: { done?: boolean; owner?: string | null }): Promise<void>;
  transcriptProvenance?(id: string): Promise<MeetingTranscriptProvenance | null>;
  report(error: unknown): void;
}) {
  let document: MeetingSummaryDocument | null = null, record: MeetingSummaryRecord | null = null, busy = true;
  let length: SummaryLength = "standard";
  let persistentLength = true;
  try { length = summaryLength(localStorage.getItem(SUMMARY_LENGTH_KEY)); } catch { persistentLength = false; }
  const lengthControl = select<SummaryLength>([["brief", l("meetings.structured.brief")], ["standard", l("meetings.structured.standard")], ["detailed", l("meetings.structured.detailed")]], length, value => {
    length = value; try { localStorage.setItem(SUMMARY_LENGTH_KEY, value); persistentLength = true; } catch { persistentLength = false; }
    updateVisibility(); if (value === "detailed") void loadTimeline();
  }, "segmented");
  lengthControl.classList.add("meeting-summary-length");
  const template = h("button", { class: "text-button meeting-summary-template", onclick: () => host.openTemplate?.() });
  const fallback = h("div", { class: "meeting-summary-markdown" });
  const lead = h("div", { class: "structured-summary-lead summary-section", style: "--summary-order:0" });
  const topics = h("section", { class: "structured-summary-topics summary-section", style: "--summary-order:2" });
  const decisions = h("section", { class: "structured-summary-decisions summary-section", style: "--summary-order:3" });
  const actions = h("section", { class: "structured-summary-actions summary-section", style: "--summary-order:4" });
  const open = h("section", { class: "structured-summary-open summary-section", style: "--summary-order:5" });
  const mapTopics = h("div", { class: "summary-map-topics" });
  const lanes = h("div", { class: "summary-map-lanes" });
  const mapStatus = h("p", { class: "help summary-map-status", role: "status" });
  const mapRetry = h("button", { class: "text-button", hidden: true, onclick: () => { timelineKey = ""; void loadTimeline(); } }, l("common.retry"));
  const map = h("section", { class: "structured-summary-map summary-section", style: "--summary-order:1" },
    h("div", { class: "summary-map-heading" }, h("h3", {}, l("meetings.structured.map")), h("p", { class: "help" }, l("meetings.structured.map_help"))), mapTopics, lanes, mapStatus, mapRetry);
  const structured = h("div", { class: "meeting-summary-structured", hidden: true }, lead, map, topics, decisions, actions, open);
  const root = h("div", { class: "meeting-summary-reading" }, fallback, structured);
  const provenance = h("div", { class: "meeting-summary-provenance", "aria-label": l("dictation.processing.title") });
  const summaryChip = h("span", { class: "summary-provenance-chip", hidden: true });
  const transcriptChip = h("span", { class: "summary-provenance-chip meeting-transcript-provenance", hidden: true });
  provenance.append(transcriptChip, summaryChip);
  const actionRows = new Map<string, { row: HTMLElement; checkbox: HTMLInputElement; owner: HTMLButtonElement; action: SummaryAction; index: number; document: MeetingSummaryDocument }>();
  const sourceLinks = new Map<HTMLButtonElement, SummarySource>();
  let bodyKey = "", contentKey = "", metadataKey = "", timelineKey = "", timelineGeneration = 0, metadataGeneration = 0;
  let timeline: MeetingSpeakerTimeline | null = null, timelineLoading = false, timelineError = false, transcript: MeetingTranscriptProvenance | null = null;
  let templates: Awaited<ReturnType<typeof api.meetingTemplates>> = [];
  let ownerDialog = false;
  void api.meetingTemplates().then(values => { templates = values; updateMetadata(); }).catch(() => undefined);

  function providerLabel(kind: string): string {
    if (kind === "local") return t("meetings.structured.local_model");
    if (kind === "apple_speech" || kind === "apple") return t("onboarding.apple_dictation");
    if (kind === "apple_intelligence" || kind === "apple-intelligence") return "Apple Intelligence";
    return ({ groq: "Groq", open_ai: "OpenAI", openai: "OpenAI", open_router: "OpenRouter", openrouter: "OpenRouter", pennel_cloud: "Pennel Cloud", "pennel-cloud": "Pennel Cloud", mistral: "Mistral", ollama: "Ollama", lm_studio: "LM Studio", lmstudio: "LM Studio", claude_code: "Claude Code", "claude-code": "Claude Code", codex: "Codex", copilot: "Copilot", open_code: "OpenCode", opencode: "OpenCode", gemini_cli: "Gemini CLI", "gemini-cli": "Gemini CLI", cursor_agent: "Cursor", cursor: "Cursor" } as Record<string, string>)[kind] ?? t("meetings.structured.chosen_provider");
  }
  function chip(node: HTMLElement, kind: string, model: string, generated: string, key: "meetings.structured.transcribed" | "meetings.structured.generated") {
    node.hidden = false;
    node.replaceChildren(icon(["local", "apple", "apple_speech", "apple_intelligence", "apple-intelligence"].includes(kind) ? "laptop" : "globe"), h("span", {}, t(key, { provider: providerLabel(kind) }), model ? ` · ${model}` : ""));
    node.title = t("meetings.structured.generated_at", { date: formatDateTime(generated) });
  }
  function updateMetadata() {
    lengthControl.hidden = !record?.structure;
    lengthControl.setAttribute("aria-label", t("meetings.structured.length"));
    const id = record?.provenance.template_id;
    const known = templates.find(value => value.id === id);
    template.textContent = id ? known ? templateLabel(known) : id : t("meetings.structured.template_unknown");
    template.disabled = !host.openTemplate || busy;
    template.title = t("meetings.structured.template_help");
    summaryChip.hidden = !record;
    if (record) chip(summaryChip, record.provenance.provider_kind, record.provenance.model, record.provenance.generated_at, "meetings.structured.generated");
    transcriptChip.hidden = !transcript;
    if (transcript) chip(transcriptChip, transcript.provider_kind, transcript.model, transcript.generated_at, "meetings.structured.transcribed");
    provenance.hidden = !record && !transcript;
  }
  function sourceButton(source: SummarySource | null, text?: string): HTMLElement | null {
    if (!source) return null;
    const button = h("button", { class: "text-button structured-source", disabled: !!document?.outdated || !host.sourceAvailable(source), onclick: () => { button.focus({ preventScroll: true }); void Promise.resolve(host.openSource(source, button)).catch(host.report); } },
      text ?? [source.speaker, formatClock(source.start_ms / 1000)].filter(Boolean).join(" · "));
    button.title = t(button.disabled ? "meetings.source_changed" : "meetings.structured.open_source");
    sourceLinks.set(button, source);
    return button;
  }
  function heading(label: TranslationKey, name: "check" | "notebook" | "chat") { return h("h3", {}, icon(name), l(label)); }
  let fallbackValue: string | null = null;
  function renderBody() {
    const structure = record?.structure;
    fallback.hidden = !!structure; structured.hidden = !structure;
    if (!structure || !document) {
      const value = document?.markdown ?? "";
      if (fallbackValue !== value) {
        fallbackValue = value;
        fallback.replaceChildren(value ? markdown(value) : h("p", { class: "help" }, l("meetings.summary_empty")));
        if (value) [...fallback.querySelectorAll<HTMLElement>(".markdown > *")].forEach((section, order) => { section.style.setProperty("--summary-order", String(order)); playMotion(section, "motion-summary"); });
      }
      return;
    }
    const nextContent = JSON.stringify([structure.lead, structure.topics, structure.decisions, structure.actions.map(({ state: _state, ...item }) => item), structure.open_questions]);
    const reveal = contentKey !== nextContent;
    contentKey = nextContent;
    if (reveal) {
      lead.replaceChildren(h("p", {}, structure.lead));
      topics.replaceChildren(heading("meetings.structured.discussed", "notebook"), ...structure.topics.map(topic => h("section", { class: "structured-topic" }, h("div", { class: "structured-topic-heading" }, h("h4", {}, topic.title), sourceButton(topic.source)), h("p", {}, topic.body))));
      decisions.replaceChildren(heading("meetings.structured.decisions", "check"), ...structure.decisions.map(item => h("div", { class: "structured-decision" }, h("span", {}, item.text), sourceButton(item.source))));
      open.replaceChildren(heading("meetings.structured.open", "chat"), ...structure.open_questions.map(item => h("div", { class: "structured-open-item" }, h("span", {}, item.text), sourceButton(item.source))));
      mapTopics.replaceChildren(...structure.topics.flatMap(topic => topic.source ? [sourceButton(topic.source, `${topic.title} · ${formatClock(topic.source.start_ms / 1000)}`)!] : []));
      for (const section of [lead, map, topics, decisions, actions, open]) playMotion(section, "motion-summary");
    }
    if (!actions.querySelector("h3") || reveal) actions.querySelector("h3")?.remove();
    if (!actions.querySelector("h3")) actions.prepend(heading("meetings.structured.actions", "notebook"));
    const keys = new Set<string>();
    structure.actions.forEach((action, index) => {
      const key = JSON.stringify([index, action.text, action.due, action.source]); keys.add(key);
      let row = actionRows.get(key);
      if (!row) {
        const checkbox = h("input", { type: "checkbox", checked: action.state.done });
        const owner = h("button", { class: "structured-action-owner text-button" });
        const entry = { row: h("div", { class: "structured-action" }), checkbox, owner, action, index, document: document! };
        checkbox.addEventListener("change", () => { void host.updateAction(entry.document, entry.index, { done: checkbox.checked }).catch(host.report); });
        owner.addEventListener("click", async () => {
          const before = entry.document;
          ownerDialog = true;
          let value: string | null;
          try { value = await askText(l("meetings.structured.assign_owner"), entry.action.state.owner ?? entry.action.owner ?? "", l("meetings.structured.owner_help")); }
          finally { ownerDialog = false; }
          if (value !== null) { try { await host.updateAction(before, entry.index, { owner: value || null }); } catch (error) { host.report(error); } }
          if (owner.isConnected) owner.focus({ preventScroll: true });
        });
        const source = sourceButton(action.source);
        entry.row.append(h("label", {}, checkbox, h("span", { class: "structured-action-marker", "aria-hidden": true }, icon("check")), h("span", { class: "structured-action-text" }, action.text)), owner,
          ...action.due ? [h("span", { class: "structured-action-due help" }, action.due)] : [], ...source ? [source] : []);
        actionRows.set(key, entry); row = entry;
      }
      row.action = action; row.index = index; row.document = document!;
      row.checkbox.checked = action.state.done; row.owner.textContent = action.state.owner ?? action.owner ?? t("meetings.structured.add_owner");
      row.row.classList.toggle("structured-action-done", action.state.done);
      if (actions.children[index + 1] !== row.row) actions.insertBefore(row.row, actions.children[index + 1] ?? null);
    });
    for (const [key, row] of actionRows) if (!keys.has(key)) { row.row.remove(); actionRows.delete(key); }
    for (const [section, count] of [[decisions, structure.decisions.length], [actions, structure.actions.length], [open, structure.open_questions.length]] as const) {
      section.querySelector(".structured-none")?.remove();
      if (!count) section.append(h("p", { class: "help structured-none" }, structure.labels.none));
    }
  }
  function updateVisibility() {
    const shown = summarySections(length);
    const focused = root.ownerDocument.activeElement;
    const restoreFocus = !shown.map && map.contains(focused) || !shown.topics && topics.contains(focused) || !shown.open && open.contains(focused);
    for (const [section, visible] of [[map, shown.map], [topics, shown.topics], [open, shown.open]] as const) {
      const arriving = section.hidden && visible;
      section.hidden = !visible;
      if (arriving) {
        playMotion(section, "motion-summary");
        if (section === map) lanes.querySelectorAll<HTMLElement>(".summary-map-span").forEach(span => playMotion(span, "motion-map-draw"));
      }
    }
    if (restoreFocus && lengthControl.isConnected) lengthControl.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus({ preventScroll: true });
    root.dataset.summaryLength = length;
  }
  let renderedTimeline = "", timelineData = "";
  function renderTimeline() {
    mapStatus.textContent = timelineLoading ? t("common.loading") : timelineError ? t("meetings.structured.timeline_error")
      : timeline ? t(timeline.duration_is_recorded ? "meetings.structured.recorded_extent" : "meetings.structured.observed_extent", { time: formatClock(timeline.duration_ms / 1000) }) : "";
    mapRetry.hidden = !timelineError;
    if (!timeline) return;
    const nextTimeline = JSON.stringify([timeline, t("meetings.structured.map")]);
    if (renderedTimeline === nextTimeline && lanes.childElementCount) return;
    const arrive = timelineData !== JSON.stringify(timeline);
    renderedTimeline = nextTimeline; timelineData = JSON.stringify(timeline);
    lanes.replaceChildren(...timeline.lanes.map((lane, index) => {
      const label = lane.kind === "person" && lane.speaker ? lane.speaker : t(lane.kind === "mixed" ? "meetings.attribution_mixed" : lane.kind === "overlap" ? "meetings.attribution_overlap" : "meetings.attribution_unknown");
      const track = h("span", { class: "summary-map-track", "aria-hidden": true }, ...laneSpans(lane, timeline!.duration_ms).map(span => h("span", { style: `left:${span.left}%;width:${span.width}%;--map-order:${index};--map-wait:860ms`, class: "summary-map-span" })));
      return h("div", { class: "summary-map-lane", "data-lane-kind": lane.kind, style: `--lane-color:var(--${lane.kind === "person" ? `voice-${index % 4 + 1}` : "muted"})` }, h("span", { class: "summary-map-speaker" }, label), track,
        h("span", { class: "help summary-map-duration" }, formatClock(lane.duration_ms / 1000)));
    }));
    if (arrive) lanes.querySelectorAll<HTMLElement>(".summary-map-span").forEach(span => playMotion(span, "motion-map-draw"));
  }
  async function loadTimeline() {
    if (!document || !record?.structure || length !== "detailed") return;
    const key = `${host.id()}|${document.meeting_uid}|${document.source_revision}`;
    if (timelineKey === key) return;
    timelineKey = key; const generation = ++timelineGeneration;
    timeline = null; lanes.replaceChildren(); timelineLoading = true; timelineError = false; renderTimeline();
    try { const result = await api.meetingSpeakerTimeline(host.id()); if (generation !== timelineGeneration || timelineKey !== key) return; timeline = result; }
    catch { if (generation === timelineGeneration) timelineError = true; }
    finally { if (generation === timelineGeneration) { timelineLoading = false; renderTimeline(); } }
  }
  bindLocalization(root, () => { if (document) { update(document, busy); renderTimeline(); } });
  function update(next: MeetingSummaryDocument | null, disabled: boolean) {
    if (persistentLength) {
      try { const saved = localStorage.getItem(SUMMARY_LENGTH_KEY); if (saved !== null) { length = summaryLength(saved); lengthControl.value = length; } }
      catch { persistentLength = false; }
    }
    document = next; record = next?.record ?? null; busy = disabled;
    updateMetadata();
    const key = JSON.stringify([next?.meeting_uid, next?.markdown, record, next?.outdated]);
    if (key !== bodyKey) { bodyKey = key; renderBody(); }
    for (const row of actionRows.values()) {
      if (next) row.document = next;
      if (!busy) row.checkbox.checked = row.action.state.done;
      row.owner.textContent = row.action.state.owner ?? row.action.owner ?? t("meetings.structured.add_owner");
    }
    for (const [button, source] of sourceLinks) {
      if (!root.contains(button)) { sourceLinks.delete(button); continue; }
      button.disabled = busy || !!document?.outdated || !host.sourceAvailable(source);
      button.title = t(button.disabled ? "meetings.source_changed" : "meetings.structured.open_source");
    }
    for (const control of structured.querySelectorAll<HTMLInputElement | HTMLButtonElement>('input,button:not(.structured-source)')) control.disabled = busy;
    updateVisibility();
    const dataKey = next ? `${host.id()}|${next.meeting_uid}|${next.source_revision}` : "";
    if (dataKey !== metadataKey) {
      ++timelineGeneration; timelineKey = ""; timeline = null; lanes.replaceChildren();
      metadataKey = dataKey; transcript = null; updateMetadata(); const generation = ++metadataGeneration;
      if (next && host.transcriptProvenance) void host.transcriptProvenance(host.id()).then(result => { if (generation === metadataGeneration && metadataKey === dataKey) { transcript = result; updateMetadata(); } }).catch(() => undefined);
    }
    void loadTimeline();
  }
  return { root, lengthControl, template, provenance, update, interacting: () => ownerDialog };
}
