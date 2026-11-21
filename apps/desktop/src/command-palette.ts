import { api, type AppInfo, type MeetingInfo, type ModelChoice } from "./api";
import type { ConfiguredDictationModel } from "./pill-model-state";
import { h } from "./dom";
import { l, t } from "./i18n.ts";
import { icon } from "./icons";
import { allSettingsEntries, searchSettings } from "./settings-search";
import type { SettingsCategory } from "./workspace-state";

export interface CommandPaletteHost {
  info: () => AppInfo;
  recording: () => boolean;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  toggleSidebar: () => void;
  navigate: (tab: string, param?: string) => void;
  openSettingsAnchor: (category: SettingsCategory, tab?: string, anchor?: string) => void;
  selectSpeechModel: (choice: ModelChoice) => Promise<void>;
  report: (error: unknown) => void;
}

interface PaletteItem {
  id: string;
  group: "actions" | "models" | "meetings" | "settings";
  title: string;
  subtitle?: string;
  badge?: string;
  iconName: Parameters<typeof icon>[0];
  run: () => void | Promise<void>;
}

const sameChoice = (a: ModelChoice, b: ModelChoice) => a.provider_id === b.provider_id && a.model === b.model;

export class CommandPalette {
  private dialog: HTMLDialogElement | null = null;
  private input: HTMLInputElement | null = null;
  private list: HTMLElement | null = null;
  private previousFocus: HTMLElement | null = null;
  private items: PaletteItem[] = [];
  private selectedIndex = 0;
  private meetings: MeetingInfo[] = [];
  private speechModels: ConfiguredDictationModel[] = [];
  private loadGeneration = 0;
  private searchTimer: number | undefined;

  constructor(private readonly host: CommandPaletteHost) {}

  get open(): boolean {
    return !!this.dialog?.open;
  }

  toggle(): void {
    if (this.open) this.close();
    else void this.show();
  }

