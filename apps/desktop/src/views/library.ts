import { bindLocalization, getInterfaceLanguage, l, t, formatNumber, formatDateTime } from "../i18n.ts";
import { api, type AppInfo, type MeetingInfo, type MeetingRuntimeStatus, type Passage } from "../api";
import { LatestRequest, SerialQueue } from "../async";
import { textField, askText, confirmDialog, h, showPopupMenu, snippet, type PopupMenuItem } from "../dom";
import { meetingDate, dayGroup } from "../meeting-labels.ts";
export { meetingDate, dayGroup } from "../meeting-labels.ts";
import { icon } from "../icons";
import { badgeFor, resolveOpenMeeting, syncActive, type SyncItemState } from "../sync-badge";
import { historyRoute, workspaceRoute, workspaceTask, type WorkspaceRoute, type WorkspaceTask } from "../workspace-state";
import { TaskSearchQueries } from "../workspace-search-state";
import { matchingDictations } from "../dictation-home-state";
import { recordingEntry } from "../voice-flow-state";
import { ListTypeahead, listKeyIndex, reconcileChildren } from "../library-list.ts";

export interface LibraryHost {
  info: () => AppInfo;
  current: () => WorkspaceRoute;
  go: (route: WorkspaceRoute) => void;
  select?: (section: WorkspaceRoute["section"]) => void;
  report: (error: unknown) => void;
  statusChanged?: (status: MeetingRuntimeStatus) => void;
}

interface LibraryRow {
  element: HTMLElement;
  open: HTMLButtonElement;
  more: HTMLButtonElement;
  title: HTMLElement;
  metadata: HTMLElement;
  badge: HTMLElement;
  snippets: HTMLElement;
  meeting: MeetingInfo;
  source?: Passage;
  signature: string;
  hits: string;
  sync: string;
}

/** Persistent task actions stay above search and the grouped meeting library. */
export class LibrarySidebar {
  readonly element: HTMLElement;
  private readonly search: HTMLInputElement;
  private readonly ask: HTMLButtonElement;
  private readonly live: HTMLButtonElement;
  private readonly liveTitle: HTMLElement;
  private readonly liveState: HTMLElement;
  private readonly liveSymbol: HTMLElement;
  private readonly list: HTMLElement;
  private readonly dictationNavigation: HTMLElement;
  private readonly dictationResults: HTMLElement;
  private task: WorkspaceTask;
  private readonly queries: TaskSearchQueries;
  private readonly requests = new LatestRequest();
  private readonly syncQueue = new SerialQueue();
  private readonly typeahead = new ListTypeahead();
  private readonly rows = new Map<string, LibraryRow>();
  private readonly headings = new Map<string, HTMLElement>();
  private query = "";
  private searchTimer: number | undefined;
  private syncOn = false;
  private syncStates = new Map<string, SyncItemState>();
  private shown: MeetingInfo[] = [];
  private passages: Passage[] = [];
  private menu: { close: (restoreFocus?: boolean) => void } | null = null;
  private menuRow: LibraryRow | null = null;
  private status: MeetingRuntimeStatus | null = null;
  private openingRecording = false;
  private visible = true;
  private disposed = false;
  private started = false;
  private dirty = true;
  private refreshing: Promise<void> | null = null;
  private pendingRoute: WorkspaceRoute | null = null;
  private syncChanged = false;
  private unsubscribe: (() => void) | null = null;

