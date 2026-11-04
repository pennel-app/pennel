export type LocalNavigation =
  | { kind: "settings"; tab: string; param: string | undefined }
  | { kind: "library"; param: string | undefined }
  | { kind: "live" };

const settingsTabs = new Set(["account", "providers", "models", "dictation", "meeting-settings", "recording", "dictionary", "styles", "appearance", "advanced", "overview"]);
const windowRoutes = new Set([...settingsTabs, "settings", "local", "meeting", "live", "history", "onboarding", "prompt", "record-prompt", "pill"]);

export function localNavigation(window: string, route: unknown): LocalNavigation | null {
  if (typeof route !== "string") return null;
  if (window === "settings") {
    if (route === "meetings") return { kind: "settings", tab: "meeting-settings", param: undefined };
    if (route === "local") return { kind: "settings", tab: "providers", param: "__local__" };
    return settingsTabs.has(route) ? { kind: "settings", tab: route, param: undefined } : null;
  }
  if (window === "live") return route === "meeting" || route === "live" ? { kind: "live" } : null;
  if (window !== "library") return null;
  if (route === "meetings" || route === "library") return { kind: "library", param: undefined };
  if (route === "chat") return { kind: "library", param: "__chat__" };
  if (windowRoutes.has(route) || !route.trim() || route === "." || route === "..") return null;
  for (const character of route) {
    const code = character.codePointAt(0)!;
    if (code < 32 || (code >= 127 && code <= 159) || character === "/" || character === "\\" || character === ":") return null;
  }
  return { kind: "library", param: route };
}
