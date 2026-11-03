import { t } from "./i18n.ts";

export type WorkspaceSection = "dictation" | "meetings" | "history" | "settings" | "recording";
export interface MeetingSourceTarget {
  kind: "notes" | "summary" | "transcript";
  start_ms: number | null;
  text: string;
  meeting_uid?: string | null;
  utterance_ids?: string[];
  utterance_fingerprints?: string[];
}
export interface WorkspaceRoute {
  section: WorkspaceSection;
  tab: string | null;
  meeting_id: string | null;
  jump_ms: number | null;
  source_target?: MeetingSourceTarget;
}

/** All product sections share main; old window names remain compatible route aliases. */
export type WindowMode = "workspace" | "settings" | "history" | "dictation";
export const windowTitles: Record<WindowMode, string> = {
  workspace: "Pennel",
  get settings() { return t("window.settings"); },
  get history() { return t("window.history"); },
  get dictation() { return t("window.dictation"); },
};

export function windowForSection(_section: WorkspaceSection): WindowMode {
  return "workspace";
}

export function windowMode(_value: string | null): WindowMode {
  return "workspace";
}

export const settingTabs = ["overview", "providers", "models", "dictation", "meeting-settings", "recording", "dictionary", "styles", "appearance", "account"] as const;
const settings = new Set<string>([...settingTabs, "local"]);
const reserved = new Set<string>([...settings, "workspace", "settings", "history", "meeting", "live", "onboarding", "prompt", "record-prompt", "pill", "library", "meetings", "chat"]);

export function validMeetingId(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0 && value !== "." && value !== ".."
    && !reserved.has(value) && !/[\u0000-\u001f\u007f-\u009f/\\:]/u.test(value);
}

export function workspaceRoute(section: WorkspaceSection, tab: string | null = null, meetingId: string | null = null, jumpMs: number | null = null, sourceTarget?: MeetingSourceTarget): WorkspaceRoute {
  return { section, tab, meeting_id: meetingId, jump_ms: jumpMs, ...(sourceTarget ? { source_target: { ...sourceTarget } } : {}) };
}

export function historyEntryId(tab: string | null | undefined): string | null {
  if (!tab?.startsWith("entry:")) return null;
  const id = tab.slice(6);
  return /^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(id) ? id : null;
}

export function historyRoute(id?: string | null): WorkspaceRoute {
  const tab = id ? `entry:${id}` : null;
  return workspaceRoute("history", historyEntryId(tab) ? tab : null);
}

export function parseWorkspaceRoute(value: unknown): WorkspaceRoute | null {
  if (!value || typeof value !== "object") return null;
  const input = value as Record<string, unknown>;
  if (!["dictation", "meetings", "history", "settings", "recording"].includes(String(input.section))) return null;
  let section = input.section as WorkspaceSection;
  const tab = typeof input.tab === "string" ? input.tab : null;
  if (section === "meetings" && tab === "recording") section = "recording";
  if (section === "settings" && tab !== null && !settings.has(tab) && tab !== "meetings") return null;
  if (section === "history" && tab !== null && !historyEntryId(tab)) return null;
  if (input.meeting_id != null && !validMeetingId(input.meeting_id)) return null;
  const id = validMeetingId(input.meeting_id) ? input.meeting_id : null;
  const jump = typeof input.jump_ms === "number" && Number.isFinite(input.jump_ms) && input.jump_ms >= 0 ? input.jump_ms : null;
  let source: MeetingSourceTarget | undefined;
  if (input.source_target != null) {
    if (section !== "meetings" || !id || typeof input.source_target !== "object") return null;
    const candidate = input.source_target as Record<string, unknown>;
    if (!["notes", "summary", "transcript"].includes(String(candidate.kind)) || typeof candidate.text !== "string" || !candidate.text.trim()) return null;
    if (candidate.start_ms !== null && !(typeof candidate.start_ms === "number" && Number.isFinite(candidate.start_ms) && candidate.start_ms >= 0)) return null;
    source = { kind: candidate.kind as MeetingSourceTarget["kind"], start_ms: candidate.start_ms as number | null, text: candidate.text };
    if (candidate.meeting_uid != null) {
      if (typeof candidate.meeting_uid !== "string" || !candidate.meeting_uid.trim()) return null;
      source.meeting_uid = candidate.meeting_uid;
    }
    if (candidate.utterance_ids != null || candidate.utterance_fingerprints != null) {
      const ids = candidate.utterance_ids;
      const fingerprints = candidate.utterance_fingerprints;
      if (source.kind !== "transcript" || !source.meeting_uid || !Array.isArray(ids) || !ids.length || !ids.every(id => typeof id === "string" && id.trim())
        || new Set(ids).size !== ids.length || !Array.isArray(fingerprints) || fingerprints.length !== ids.length
        || !fingerprints.every(fingerprint => typeof fingerprint === "string" && /^[a-f0-9]{64}$/.test(fingerprint))) return null;
      source.utterance_ids = [...ids]; source.utterance_fingerprints = [...fingerprints];
    }
  }
  return workspaceRoute(section, section === "settings" && tab === "meetings" ? "meeting-settings" : tab, id, jump, source);
}