  constructor(private readonly host: LibraryHost) {
    this.task = workspaceTask(host.current());
    this.queries = new TaskSearchQueries(this.task);
    const select = (section: WorkspaceRoute["section"]) => host.select ? host.select(section) : host.go(workspaceRoute(section));
    this.search = textField({ type: "search", placeholder: l("common.search"), "aria-label": l("meetings.search_label"), spellcheck: false });
    this.search.addEventListener("input", () => {
      this.query = this.search.value;
      this.queries.set(this.query);
      window.clearTimeout(this.searchTimer);
      this.searchTimer = window.setTimeout(() => { void this.invalidate().catch(host.report); }, 180);
    });
    this.ask = h("button", { class: "library-ask", onclick: () => host.go(workspaceRoute("meetings", "chat")) }, icon("chat"), h("span", {}, l("meetings.ask_all")));
    this.liveTitle = h("span", { class: "library-live-title" }, t("meetings.new_recording"));
    this.liveState = h("span", { class: "help library-record-state", hidden: true }, l("meetings.finishing"));
    this.liveSymbol = h("span", { class: "library-record-symbol" }, icon("mic"));
    this.live = h("button", { class: "library-record library-live", onclick: () => { void this.openRecording(); } }, this.liveSymbol, this.liveTitle, this.liveState);
    this.list = h("div", { class: "meeting-list", role: "list" });
    this.list.addEventListener("keydown", (event) => this.listKeydown(event));
    this.list.addEventListener("focusin", () => this.highlight());
    this.dictationNavigation = h("nav", { class: "dictation-navigation", "aria-label": l("navigation.dictation") },
      h("button", { class: "text-button", "data-workspace-section": "history", onclick: () => select("history") }, icon("history"), l("dictation.history.title")),
      h("button", { class: "text-button", "data-dictation-section": "pad", onclick: () => host.go(workspaceRoute("dictation", "pad")) }, l("dictation.home.open_pad")));
    this.dictationResults = h("div", { class: "dictation-search-results", role: "list", hidden: true });
    this.element = h("aside", { class: "library-sidebar", "aria-label": l("meetings.library_label") },
      h("div", { class: "library-actions" }, this.live),
      h("div", { class: "library-shortcuts" },
        h("button", { class: "text-button", "data-workspace-section": "dictation", onclick: () => select("dictation") }, icon("mic"), l("meetings.open_dictation")),
        this.dictationNavigation,
        h("button", { class: "text-button", "data-workspace-section": "meetings", onclick: () => select("meetings") }, icon("notebook"), l("navigation.meetings"))),
      h("div", { class: "library-search" }, h("div", { class: "library-search-control" }, icon("search"), this.search)), this.ask, this.list, this.dictationResults,
      h("button", { class: "text-button library-settings", onclick: () => host.go(workspaceRoute("settings")) }, icon("settings"), l("common.settings")));
    bindLocalization(this.live, () => { if (this.status) this.showLive(this.status); });
    bindLocalization(this.search, () => this.syncTask());
    bindLocalization(this.dictationResults, () => { if (this.task === "dictation" && this.query.trim()) void this.invalidate().catch(this.host.report); });
    bindLocalization(this.list, () => { if (!this.disposed) this.draw(this.shown, this.passages, this.query.trim()); });
    void api.onSyncChanged((report) => {
      if (this.disposed) return;
      void this.syncQueue.run(async () => {
        const wasOn = this.syncOn;
        this.syncOn = syncActive(this.host.info().settings.cloud_sync === true, report.status.kind);
        if (!this.syncOn && !wasOn) return;
        this.syncChanged = true;
        await this.invalidate();
      }).catch(this.host.report);
    }).then((unsubscribe) => { if (this.disposed) unsubscribe(); else this.unsubscribe = unsubscribe; }).catch(() => undefined);
    this.syncTask();
  }

  focusSearch(): void { this.syncTask(); this.search.focus(); this.search.select(); }

  private syncTask(): void {
    const task = workspaceTask(this.host.current());
    if (task !== this.task) {
      this.task = task;
      this.query = this.queries.select(task);
      this.search.value = this.query;
      this.requests.cancel(); this.closeMenu(); this.dirty = true;
      window.clearTimeout(this.searchTimer);
      if (this.started && this.visible) void this.refresh().catch(this.host.report);
    }
    this.element.dataset.workspaceTask = task;
    this.dictationNavigation.hidden = task !== "dictation";
    this.dictationResults.hidden = task !== "dictation" || !this.query.trim();
    this.ask.hidden = task !== "meetings";
    this.list.hidden = task !== "meetings";
    const searchLabel = t(task === "dictation" ? "dictation.history.search" : "meetings.search_label");
    this.search.placeholder = searchLabel;
    this.search.setAttribute("aria-label", searchLabel);
    if (this.status) this.showLive(this.status); else this.live.hidden = task !== "meetings";
  }