  async show(): Promise<void> {
    if (this.open) {
      this.input?.focus();
      this.input?.select();
      return;
    }
    this.previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const input = h("input", {
      type: "search",
      class: "command-palette-input",
      placeholder: t("command_palette.placeholder"),
      "aria-label": t("command_palette.title"),
      spellcheck: false,
      autocomplete: "off",
    });
    const list = h("div", { class: "command-palette-list", role: "listbox", "aria-label": t("command_palette.title") });
    const dialog = h("dialog", { class: "command-palette", "aria-modal": "true", "aria-label": t("command_palette.title") },
      h("div", { class: "command-palette-header" }, icon("search"), input),
      list,
      h("footer", { class: "command-palette-footer help" }, l("command_palette.hint")),
    );
    this.dialog = dialog;
    this.input = input;
    this.list = list;
    this.selectedIndex = 0;

    input.addEventListener("input", () => {
      this.selectedIndex = 0;
      this.render();
      window.clearTimeout(this.searchTimer);
      const q = input.value.trim();
      this.searchTimer = window.setTimeout(() => { void this.loadMeetings(q); }, 100);
    });
    dialog.addEventListener("keydown", (event) => {
      if (event.isComposing) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        if (this.items.length) this.selectIndex((this.selectedIndex + 1) % this.items.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        if (this.items.length) this.selectIndex((this.selectedIndex - 1 + this.items.length) % this.items.length);
      } else if (event.key === "Home") {
        event.preventDefault();
        if (this.items.length) this.selectIndex(0);
      } else if (event.key === "End") {
        event.preventDefault();
        if (this.items.length) this.selectIndex(this.items.length - 1);
      } else if (event.key === "Enter") {
        const chosen = this.items[this.selectedIndex];
        if (chosen) {
          event.preventDefault();
          this.execute(chosen);
        }
      }
    });
    dialog.addEventListener("click", (event) => {
      if (event.target === dialog) this.close();
    });
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      this.close();
    });
    document.body.append(dialog);
    dialog.showModal();
    input.focus();
    this.render();

    const gen = ++this.loadGeneration;
    const [meetings, models] = await Promise.all([
      api.meetings("").catch(() => [] as MeetingInfo[]),
      api.configuredDictationModels().catch(() => [] as ConfiguredDictationModel[]),
    ]);
    if (gen !== this.loadGeneration || !this.open) return;
    this.meetings = meetings;
    this.speechModels = models;
    this.render();
  }

  close(): void {
    window.clearTimeout(this.searchTimer);
    this.loadGeneration++;
    const dialog = this.dialog;
    this.dialog = null;
    this.input = null;
    this.list = null;
    if (dialog) {
      if (dialog.open) dialog.close();
      dialog.remove();
    }
    if (this.previousFocus?.isConnected) this.previousFocus.focus({ preventScroll: true });
    this.previousFocus = null;
  }

  private async loadMeetings(query: string): Promise<void> {
    const gen = ++this.loadGeneration;
    const meetings = await api.meetings(query).catch(() => [] as MeetingInfo[]);
    if (gen !== this.loadGeneration || !this.open) return;
    this.meetings = meetings;
    this.render();
  }

  private execute(item: PaletteItem): void {
    this.close();
    void Promise.resolve(item.run()).catch(this.host.report);
  }

  private selectIndex(next: number): void {
    this.selectedIndex = next;
    const buttons = this.list?.querySelectorAll<HTMLButtonElement>(".command-palette-item") ?? [];
    buttons.forEach((button, idx) => {
      const active = idx === next;
      button.setAttribute("aria-selected", String(active));
      if (active) button.scrollIntoView({ block: "nearest" });
    });
  }

  private buildItems(rawQuery: string): PaletteItem[] {
    const query = rawQuery.trim().toLocaleLowerCase();
    const tokens = query ? query.split(/\s+/).filter(Boolean) : [];
    const matches = (text: string) => {
      if (!tokens.length) return true;
      const hay = text.toLocaleLowerCase();
      return tokens.every((token) => hay.includes(token));
    };
    const info = this.host.info();
    const isRecording = this.host.recording();
    const actionDefs: PaletteItem[] = [
      {
        id: "action-recording",
        group: "actions",
        title: isRecording ? t("common.stop_recording") : t("common.start_recording"),
        iconName: isRecording ? "square" : "mic",
        run: () => (isRecording ? this.host.stopRecording() : this.host.startRecording()),
      },
      {
        id: "action-ai-models",
        group: "actions",
        title: t("navigation.ai_models"),
        subtitle: t("settings.ai.subtitle"),
        iconName: "settings",
        run: () => this.host.navigate("ai-models"),
      },
      {
        id: "action-dictation",
        group: "actions",
        title: t("shortcuts.action.dictation"),
        iconName: "mic",
        run: () => this.host.navigate("dictation-home"),
      },
      {
        id: "action-dictation-pad",
        group: "actions",
        title: t("dictation.home.open_pad"),
        iconName: "mic",
        run: () => this.host.navigate("dictation-pad"),
      },
      {
        id: "action-meetings",
        group: "actions",
        title: t("shortcuts.action.meetings"),
        iconName: "notebook",
        run: () => this.host.navigate("meetings"),
      },
      {
        id: "action-ask-all",
        group: "actions",
        title: t("meetings.ask_all"),
        iconName: "chat",
        run: () => this.host.navigate("chat"),
      },
      {
        id: "action-history",
        group: "actions",
        title: t("shortcuts.action.history"),
        iconName: "history",
        run: () => this.host.navigate("history"),
      },
      {
        id: "action-sidebar",
        group: "actions",
        title: t("shortcuts.action.toggle_sidebar"),
        iconName: "sidebar",
        run: () => this.host.toggleSidebar(),
      },
    ];
    const actions = actionDefs.filter((item) => matches(`${item.title} ${item.subtitle ?? ""} record opname dicteren meeting model`));

    const modelItems: PaletteItem[] = this.speechModels
      .filter((model) => matches(`${model.label} ${model.choice.model} speech spraak model`))
      .slice(0, query ? 8 : 4)
      .map((model) => {
        const active = sameChoice(model.choice, info.settings.dictation_stt);
        return {
          id: `model-${model.choice.provider_id}-${model.choice.model}`,
          group: "models",
          title: model.label,
          badge: active ? t("command_palette.model_active") : undefined,
          iconName: "audioLines",
          run: () => this.host.selectSpeechModel(model.choice),
        };
      });

    const meetingItems: PaletteItem[] = this.meetings
      .filter((meeting) => matches(`${meeting.title} ${meeting.id}`))
      .slice(0, query ? 8 : 5)
      .map((meeting) => ({
        id: `meeting-${meeting.id}`,
        group: "meetings",
        title: meeting.title || meeting.id,
        subtitle: meeting.start_iso ? meeting.start_iso.slice(0, 10) : undefined,
        iconName: "notebook",
        run: () => this.host.navigate("meetings", meeting.id),
      }));

    const settingMatches = query ? searchSettings(query).slice(0, 8) : allSettingsEntries().slice(0, 4);
    const settingItems: PaletteItem[] = settingMatches.map(({ entry, title, categoryLabel }) => ({
      id: `setting-${entry.id}`,
      group: "settings",
      title,
      subtitle: categoryLabel,
      iconName: "settings",
      run: () => this.host.openSettingsAnchor(entry.category, entry.tab, entry.anchor),
    }));

    return query
      ? [...actions, ...settingItems, ...modelItems, ...meetingItems]
      : [...actions.slice(0, 5), ...meetingItems, ...modelItems, ...settingItems];
  }

  private render(): void {
    const list = this.list;
    if (!list) return;
    this.items = this.buildItems(this.input?.value ?? "");
    if (this.selectedIndex >= this.items.length) this.selectedIndex = Math.max(0, this.items.length - 1);
    if (!this.items.length) {
      list.replaceChildren(h("p", { class: "help command-palette-empty" }, l("command_palette.empty")));
      return;
    }
    const groups: PaletteItem["group"][] = ["actions", "settings", "models", "meetings"];
    const nodes: HTMLElement[] = [];
    let flatIndex = 0;
    const ordered: PaletteItem[] = [];
    for (const group of groups) {
      const grouped = this.items.filter((item) => item.group === group);
      if (!grouped.length) continue;
      nodes.push(h("div", { class: "command-palette-group" }, l(`command_palette.group.${group}`)));
      for (const item of grouped) {
        const index = flatIndex++;
        ordered.push(item);
        nodes.push(h("button", {
          type: "button",
          role: "option",
          class: "command-palette-item",
          "aria-selected": String(index === this.selectedIndex),
          onmouseenter: () => this.selectIndex(index),
          onclick: () => this.execute(item),
        },
          icon(item.iconName),
          h("span", { class: "command-palette-copy grow" },
            h("span", { class: "command-palette-title" }, item.title),
            item.subtitle ? h("span", { class: "command-palette-subtitle" }, item.subtitle) : null),
          item.badge ? h("span", { class: "badge" }, item.badge) : null));
      }
    }
    this.items = ordered;
    list.replaceChildren(...nodes);
  }
}