export function legacyWorkspaceRoute(kind: string, route?: string | null): WorkspaceRoute | null {
  if (kind === "settings") return workspaceRoute("settings", route === "meetings" ? "meeting-settings" : settings.has(route ?? "") ? route! : "overview");
  if (kind === "history") return historyRoute(historyEntryId(route));
  if (kind === "dictation") return workspaceRoute("dictation", route === "pad" || route === "home" ? route : null);
  if (kind === "live" || kind === "meeting") return workspaceRoute("recording", null, validMeetingId(route) ? route : null);
  if (kind === "library" || kind === "meetings") return workspaceRoute("meetings", route === "chat" ? "chat" : null, validMeetingId(route) ? route : null);
  if (kind === "workspace") {
    if (!route) return workspaceRoute("meetings");
    if (route === "history") return workspaceRoute("history");
    if (route === "meeting" || route === "live") return workspaceRoute("recording");
    if (route === "chat") return workspaceRoute("meetings", "chat");
    if (route === "meetings" || route === "library") return workspaceRoute("meetings");
    if (settings.has(route)) return workspaceRoute("settings", route);
    return validMeetingId(route) ? workspaceRoute("meetings", null, route) : null;
  }
  return null;
}

/** A delivered route only navigates the window that shows it; it never opens another window, which would answer back. */
export function deliveredRouteApplies(mode: string, route: WorkspaceRoute): boolean {
  return windowForSection(route.section) === mode;
}

export function routeKey(route: WorkspaceRoute): string {
  return JSON.stringify([route.section, route.tab, route.meeting_id, route.jump_ms, route.source_target ?? null]);
}

export function samePadRoute(current: WorkspaceRoute, next: WorkspaceRoute): boolean {
  return current.section === "dictation" && next.section === "dictation" && routeKey(current) === routeKey(next);
}

export class WorkspaceNavigation {
  current: WorkspaceRoute;
  constructor(initial: WorkspaceRoute = workspaceRoute("meetings")) { this.current = { ...initial }; this.recordingWasCurrent = initial.section === "recording"; }
  private readonly remembered = new Map<WorkspaceSection, WorkspaceRoute>();
  private settingsOrigin: WorkspaceRoute | null = null;
  private recordingWasCurrent = false;

  set(route: WorkspaceRoute): void {
    if (route.section === "recording") this.recordingWasCurrent = true;
    else if (route.section === "meetings") this.recordingWasCurrent = false;
    if (route.section === "settings" && this.current.section !== "settings") this.settingsOrigin = { ...this.current };
    this.current = { ...route };
    if (route.section !== "recording") {
      const remembered = { ...route, jump_ms: null };
      delete remembered.source_target;
      this.remembered.set(route.section, remembered);
    }
  }

  select(section: WorkspaceSection): WorkspaceRoute {
    if (section === "history") return historyRoute();
    return this.remembered.get(section) ?? (this.current.section === section ? { ...this.current } : workspaceRoute(section, section === "settings" ? "overview" : null));
  }

  settingsBack(): WorkspaceRoute {
    return this.settingsOrigin ? { ...this.settingsOrigin } : this.select("meetings");
  }

  recordingFinished(id: string): void {
    if (this.recordingWasCurrent) { this.remembered.set("meetings", workspaceRoute("meetings", null, id)); this.recordingWasCurrent = false; }
    if (this.settingsOrigin?.section === "recording") this.settingsOrigin = workspaceRoute("meetings", null, id);
  }

  meetingRenamed(previousId: string, id: string): void {
    const update = (route: WorkspaceRoute | null) => {
      if (route && (route.section === "meetings" || route.section === "recording") && route.meeting_id === previousId) route.meeting_id = id;
    };
    update(this.current);
    update(this.settingsOrigin);
    this.remembered.forEach(update);
  }
}