  async start() {
    if (!this.started) {
      this.started = true;
      this.syncOn = this.host.info().settings.cloud_sync === true
        && syncActive(true, (await api.cloudSyncLast().catch(() => null))?.status.kind);
    }
    await this.refresh();
  }

  setVisible(visible: boolean): void {
    if (this.disposed || this.visible === visible) return;
    this.visible = visible;
    if (!visible) {
      this.requests.cancel();
      this.dirty = true;
      window.clearTimeout(this.searchTimer);
      this.closeMenu();
    } else if (this.started && this.dirty) void this.refresh().catch(this.host.report);
  }

  setPendingRoute(route: WorkspaceRoute | null): void {
    this.pendingRoute = route;
    this.highlight();
  }

  invalidate(): Promise<void> {
    return this.refresh();
  }

  async refresh(): Promise<void> {
    if (this.disposed) return;
    this.dirty = true;
    if (!this.visible) return;
    if (this.refreshing) return this.refreshing;
    this.refreshing = this.refreshPending().finally(() => { this.refreshing = null; });
    return this.refreshing;
  }

  private async refreshPending(): Promise<void> {
    while (this.dirty && this.visible && !this.disposed) {
      this.dirty = false;
      const query = this.query.trim();
      const task = this.task;
      if (task === "dictation") {
        await this.requests.run(async () => ({ entries: query ? await api.history(200) : [], status: await api.meetingRuntimeStatus() }), ({ entries, status }) => {
          if (task !== this.task || query !== this.query.trim()) { this.dirty = true; return; }
          this.showLive(status);
          const matches = matchingDictations(entries, query);
          this.dictationResults.hidden = !query;
          this.dictationResults.replaceChildren(...matches.map((entry) => h("div", { role: "listitem" },
            h("button", { class: "dictation-search-result", "data-history-result": entry.id, onclick: () => this.host.go(historyRoute(entry.id)) },
              h("span", {}, entry.text || entry.raw), h("small", { class: "help" }, formatDateTime(entry.created_at_ms, { dateStyle: "medium", timeStyle: "short" }))))),
            ...(query && !matches.length ? [h("p", { class: "library-empty help", role: "status" }, l("dictation.history.none_found"))] : []));
          this.highlight();
        }, this.host.report);
        continue;
      }
      const route = this.host.current();
      const open = route.section === "meetings" ? route.meeting_id : null;
      const remembered = this.shown.find((meeting) => meeting.id === open) ?? null;
      const resolveRemote = this.syncChanged;
      const states = this.syncOn ? api.meetingSyncStates().catch(() => []) : Promise.resolve([]);
      await this.requests.run(async () => {
        const [result, sync, status] = await Promise.all([
          query ? api.searchLibrary(query) : api.meetings("").then((meetings) => ({ meetings, passages: [] as Passage[] })),
          states, api.meetingRuntimeStatus(),
        ]);
        return { result, sync, status };
      }, ({ result, sync, status }) => {
        if (query !== this.query.trim()) { this.dirty = true; return; }
        this.syncStates = new Map(sync.map((item) => [item.id, item.state]));
        this.shown = result.meetings;
        this.passages = result.passages;
        this.showLive(status);
        this.draw(result.meetings, result.passages, query);
        this.highlight();
        if (resolveRemote) {
          this.syncChanged = false;
          if (open && route === this.host.current()) {
            const resolved = resolveOpenMeeting(open, this.shown, remembered);
            if (resolved !== open) this.host.go(workspaceRoute("meetings", null, resolved ?? null));
          }
        }
      }, this.host.report);
    }
  }

  async refreshLive() {
    const status = await api.meetingRuntimeStatus();
    if (!this.disposed) this.showLive(status);
  }

  meetings(): MeetingInfo[] {
    return this.shown;
  }

