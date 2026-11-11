import { bindLocalization, formatNumber, l, liveText, LiveText, localizeBackendText, type UiText } from "./i18n.ts";
import { contentHeadingTags } from "./content-headings.ts";
import { icon, type IconName } from "./icons.ts";
import { nativeMenusAvailable, popupOwnerAvailable, showNativePopup } from "./native-menu.ts";
import type { MenuPoint } from "./native-menu-state.ts";

type Child = Node | UiText | null | undefined | false;
type Attrs = Record<string, UiText | number | boolean | EventListener | null | undefined>;

function localizedAttribute(el: HTMLElement, key: string, value: UiText): void {
  const update = () => el.setAttribute(key, String(value));
  update();
  if (value instanceof LiveText) bindLocalization(el, update);
}

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: (Child | Child[])[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on") && typeof value === "function") {
      el.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (key === "value" && "value" in el) {
      (el as HTMLInputElement).value = String(value);
    } else if (key === "checked" && "checked" in el) {
      (el as HTMLInputElement).checked = Boolean(value);
    } else {
      localizedAttribute(el, key, value === true ? "" : value as UiText);
    }
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    if (child instanceof LiveText) {
      const text = document.createTextNode(String(child));
      bindLocalization(text, () => { text.data = String(child); });
      el.append(text);
    } else el.append(child);
  }
  return el;
}

/** Shared push button and text-field constructors keep ordinary controls consistent across views. */
export function controlButton(attrs: Attrs = {}, ...children: (Child | Child[])[]): HTMLButtonElement {
  const classes = String(attrs.class ?? "").split(/\s+/).filter(Boolean);
  if (!classes.includes("btn")) classes.unshift("btn");
  return h("button", { ...attrs, class: classes.join(" ") }, ...children);
}

export function textField(attrs: Attrs = {}): HTMLInputElement {
  return h("input", { type: "text", ...attrs });
}

export function section(title: UiText, subtitle: UiText | null, ...body: (Child | Child[])[]) {
  const headingId = `section-${++sectionNumber}`;
  const node = h(
    "section",
    { class: "pane-section", "aria-labelledby": headingId },
    h("h2", { id: headingId }, title),
    subtitle ? h("p", { class: "subtitle" }, subtitle) : null,
    ...body,
  );
  for (const choice of node.querySelectorAll<HTMLElement>(".choice")) {
    if (!choice.closest(".field") && !choice.hasAttribute("aria-label")) localizedAttribute(choice, "aria-label", title);
  }
  for (const control of node.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("input,select,textarea")) {
    if (!control.closest("label") && !control.labels?.length && !control.hasAttribute("aria-label") && !control.hasAttribute("aria-labelledby")) control.setAttribute("aria-labelledby", headingId);
  }
  return node;
}

let sectionNumber = 0;

export function field(label: UiText, control: HTMLElement) {
  const formControl = control.matches("input,textarea");
  if (control.classList.contains("choice") && !control.hasAttribute("aria-label")) localizedAttribute(control, "aria-label", label);
  return h(formControl ? "label" : "div", { class: "field" }, h("span", { class: "field-label" }, label), control);
}

/** A small icon-and-label chip. Group chips in `.chips`. */
export function chip(iconName: IconName, text: UiText): HTMLSpanElement {
  return h("span", { class: "chip" }, icon(iconName), text);
}

/** A shortcut shown as one keycap per key, in the order they are pressed. */
export function keycaps(keys: UiText[]): HTMLSpanElement {
  return h("span", { class: "keycaps" }, ...keys.map((key) => h("kbd", {}, key)));
}

export interface ChoiceOption { readonly value: string; label: string; disabled: boolean }
/** A choice control with the subset of the select element's interface the views use. */
export interface ChoiceElement<T extends string = string> extends HTMLElement {
  value: T;
  disabled: boolean;
  readonly options: ChoiceOption[];
}
export type ChoiceStyle = "auto" | "segmented" | "menu";

