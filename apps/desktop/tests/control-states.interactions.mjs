const assert = (value, message) => { if (!value) throw new Error(message); };
const colorOf = value => { const probe = document.createElement("span"); probe.style.color = value; document.body.append(probe); const color = getComputedStyle(probe).color; probe.remove(); return color; };
const ringOf = element => { const style = getComputedStyle(element); return { visible: element.matches(":focus-visible") && style.outlineStyle !== "none" && parseFloat(style.outlineWidth) >= 2, color: style.outlineColor }; };
const background = selector => getComputedStyle(document.querySelector(selector)).backgroundColor;
const nonCurrentRow = ".library-shortcuts > button:not([aria-current])";
const primary = ".dictation-home .btn.primary";

/** Runs after a real Tab key press, so script focus counts as keyboard focus. */
export function checkFocusRings() {
  const ring = colorOf("var(--focus-ring)");
  for (const selector of [".library-search input", ".dictation-home .btn.primary", nonCurrentRow]) {
    const element = document.querySelector(selector);
    if (!element) continue;
    element.focus();
    const state = ringOf(element);
    assert(state.visible, `${selector} shows no focus-visible ring on keyboard focus.`);
    assert(state.color === ring, `${selector} ring is ${state.color}, expected the focus-ring role ${ring}.`);
  }
  assert(document.querySelector(".library-search input") && document.querySelector(primary), "The focus check needs the search field and a primary button.");
}

/** Records the resting backgrounds that the hover and pressed checks compare against. */
export function rememberResting() {
  window.__controlResting = { row: background(nonCurrentRow), primaryOpacity: getComputedStyle(document.querySelector(primary)).opacity };
}

export function checkHover() {
  const hover = background(nonCurrentRow);
  assert(hover !== window.__controlResting.row, "A sidebar row does not change on hover.");
  window.__controlHover = hover;
}

export function checkPressed() {
  const pressed = background(nonCurrentRow);
  assert(pressed !== window.__controlHover, "A sidebar row looks the same pressed as hovered.");
}

export function checkPrimaryPressed() {
  const button = document.querySelector(primary);
  const style = getComputedStyle(button);
  assert(style.backgroundColor === colorOf("var(--button)"), "A pressed primary button changed its fill, which would drop its label contrast.");
  assert(parseFloat(style.opacity) < parseFloat(window.__controlResting.primaryOpacity), "A pressed primary button gives no pressed feedback.");
}

const movement = /^(all|transform|translate|scale|rotate|width|height|top|left|right|bottom|inset|margin.*|padding.*|max-height|min-height|flex.*)$/;
const fade = /^(opacity|color|background-color|background|border-color|box-shadow)$/;
function transitions(pseudo) {
  const result = [];
  for (const element of document.querySelectorAll("*")) {
    const style = getComputedStyle(element, pseudo || null);
    const durations = style.transitionDuration.split(",").map(parseFloat);
    const properties = style.transitionProperty.split(",").map(item => item.trim());
    properties.forEach((property, index) => { if (durations[index % durations.length] > 0) result.push({ property, selector: element.className || element.tagName, pseudo }); });
    if (style.animationName !== "none" && parseFloat(style.animationDuration) > 0) result.push({ property: `animation ${style.animationName}`, selector: element.className || element.tagName, pseudo });
  }
  return result;
}

/** With the system setting emulated, `emulated` is true; otherwise the app's own switch is set here. */
export function checkReducedMotion(emulated) {
  const root = document.documentElement;
  if (!emulated) {
    const before = [...transitions(), ...transitions("::after")];
    assert(before.some(item => /transform/.test(item.property)), "The check is vacuous: no transform transition exists before reduced motion.");
    root.dataset.reduceMotion = "true";
  }
  try {
    const active = [...transitions(), ...transitions("::before"), ...transitions("::after")];
    const moving = active.filter(item => movement.test(item.property));
    assert(!moving.length, `Reduced motion leaves movement: ${JSON.stringify(moving.slice(0, 5))}`);
    assert(active.some(item => fade.test(item.property)), "Reduced motion removed the fade feedback too.");
  } finally { if (!emulated) delete root.dataset.reduceMotion; }
}

const danger = "#control-danger-probe";
const toggle = "#control-toggle-probe";
const addProbe = element => { document.querySelector(".settings-pane").prepend(element); return element; };
/** The saved switches are mostly on or disabled in the fixtures, and the destructive buttons sit in closed disclosures, so the checks add an off switch and a danger button in view. */
export function markToggle() {
  addProbe(Object.assign(document.createElement("input"), { id: "control-toggle-probe", type: "checkbox" }));
  window.__controlResting = { toggle: background(toggle) };
}
export function rememberFields() {
  addProbe(Object.assign(document.createElement("button"), { id: "control-danger-probe", className: "btn danger", textContent: "Remove" }));
  window.__controlResting = { danger: background(danger) };
}
export function checkDangerHover() {
  const hover = background(danger);
  assert(hover !== window.__controlResting.danger, "A danger button does not change on hover.");
  assert(hover !== colorOf("var(--hover)"), "A danger button lost its red tint on hover.");
}
export function checkToggleHover() {
  assert(background(toggle) !== window.__controlResting.toggle, "An off switch does not change on hover.");
}