  highlight() {
    this.syncTask();
    const route = this.pendingRoute ?? this.host.current();
    for (const button of this.element.querySelectorAll<HTMLButtonElement>("[data-workspace-section]")) {
      const current = button.dataset.workspaceSection === "dictation" ? workspaceTask(route) === "dictation" : button.dataset.workspaceSection === "meetings" ? workspaceTask(route) === "meetings" : button.dataset.workspaceSection === route.section;
      if (current) button.setAttribute("aria-current", "page"); else button.removeAttribute("aria-current");
    }
    for (const button of this.dictationNavigation.querySelectorAll<HTMLButtonElement>("[data-dictation-section]")) {
      if (route.section === "dictation" && button.dataset.dictationSection === (route.tab === "pad" ? "pad" : "home")) button.setAttribute("aria-current", "page"); else button.removeAttribute("aria-current");
    }
    const chat = route.section === "meetings" && route.tab === "chat";
    this.ask.toggleAttribute("aria-current", chat);
    this.live.toggleAttribute("aria-current", route.section === "recording");
    const focused = document.activeElement?.closest<HTMLElement>(".meeting-row")?.dataset.id;
    const tabStop = focused && this.rows.has(focused) ? focused : route.meeting_id && this.rows.has(route.meeting_id) ? route.meeting_id : this.shown[0]?.id;
    for (const [id, row] of this.rows) {
      const selected = !chat && route.section === "meetings" && id === route.meeting_id;
      row.element.classList.toggle("selected", selected);
      row.open.toggleAttribute("aria-current", selected);
      row.open.tabIndex = id === tabStop ? 0 : -1;
      row.more.tabIndex = id === tabStop ? 0 : -1;
    }
  }

  private showLive(status: MeetingRuntimeStatus) {
    this.status = status;
    this.host.statusChanged?.(status);
    const entry = recordingEntry(status);
    const label = t(entry.kind === "new" ? "meetings.new_recording" : entry.kind === "recovery" ? "meetings.recording_needs_attention"
      : entry.kind === "finishing" ? "meetings.return_finishing" : "meetings.return_recording");
    if (this.live.dataset.phase !== entry.kind) {
      this.live.dataset.phase = entry.kind;
      this.liveSymbol.replaceChildren(entry.kind === "new" ? icon("mic") : entry.kind === "recovery" ? icon("warning") : h("span", { class: "status-dot" }));
    }
    if (this.liveTitle.textContent !== label) this.liveTitle.textContent = label;
    this.liveState.hidden = entry.kind !== "finishing";
    this.live.disabled = this.openingRecording;
    this.live.hidden = entry.kind === "new" && this.task !== "meetings";
    this.live.title = entry.kind === "new" ? "" : status.title ?? "";
  }

  private async openRecording() {
    if (this.openingRecording) return;
    this.openingRecording = true;
    this.live.disabled = true;
    try {
      const status = await api.meetingRuntimeStatus();
      this.showLive(status);
      this.host.go(workspaceRoute("recording", null, recordingEntry(status).meetingId));
    } catch (error) { this.host.report(error); }
    finally { this.openingRecording = false; this.live.disabled = false; }
  }