/**
 * A few short fixed options become a segmented control; longer lists keep a branded trigger and open
 * a system menu in the app. Browser fixtures use the renderer menu without touching native IPC.
 */
export function select<T extends string>(
  options: [T, UiText][],
  value: T,
  onChange: (value: T) => void,
  style: ChoiceStyle = "auto",
): ChoiceElement<T> {
  const segmented = style === "segmented" || (style === "auto" && options.length <= 4 && options.reduce((total, [, label]) => total + label.length, 0) <= 44);
  return segmented ? segmentedChoice(options, value, onChange) : menuChoice(options, value, onChange);
}

function segmentedChoice<T extends string>(options: [T, UiText][], value: T, onChange: (value: T) => void): ChoiceElement<T> {
  let current = value;
  let disabled = false;
  const root = h("div", { class: "segmented choice", role: "radiogroup" }) as unknown as ChoiceElement<T>;
  const buttons = options.map(([option, label]) => {
    const button = h("button", { type: "button", role: "radio", "data-value": option }, label);
    button.addEventListener("click", () => pick(option));
    button.addEventListener("keydown", (event) => {
      const enabled = buttons.filter((item) => !item.disabled);
      const index = enabled.indexOf(button);
      const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
      if (!step || index < 0) return;
      event.preventDefault();
      const next = enabled[(index + step + enabled.length) % enabled.length];
      next.focus();
      pick(next.dataset.value as T);
    });
    return button;
  });
  root.append(...buttons);
  const draw = () => buttons.forEach((button) => {
    const selected = button.dataset.value === current;
    button.setAttribute("aria-checked", String(selected));
    button.tabIndex = selected ? 0 : -1;
  });
  const pick = (option: T) => {
    if (disabled || option === current) return;
    current = option; draw(); onChange(option);
  };
  draw();
  const choices = options.map(([option], index): ChoiceOption => ({
    value: option,
    get label() { return String(options[index][1]); },
    set label(next: string) { options[index][1] = next; buttons[index].textContent = next; },
    get disabled() { return buttons[index].disabled; },
    set disabled(next: boolean) { buttons[index].disabled = next || disabled; },
  }));
  Object.defineProperties(root, {
    value: { get: () => current, set: (next: T) => { current = next; draw(); } },
    disabled: { get: () => disabled, set: (next: boolean) => { disabled = next; buttons.forEach((button) => { button.disabled = next; }); root.toggleAttribute("aria-disabled", next); } },
    options: { get: () => choices },
  });
  return root;
}

function menuChoice<T extends string>(options: [T, UiText][], value: T, onChange: (value: T) => void): ChoiceElement<T> {
  let current = value;
  let name = "";
  const disabledOptions = new Set<string>();
  const text = h("span", { class: "menu-button-label" });
  const root = h("button", { type: "button", class: "menu-button choice", "aria-haspopup": nativeMenusAvailable() ? "menu" : "listbox", "aria-expanded": nativeMenusAvailable() ? null : "false" }, text, chevron()) as unknown as ChoiceElement<T>;
  const setAttribute = root.setAttribute.bind(root);
  const draw = () => {
    const label = options.find(([option]) => option === current)?.[1] ?? current;
    text.textContent = String(label);
    if (name) setAttribute("aria-label", `${name}: ${label}`);
  };
  root.setAttribute = (attribute: string, next: string) => { if (attribute === "aria-label") { name = next; draw(); } else setAttribute(attribute, next); };
  root.addEventListener("click", () => showPopupMenu(root, options.map(([option, label]) => ({ value: option, label, disabled: disabledOptions.has(option), run: () => {
    if (option === current || disabledOptions.has(option) || root.disabled) return;
    current = option; draw(); onChange(current);
  } })), { current, role: "listbox" }));
  root.addEventListener("keydown", (event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); root.click(); } });
  draw();
  bindLocalization(root, draw);
  const choices = options.map(([option], index): ChoiceOption => ({
    value: option,
    get label() { return String(options[index][1]); },
    set label(next: string) { options[index][1] = next; draw(); },
    get disabled() { return disabledOptions.has(option); },
    set disabled(next: boolean) { if (next) disabledOptions.add(option); else disabledOptions.delete(option); },
  }));
  Object.defineProperties(root, {
    value: { get: () => current, set: (next: T) => { current = next; draw(); } },
    options: { get: () => choices },
  });
  return root;
}

