import "./fonts.css";
import "./pill.css";
import { api, listen, listenToWindow, type AppInfo, type PillPreview, type PillState } from "./api";
import { icon } from "./icons";
import { PillPreviewState } from "./pill-preview-state";
import { PILL_METRICS } from "./pill-metrics.generated";
import { hotkeyKeyLabel } from "./presentation";
import { bindLocalization, getInterfaceLanguage, localizeBackendText, setInterfaceLanguage, t } from "./i18n.ts";

const pill = document.getElementById("pill")!;
const status = document.getElementById("status")!;
const indicator = document.getElementById("indicator")!;
const label = document.getElementById("label")!;
const detail = document.getElementById("detail")!;
const hint = document.getElementById("hint")!;
const preview = document.getElementById("preview")!;
const previewText = document.getElementById("preview-text")!;
const previewState = new PillPreviewState(PILL_METRICS.previewLimit);
let visible = false;
let localeReady = false;
let pendingState: PillState | null = null;
let shown: PillState["state"] = "hidden";
let doneKey = "";
const weights = [0.5, 0.8, 1, 0.75, 0.45];
const bars = weights.map(() => document.createElement("i"));
const listening: PillState["state"][] = ["listening", "hands_free", "command", "no_sound"];

function keycap(text: string): HTMLElement {
  const key = document.createElement("kbd");
  key.textContent = text;
  return key;
}

function content(state: PillState): { text: string; detail?: string; hint?: (string | Node)[] } {
  switch (state.state) {
    case "listening": return { text: t("dictation.pill.listening") };
    case "hands_free": return { text: t("dictation.pill.hands_free"), hint: doneKey ? [keycap(doneKey), t("dictation.pill.done")] : [] };
    case "command": return { text: t("dictation.pill.command"), hint: [t("dictation.pill.command_hint")] };
    case "no_sound": return { text: t("dictation.pill.no_sound"), hint: [t("dictation.pill.no_sound_hint")] };
    case "working": return { text: t("dictation.pill.working") };
    case "inserted": return { text: state.app ? t("dictation.pill.inserted_in", { app: state.app }) : t("dictation.pill.inserted") };
    case "copied": return { text: t("dictation.pill.copied"), hint: [keycap(state.paste_key), t("dictation.pill.to_paste")] };
    case "problem": return { text: localizeBackendText(state.title), detail: state.detail ? localizeBackendText(state.detail) : undefined };
    case "hidden": return { text: "" };
  }
}

function setLevel(level: number) {
  const gain = shown === "no_sound" ? 0 : Math.min(1, level * 12);
  bars.forEach((bar, i) => (bar.style.transform = `scaleY(${(4 + 12 * weights[i] * gain) / 16})`));
}

function paint(next: PillState) {
  const parts = content(next);
  shown = next.state;
  pill.dataset.state = next.state;
  label.textContent = parts.text;
  detail.textContent = parts.detail ?? "";
  detail.hidden = !parts.detail;
  hint.replaceChildren(...parts.hint ?? []);
  hint.hidden = !parts.hint?.length;
  indicator.className = next.state;
  if (listening.includes(next.state)) {
    indicator.replaceChildren(...bars);
    setLevel(0);
  } else {
    const symbol = next.state === "inserted" ? "check" : next.state === "copied" ? "copy" : next.state === "problem" ? "warning" : null;
    indicator.replaceChildren(...symbol ? [icon(symbol)] : []);
  }
}

/** Restarts a CSS animation that is tied to a class. */
function replay(element: HTMLElement, name: string) {
  element.classList.remove(name);
  void element.offsetWidth;
  element.classList.add(name);
}

function render(next: PillState) {
  pendingState = next;
  if (!localeReady) return;
  const wasVisible = visible;
  visible = next.state !== "hidden";
  if (!visible) {
    previewState.receive(null);
    shown = "hidden";
    if (wasVisible) {
      pill.classList.remove("entering");
      pill.classList.add("leaving");
    } else {
      pill.hidden = true;
    }
    renderPreview();
    return;
  }
  const changed = shown !== next.state;
  pill.classList.remove("leaving");
  pill.hidden = false;
  paint(next);
  if (!wasVisible) replay(pill, "entering");
  else if (changed) replay(status, "switching");
  renderPreview();
}

function renderPreview() {
  const text = visible ? previewState.snapshot?.text ?? "" : "";
  previewText.textContent = text;
  preview.hidden = !text;
  pill.classList.toggle("has-preview", !!text);
}

function applyInfo(info: AppInfo) {
  setInterfaceLanguage(info.interface_language ?? info.settings.ui_language ?? "en");
  doneKey = hotkeyKeyLabel(info.settings.hotkey, info.platform) ?? "";
  if (visible && pendingState) paint(pendingState);
}

async function boot() {
  pill.addEventListener("animationend", (event) => {
    if (event.target === pill && event.animationName === "pill-leave" && !visible) {
      pill.classList.remove("leaving");
      pill.hidden = true;
    }
  });
  bindLocalization(label, () => { if (visible && pendingState) paint(pendingState); });
  const locale = api.info().then(applyInfo).catch(() => { setInterfaceLanguage("en"); });
  let eventReceived = false;
  let previewReceived = false;
  let pendingPreview: PillPreview | null | undefined;
  await listenToWindow<PillState>("pill-state", (event) => { eventReceived = true; render(event.payload); });
  await listenToWindow<PillPreview | null>("pill-preview", (event) => {
    pendingPreview = event.payload;
    if (previewState.receive(event.payload)) previewReceived = true;
    renderPreview();
  });
  await listenToWindow<number>("pill-level", (event) => setLevel(event.payload));
  await listen("settings-changed", () => { void api.info().then(applyInfo).catch(() => {}); });
  await locale;
  document.documentElement.lang = getInterfaceLanguage();
  localeReady = true;
  const snapshot = await api.dictationState();
  if (!eventReceived) render(snapshot);
  else if (pendingState) render(pendingState);
  if (!previewReceived && visible) {
    previewState.restore(snapshot.preview);
    if (pendingPreview !== undefined) previewState.receive(pendingPreview);
    renderPreview();
  }
}

void boot();