  private draw(meetings: MeetingInfo[], passages: Passage[], query: string) {
    const wanted: HTMLElement[] = [];
    const retained = new Set(meetings.map((meeting) => meeting.id));
    const focused = this.list.contains(document.activeElement) ? document.activeElement as HTMLElement : null;
    const focusedId = focused?.closest<HTMLElement>(".meeting-row")?.dataset.id;
    const focusedIndex = focusedId ? [...this.list.querySelectorAll<HTMLElement>(".meeting-row")].findIndex((row) => row.dataset.id === focusedId) : -1;
    const selection = document.getSelection();
    const selected = selection?.anchorNode && selection.focusNode && this.list.contains(selection.anchorNode) && this.list.contains(selection.focusNode)
      ? { anchor: selection.anchorNode, anchorOffset: selection.anchorOffset, focus: selection.focusNode, focusOffset: selection.focusOffset } : null;
    const grouped = new Set<string>();
    for (const [id, row] of this.rows) {
      if (retained.has(id)) continue;
      if (row === this.menuRow) this.closeMenu();
      this.rows.delete(id);
    }
    const hitsByMeeting = new Map<string, Passage[]>();
    for (const passage of passages) {
      const hits = hitsByMeeting.get(passage.meeting_id) ?? [];
      if (hits.length >= 2) continue;
      hits.push(passage);
      hitsByMeeting.set(passage.meeting_id, hits);
    }
    for (const meeting of meetings) {
      const day = dayGroup(meeting.start_iso);
      if (!grouped.has(day)) {
        grouped.add(day);
        let heading = this.headings.get(day);
        if (!heading) { heading = h("h3", { class: "meeting-day" }, day); this.headings.set(day, heading); }
        wanted.push(heading);
      }
      let row = this.rows.get(meeting.id);
      if (!row) { row = this.createRow(meeting); this.rows.set(meeting.id, row); }
      row.meeting = meeting;
      const hits = hitsByMeeting.get(meeting.id) ?? [];
      row.source = hits[0];
      const signature = JSON.stringify([getInterfaceLanguage(), meeting.title, meeting.start_iso, meeting.duration_min]);
      if (row.signature !== signature) {
        row.signature = signature;
        if (row.title.textContent !== meeting.title) row.title.textContent = meeting.title;
        row.metadata.textContent = `${meetingDate(meeting.start_iso)}${meeting.duration_min ? ` · ${t("meetings.duration_minutes", { count: formatNumber(meeting.duration_min) })}` : ""}`;
        row.more.setAttribute("aria-label", t("meetings.actions_for", { title: meeting.title }));
      }
      const syncState = this.syncStates.get(meeting.id);
      const syncKey = JSON.stringify([getInterfaceLanguage(), syncState, this.syncOn]);
      if (syncKey !== row.sync) {
        row.sync = syncKey;
        const badge = badgeFor(syncState, this.syncOn);
        row.badge.replaceChildren(...(badge ? [h("span", { class: `sync-badge ${syncState}`, title: badge.label, role: "img", "aria-label": badge.label }, icon(badge.icon))] : []));
      }
      const hitKey = JSON.stringify(hits.map((passage) => passage.snippet));
      if (hitKey !== row.hits) { row.hits = hitKey; row.snippets.replaceChildren(...hits.map((passage) => snippet(passage.snippet))); }
      row.snippets.hidden = !hits.length;
      wanted.push(row.element);
    }
    for (const key of this.headings.keys()) if (!grouped.has(key)) this.headings.delete(key);
    if (!meetings.length) {
      const label = t(query ? "meetings.none_found" : "meetings.empty_meetings");
      const empty = this.list.querySelector<HTMLElement>(".library-empty") ?? h("p", { class: "library-empty help" });
      if (empty.textContent !== label) empty.textContent = label;
      wanted.push(empty);
    }
    reconcileChildren<Element>(this.list, wanted);
    if (focused?.isConnected && document.activeElement !== focused) focused.focus({ preventScroll: true });
    else if (focusedId && !retained.has(focusedId)) this.rows.get(meetings[Math.min(Math.max(0, focusedIndex), meetings.length - 1)]?.id)?.open.focus({ preventScroll: true });
    if (selected?.anchor.isConnected && selected.focus.isConnected && selection
      && (selection.anchorNode !== selected.anchor || selection.anchorOffset !== selected.anchorOffset || selection.focusNode !== selected.focus || selection.focusOffset !== selected.focusOffset)) {
      selection.setBaseAndExtent(selected.anchor, selected.anchorOffset, selected.focus, selected.focusOffset);
    }
  }