/** A pill button that offers a list of actions in a menu, such as Delen or Provider toevoegen…. */
export function menuButton(label: UiText, items: { label: UiText; disabled?: boolean; title?: UiText | null; run: () => unknown }[], className = ""): HTMLButtonElement {
  const button = h("button", { type: "button", class: `menu-button ${className}`.trim(), "aria-haspopup": "menu", "aria-expanded": nativeMenusAvailable() ? null : "false" }, h("span", { class: "menu-button-label" }, label), chevron());
  button.addEventListener("click", () => showPopupMenu(button, items.map((item, index) => ({ value: String(index), label: item.label, disabled: !!item.disabled, title: item.title ?? undefined, run: () => { if (!item.disabled) return item.run(); } }))));
  button.addEventListener("keydown", (event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); button.click(); } });
  return button;
}

function chevron(): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 12 12"); svg.setAttribute("aria-hidden", "true"); svg.setAttribute("class", "menu-button-chevron");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M3 4.5 6 7.5 9 4.5"); path.setAttribute("fill", "none"); path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "1.5"); path.setAttribute("stroke-linecap", "round"); path.setAttribute("stroke-linejoin", "round");
  svg.append(path);
  return svg;
}

export interface PopupMenuItem {
  value: string;
  label: UiText;
  disabled?: boolean;
  title?: UiText;
  separatorBefore?: boolean;
  run: () => unknown;
}

export interface PopupMenuOptions {
  position?: MenuPoint;
  current?: string;
  role?: "listbox" | "menu";
  report?: (error: unknown) => void;
}

export interface PopupMenuHandle { close: (restoreFocus?: boolean) => void }
let openMenu: PopupMenuHandle | null = null;
globalThis.window?.addEventListener("beforeunload", () => openMenu?.close());

function popupError(error: unknown) { void Promise.reject(error); }

export function showPopupMenu(anchor: HTMLElement, items: PopupMenuItem[], options: PopupMenuOptions = {}): PopupMenuHandle {
  openMenu?.close();
  if (!nativeMenusAvailable()) return rendererPopup(anchor, items, options);
  let closed = false;
  let fallback: PopupMenuHandle | null = null;
  const close = (restoreFocus = false) => {
    if (closed) return;
    closed = true;
    observer.disconnect(); unbind(); native.close(); fallback?.close(restoreFocus);
    if (openMenu === handle) openMenu = null;
    if (restoreFocus && anchor.isConnected) anchor.focus({ preventScroll: true });
  };
  const handle = { close };
  const observer = new MutationObserver(() => { if (!popupOwnerAvailable(anchor)) close(); });
  observer.observe(document.body, { childList: true, attributes: true, attributeFilter: ["hidden", "inert", "disabled"], subtree: true });
  const unbind = bindLocalization(anchor, () => close());
  anchor.setAttribute("aria-haspopup", "menu");
  // The native accessibility tree owns expanded/dismissed state, including Escape and outside clicks.
  anchor.removeAttribute("aria-expanded");
  const native = showNativePopup(anchor, items.map((item) => ({ ...item, label: String(item.label) })), {
    current: options.current,
    position: options.position,
    selected: () => close(true),
    report: options.report ?? popupError,
  });
  openMenu = handle;
  void native.ready.catch((error) => {
    if (closed || !popupOwnerAvailable(anchor)) return;
    observer.disconnect(); unbind();
    options.report?.(error);
    fallback = rendererPopup(anchor, items, options);
  });
  return handle;
}

