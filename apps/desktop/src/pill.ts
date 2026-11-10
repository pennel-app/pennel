import "./fonts.css";
import "./pill.css";
import { api, listen, listenToWindow, type AppInfo, type PillPreview, type PillState } from "./api";
import { icon } from "./icons";
import { PillModelState, type PillModelPresentation } from "./pill-model-state";
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
const modelChip = document.getElementById("model-chip")!;
const modelState = new PillModelState();
let modelTimer: ReturnType<typeof setTimeout> | null = null;
let providers: AppInfo["settings"]["providers"] = [];
let indicatorKey = "";
const previewText = document.getElementById("preview-text")!;
const previewState = new PillPreviewState(PILL_METRICS.previewLimit);
let visible = false;
let localeReady = false;
let pendingState: PillState | null = null;
let shown: PillState["state"] = "hidden";
let doneKey = "";
let messageKey = "";
let hintKey = "";
let outgoing: HTMLElement | null = null;
const weights = [0.5, 0.8, 1, 0.75, 0.45];
const bars = weights.map(() => document.createElement("i"));
const listening: PillState["state"][] = ["listening", "hands_free", "command", "no_sound"];

function keycap(text: string): HTMLElement {
  const key = document.createElement("kbd");
  key.textContent = text;
  return key;
}