  private createRow(meeting: MeetingInfo): LibraryRow {
    const title = h("span", { class: "meeting-title" });
    const metadata = h("span", { class: "meeting-meta" });
    const badge = h("span", { class: "meeting-sync" });
    const snippets = h("span", { class: "meeting-snippets" });
    const open = h("button", { class: "meeting-select" }, h("span", { class: "meeting-name" }, title, badge), metadata, snippets);
    const more = h("button", { class: "meeting-more button-icon", "aria-haspopup": "menu", "aria-expanded": "false" }, icon("more"));
    const element = h("div", { class: "meeting-row", role: "listitem", "data-id": meeting.id }, open, more);
    const row: LibraryRow = { element, open, more, title, metadata, badge, snippets, meeting, signature: "", hits: "", sync: "" };
    open.addEventListener("click", () => this.host.go(workspaceRoute("meetings", null, row.meeting.id, row.source?.start_ms ?? null,
      row.source ? { kind: row.source.kind, start_ms: row.source.start_ms, text: row.source.text } : undefined)));
    const menu = (position?: { x: number; y: number }) => this.openMenu(row, position);
    more.addEventListener("click", (event) => { event.stopPropagation(); menu(); });
    more.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); event.stopPropagation(); menu(); }
    });
    element.addEventListener("contextmenu", (event) => { event.preventDefault(); menu({ x: event.clientX, y: event.clientY }); });
    open.addEventListener("keydown", (event) => {
      if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) { event.preventDefault(); event.stopPropagation(); menu(); }
    });
    return row;
  }

  private listKeydown(event: KeyboardEvent): void {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>(".meeting-select");
    if (!button || event.altKey || event.ctrlKey || event.metaKey || event.isComposing) return;
    const current = this.shown.findIndex((meeting) => meeting.id === button.closest<HTMLElement>(".meeting-row")?.dataset.id);
    const next = listKeyIndex(event.key, current, this.shown.length);
    const index = next >= 0 ? next : this.typeahead.find(this.shown.map((meeting) => meeting.title), current, event.key, performance.now(), getInterfaceLanguage());
    if (index < 0) return;
    event.preventDefault();
    const row = this.rows.get(this.shown[index].id);
    row?.open.focus({ preventScroll: true });
    row?.element.scrollIntoView({ block: "nearest" });
    row?.open.click();
  }

  private openMenu(row: LibraryRow, position?: { x: number; y: number }) {
    this.closeMenu();
    const meeting = row.meeting;
    const currentBusy = this.status?.busy && this.status.id === meeting.id;
    const inactive = async () => {
      const status = await api.meetingRuntimeStatus();
      if (status.busy && status.id === meeting.id) throw new Error(t("meetings.recording_active"));
    };
    const items: PopupMenuItem[] = [
      { value: "rename", label: l("meetings.rename_action"), disabled: currentBusy, run: async () => {
        await inactive();
        const title = await askText(l("meetings.rename_title"), meeting.title);
        if (!title) return;
        await inactive();
        const id = await api.renameMeeting(meeting.id, title);
        await this.invalidate();
        this.host.go(workspaceRoute("meetings", null, id));
      } },
      { value: "folder", label: this.host.info().platform === "macos" ? l("meetings.show_finder") : l("meetings.open_folder"), run: async () => {
        await api.openPath((await api.meetingDetail(meeting.id)).dir);
      } },
      { value: "delete", label: l("meetings.delete_action"), disabled: currentBusy, separatorBefore: true, run: async () => {
        await inactive();
        if (!await confirmDialog(l("meetings.delete_title", { title: meeting.title }), l("meetings.delete_help"), l("meetings.delete_confirm"))) return;
        await inactive();
        await api.deleteMeeting(meeting.id);
        const route = this.host.current();
        await this.invalidate();
        if (route.section === "meetings" && route.meeting_id === meeting.id) this.host.go(workspaceRoute("meetings"));
      } },
    ];
    this.menuRow = row;
    this.menu = showPopupMenu(row.more, items, { position, report: this.host.report });
  }

  private closeMenu() {
    this.menu?.close();
    this.menu = null;
    this.menuRow = null;
  }

  dispose(): void {
    this.setVisible(false);
    this.disposed = true;
    this.requests.cancel();
    window.clearTimeout(this.searchTimer);
    this.unsubscribe?.();
    this.unsubscribe = null;
  }
}