function rendererPopup(anchor: HTMLElement, items: PopupMenuItem[], options: PopupMenuOptions): PopupMenuHandle {
  const role = options.role ?? "menu";
  const current = options.current;
  const itemRole = role === "listbox" ? "option" : "menuitem";
  const list = h("div", { class: "context-menu choice-list", role });
  let closed = false;
  const close = (focus = true) => {
    if (closed) return;
    closed = true;
    list.remove(); anchor.setAttribute("aria-expanded", "false");
    observer.disconnect();
    document.removeEventListener("pointerdown", outside, true);
    if (openMenu === handle) openMenu = null;
    if (focus && anchor.isConnected) anchor.focus({ preventScroll: true });
  };
  const handle = { close: (focus = false) => close(focus) };
  const observer = new MutationObserver(() => { if (!anchor.isConnected || anchor.matches(":disabled") || anchor.closest("[hidden],[inert]")) close(false); });
  observer.observe(document.body, { childList: true, attributes: true, attributeFilter: ["hidden", "inert", "disabled"], subtree: true });
  const outside = (event: Event) => { if (!list.contains(event.target as Node) && event.target !== anchor && !anchor.contains(event.target as Node)) close(false); };
  const entries = items.map((item) => {
    const entry = h("button", { type: "button", role: itemRole, disabled: item.disabled, title: item.title ?? null, "aria-selected": role === "listbox" ? String(item.value === current) : null, class: item.value === current ? "chosen" : null }, item.label);
    entry.addEventListener("click", () => {
      if (item.disabled || !anchor.isConnected || anchor.matches(":disabled")) return;
      close();
      try { void Promise.resolve(item.run()).catch(options.report ?? popupError); }
      catch (error) { (options.report ?? popupError)(error); }
    });
    return entry;
  });
  entries.forEach((entry, index) => { if (items[index].separatorBefore && index > 0) list.append(h("hr", { role: "separator" })); list.append(entry); });
  let typed = "";
  let typedAt = 0;
  list.addEventListener("keydown", (event) => {
    const enabled = entries.filter((entry) => !entry.disabled);
    const index = enabled.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Escape") { event.preventDefault(); close(); return; }
    if (event.key === "Tab") { close(); return; }
    const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (event.key === "Home" || event.key === "End") { event.preventDefault(); (event.key === "Home" ? enabled[0] : enabled.at(-1))?.focus(); }
    if (step) { event.preventDefault(); enabled[(index + step + enabled.length) % enabled.length]?.focus(); }
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey && event.key !== " ") {
      const now = performance.now();
      typed = now - typedAt > 700 ? event.key : typed + event.key;
      typedAt = now;
      const match = [...enabled.slice(index + 1), ...enabled.slice(0, index + 1)].find((entry) => entry.textContent?.trim().toLocaleLowerCase().startsWith(typed.toLocaleLowerCase()));
      if (match) { event.preventDefault(); match.focus(); }
    }
  });
  // Controls inside a modal must keep their menu in its active top-layer subtree.
  (anchor.closest("dialog[open]") ?? document.body).append(list);
  const bounds = anchor.getBoundingClientRect();
  list.style.minWidth = `${Math.min(Math.max(bounds.width, 160), innerWidth - 16)}px`;
  const size = list.getBoundingClientRect();
  list.style.left = `${Math.max(8, Math.min(options.position?.x ?? bounds.left, innerWidth - size.width - 8))}px`;
  const below = options.position?.y ?? bounds.bottom + 4;
  list.style.top = `${below + size.height > innerHeight - 8 ? Math.max(8, bounds.top - size.height - 4) : below}px`;
  anchor.setAttribute("aria-haspopup", role);
  anchor.setAttribute("aria-expanded", "true");
  document.addEventListener("pointerdown", outside, true);
  openMenu = handle;
  (entries.find((entry) => entry.classList.contains("chosen") && !entry.disabled) ?? entries.find((entry) => !entry.disabled))?.focus();
  return handle;
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${formatNumber(bytes / 1e9, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} GB`;
  return `${formatNumber(Math.round(bytes / 1e6))} MB`;
}

export function formatClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const remainder = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${remainder}` : `${String(minutes).padStart(2, "0")}:${remainder}`;
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return localizeBackendText(e);
  if (e instanceof Error) return localizeBackendText(e.message);
  if (typeof e === "object" && e !== null && typeof (e as { message?: unknown }).message === "string") {
    return localizeBackendText((e as { message: string }).message);
  }
  return JSON.stringify(e);
}