function content(state: PillState): { text: string; detail?: string; hint?: (string | Node)[] } {
  const confirmation = modelState.confirmation(performance.now());
  if (confirmation && listening.includes(state.state)) return { text: t("dictation.pill.model_switched", { model: confirmation }) };
  switch (state.state) {
    case "listening": return { text: t("dictation.pill.listening") };
    case "hands_free": return { text: t("dictation.pill.hands_free"), hint: [
      ...doneKey ? [keycap(doneKey), t("dictation.pill.done")] : [],
      ...state.cancel_shortcut?.text ? [...doneKey ? ["·"] : [], keycap(state.cancel_shortcut.text), t("dictation.pill.cancel")] : [],
    ] };
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

function clearOutgoing() {
  outgoing?.remove();
  outgoing = null;
  status.classList.remove("switching");
}

function crossfade() {
  clearOutgoing();
  const layer = status.cloneNode(true) as HTMLElement;
  for (const node of [layer, ...layer.querySelectorAll("[id], [role], [aria-live], [aria-atomic]")]) {
    for (const attribute of ["id", "role", "aria-live", "aria-atomic"]) node.removeAttribute(attribute);
  }
  layer.setAttribute("aria-hidden", "true");
  layer.classList.add("pill-status-outgoing");
  layer.addEventListener("animationend", (event) => {
    if (event.target !== layer) return;
    layer.remove();
    if (outgoing === layer) outgoing = null;
  });
  status.after(layer);
  outgoing = layer;
}

function paint(next: PillState, animate = false) {
  const parts = content(next);
  const nextHintKey = JSON.stringify((parts.hint ?? []).map(part => typeof part === "string" ? part : part.textContent));
  const nextMessageKey = JSON.stringify([parts.text, parts.detail ?? "", nextHintKey]);
  const changed = shown !== next.state || messageKey !== nextMessageKey;
  if (!changed) return;
  if (animate) crossfade();
  else clearOutgoing();
  const confirmation = !!modelState.confirmation(performance.now()) && listening.includes(next.state);
  const nextIndicator = `${next.state}:${confirmation}`;
  const stateChanged = indicatorKey !== nextIndicator;
  indicatorKey = nextIndicator;
  shown = next.state;
  pill.dataset.state = next.state;
  status.dataset.state = next.state;
  if (label.textContent !== parts.text) label.textContent = parts.text;
  if (detail.textContent !== (parts.detail ?? "")) detail.textContent = parts.detail ?? "";
  detail.hidden = !parts.detail;
  if (hintKey !== nextHintKey) hint.replaceChildren(...parts.hint ?? []);
  hint.hidden = !parts.hint?.length;
  if (stateChanged) {
    indicator.className = `pill-indicator ${confirmation ? "inserted" : next.state}`;
    if (listening.includes(next.state) && !confirmation) {
      indicator.replaceChildren(...bars);
      setLevel(0);
    } else {
      const symbol = confirmation || next.state === "inserted" ? "check" : next.state === "copied" ? "copy" : next.state === "problem" ? "warning" : null;
      indicator.replaceChildren(...symbol ? [icon(symbol)] : []);
    }
  }
  messageKey = nextMessageKey;
  hintKey = nextHintKey;
  if (animate) replay(status, "switching");
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
    clearModelTimer(); modelState.clear();
    clearOutgoing();
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
  if (!listening.includes(next.state)) { clearModelTimer(); modelState.clearConfirmation(); }
  if (next.speech_model) {
    modelState.begin(next.speech_model.session_id);
    modelState.receive(next.speech_model, performance.now());
  }
  pill.classList.remove("leaving");
  pill.hidden = false;
  paint(next, wasVisible);
  if (!wasVisible) replay(pill, "entering");
  renderPreview(); renderModel();
}

function clearModelTimer() { if (modelTimer !== null) clearTimeout(modelTimer); modelTimer = null; }

function renderModel() {
  const model = modelState.chip;
  modelChip.hidden = !visible || !model || !listening.includes(shown) || !!modelState.confirmation(performance.now());
  if (model) {
    const kind = providers.find(provider => provider.id === model.choice.provider_id)?.kind;
    const name = document.createElement("span"); name.textContent = model.label;
    modelChip.replaceChildren(icon(kind === "local" || kind === "apple_speech" ? "laptop" : "globe"), name);
  } else modelChip.replaceChildren();
  const nextKey = modelState.nextKey;
  modelChip.title = model ? `${model.label}${nextKey ? ` · ${t("dictation.pill.switch_key", { shortcut: nextKey })}` : ""}` : "";
}

function receiveModel(next: PillModelPresentation | null) {
  if (next === null) { modelState.clear(); if (previewState.snapshot) modelState.begin(previewState.snapshot.session_id); }
  else if (!modelState.receive(next, performance.now())) return;
  clearModelTimer();
  const remaining = modelState.confirmationRemaining(performance.now());
  if (remaining > 0) modelTimer = setTimeout(() => {
    modelTimer = null;
    if (visible && pendingState) { modelState.clearConfirmation(); paint(pendingState, true); renderModel(); }
  }, remaining);
  if (visible && pendingState) { paint(pendingState, true); renderModel(); }
}

function renderPreview() {
  const text = visible ? previewState.snapshot?.text ?? "" : "";
  previewText.textContent = text;
  preview.hidden = !text;
  pill.classList.toggle("has-preview", !!text);
}

function applyInfo(info: AppInfo) {
  providers = info.settings.providers;
  doneKey = info.shortcuts?.find(row => row.id === "dictate")?.label.text ?? hotkeyKeyLabel(info.settings.hotkey, info.platform) ?? "";
  setInterfaceLanguage(info.interface_language ?? info.settings.ui_language ?? "en");
  if (visible && pendingState) render(pendingState);
}

async function boot() {
  pill.addEventListener("animationend", (event) => {
    if (event.target === pill && event.animationName === "pill-leave" && !visible) {
      pill.classList.remove("leaving");
      pill.hidden = true;
    }
  });
  bindLocalization(status, () => { if (visible && pendingState) render(pendingState); });
  const locale = api.info().then(applyInfo).catch(() => { setInterfaceLanguage("en"); });
  let eventReceived = false;
  let previewReceived = false;
  let pendingPreview: PillPreview | null | undefined;
  await listenToWindow<PillState>("pill-state", (event) => { eventReceived = true; render(event.payload); });
  await listenToWindow<PillPreview | null>("pill-preview", (event) => {
    pendingPreview = event.payload;
    if (previewState.receive(event.payload)) {
      previewReceived = true;
      if (event.payload) modelState.begin(event.payload.session_id);
    }
    renderPreview();
  });
  await listenToWindow<PillModelPresentation | null>("pill-model", event => receiveModel(event.payload));
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