export function errorCode(e: unknown): string | null {
  const code = typeof e === "object" && e !== null ? (e as { code?: unknown }).code : null;
  return typeof code === "string" ? code : null;
}

export function isCancellation(error: unknown): boolean {
  return errorCode(error) === "cancelled" || /^(cancelled|canceled|geannuleerd)$/i.test(errorText(error).trim());
}

export function toggle(
  label: UiText,
  checked: boolean,
  change: (value: boolean) => void,
  capability?: { status: string; reason: string | null },
) {
  const disabled = capability?.status === "unsupported" || capability?.status === "unavailable";
  const input = h("input", { type: "checkbox", role: "switch", checked, disabled });
  input.addEventListener("change", () => change(input.checked));
  return h("div", { class: "toggle-field" },
    h("label", { class: "checkbox" }, input, label),
    capability?.reason ? h("p", { class: "help" }, liveText(() => localizeBackendText(capability.reason ?? ""))) : null);
}

function inline(text: string): DocumentFragment {
  const result = document.createDocumentFragment();
  const pattern = /(\*\*[^*]+\*\*|`[^`]+`)/g;
  let offset = 0;
  for (const match of text.matchAll(pattern)) {
    result.append(document.createTextNode(text.slice(offset, match.index)));
    const bold = match[0].startsWith("**");
    result.append(h(bold ? "strong" : "code", {}, match[0].slice(bold ? 2 : 1, bold ? -2 : -1)));
    offset = (match.index ?? 0) + match[0].length;
  }
  result.append(document.createTextNode(text.slice(offset)));
  return result;
}

export function markdown(text: string): HTMLElement {
  const block = h("div", { class: "markdown" });
  const lines = text.split("\n");
  const headings = contentHeadingTags(lines.flatMap((line) => {
    const marker = line.trim().match(/^(#{1,6})\s+/);
    return marker ? [marker[1].length] : [];
  }));
  let headingIndex = 0;
  for (const line of lines) {
    const clean = line.trim();
    if (!clean) { block.append(h("div", { class: "paragraph-gap" })); continue; }
    const heading = clean.match(/^(#{1,6})\s+(.+)/);
    const bullet = clean.match(/^[-*•]\s+(.+)/);
    const task = bullet?.[1].match(/^\[([ xX])\]\s+(.+)/);
    const node = heading ? h(headings[headingIndex++], { class: "markdown-heading" }) : h("p", { class: bullet ? "bullet" : null });
    if (task) node.append(h("span", { class: "markdown-check", role: "img", "aria-label": l(task[1].trim() ? "dom.task_done" : "dom.task_open") }, task[1].trim() ? "☑ " : "□ "));
    else if (bullet) node.append(h("span", { class: "bullet-mark" }, "• "));
    node.append(inline(heading?.[2] ?? task?.[2] ?? bullet?.[1] ?? clean));
    block.append(node);
  }
  return block;
}

export function snippet(text: string): HTMLElement {
  const result = h("span", { class: "search-snippet" });
  for (const part of text.split(/(«[^»]*»)/g)) {
    result.append(part.startsWith("«") && part.endsWith("»")
      ? h("mark", {}, part.slice(1, -1)) : document.createTextNode(part));
  }
  return result;
}

let dialogNumber = 0;

export function namedDialog(title: UiText, detail: UiText = "") {
  const id = `dialog-${++dialogNumber}`;
  const invoker = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const dialog = h("dialog", { class: "editor-dialog", "aria-modal": "true", "aria-labelledby": `${id}-title`, "aria-describedby": detail ? `${id}-detail` : null });
  const heading = h("h2", { id: `${id}-title` }, title);
  const description = detail ? h("p", { id: `${id}-detail`, class: "muted" }, detail) : null;
  const close = () => {
    dialog.close(); dialog.remove();
    if (invoker?.isConnected) invoker.focus({ preventScroll: true });
  };
  return { dialog, heading, description, close };
}

export function confirmDialog(title: UiText, detail: UiText, action: UiText = l("common.delete")): Promise<boolean> {
  return new Promise((resolve) => {
    const { dialog, heading, description, close } = namedDialog(title, detail);
    const finish = (value: boolean) => { close(); resolve(value); };
    const cancel = controlButton({ class: "btn", onclick: () => finish(false) }, l("common.cancel"));
    dialog.append(heading, ...(description ? [description] : []),
      h("div", { class: "dialog-actions row" }, h("span", { class: "grow" }),
        cancel,
        controlButton({ class: "btn danger", onclick: () => finish(true) }, action)));
    dialog.addEventListener("cancel", (event) => { event.preventDefault(); finish(false); });
    document.body.append(dialog);
    dialog.showModal();
    cancel.focus();
  });
}

export function askText(title: UiText, value: string, detail: UiText = ""): Promise<string | null> {
  return new Promise((resolve) => {
    const { dialog, heading, description, close } = namedDialog(title, detail);
    const input = textField({ type: "text", value, "aria-label": title });
    const finish = (result: string | null) => { close(); resolve(result); };
    const form = h("form", {}, heading, description, input,
      h("div", { class: "dialog-actions row" }, h("span", { class: "grow" }),
        controlButton({ class: "btn", type: "button", onclick: () => finish(null) }, l("common.cancel")),
        controlButton({ class: "btn primary", type: "submit" }, l("common.save"))));
    form.addEventListener("submit", (event) => { event.preventDefault(); finish(input.value.trim()); });
    dialog.addEventListener("cancel", (event) => { event.preventDefault(); finish(null); });
    dialog.append(form);
    document.body.append(dialog);
    dialog.showModal();
    input.focus();
    input.select();
  });
}

export interface NameChoice { name: string; hint: UiText }

/** A name dialog with one opt-in switch below the field, such as Onthoud deze stem, and optional names to pick from. */
export function askTextWithSwitch(title: UiText, value: string, detail: UiText, label: UiText, checked = false,
  choices: NameChoice[] = [], switchDisabled = false): Promise<{ text: string; on: boolean } | null> {
  return new Promise((resolve) => {
    const { dialog, heading, description, close } = namedDialog(title, detail);
    const input = textField({ type: "text", value, "aria-label": title });
    const option = h("input", { type: "checkbox", role: "switch", checked: checked && !switchDisabled, disabled: switchDisabled });
    const finish = (result: { text: string; on: boolean } | null) => { close(); resolve(result); };
    const picks = choices.length ? [h("div", { class: "name-choices" }, ...choices.map(choice =>
      controlButton({ class: "btn compact", type: "button", title: choice.hint,
        onclick: () => { input.value = choice.name; input.focus(); } }, choice.name)))] : [];
    const form = h("form", {}, heading, description, input, ...picks, h("label", { class: "checkbox" }, option, label),
      h("div", { class: "dialog-actions row" }, h("span", { class: "grow" }),
        controlButton({ class: "btn", type: "button", onclick: () => finish(null) }, l("common.cancel")),
        controlButton({ class: "btn primary", type: "submit" }, l("common.save"))));
    form.addEventListener("submit", (event) => { event.preventDefault(); finish({ text: input.value.trim(), on: option.checked }); });
    dialog.addEventListener("cancel", (event) => { event.preventDefault(); finish(null); });
    dialog.append(form);
    document.body.append(dialog);
    dialog.showModal();
    input.focus();
    input.select();
  });
}
