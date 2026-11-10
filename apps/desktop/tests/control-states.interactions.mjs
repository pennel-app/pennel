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
const fade = /^(opacity|color|background-color|background|border-color|outline-color|box-shadow|text-decoration-color|fill|stroke)$/;
function transitions(pseudo) {
  const result = [];
  for (const element of document.querySelectorAll("*")) {
    const style = getComputedStyle(element, pseudo || null);
    const durations = style.transitionDuration.split(",").map(parseFloat);
    const properties = style.transitionProperty.split(",").map(item => item.trim());
    properties.forEach((property, index) => { if (durations[index % durations.length] > 0) result.push({ property, selector: element.className || element.tagName, pseudo }); });
    const animationDurations = style.animationDuration.split(",").map(parseFloat);
    style.animationName.split(",").map(name => name.trim()).forEach((name, index) => {
      if (name !== "none" && animationDurations[index % animationDurations.length] > 0) result.push({ property: `animation ${name}`, selector: element.className || element.tagName, pseudo });
    });
  }
  return result;
}

/** With the system setting emulated, `emulated` is true; otherwise the native-environment dataset route is applied here. */
export function checkReducedMotion(emulated) {
  const root = document.documentElement;
  const original = root.getAttribute("data-reduce-motion");
  if (emulated) assert(matchMedia("(prefers-reduced-motion: reduce)").matches, "The system reduced-motion setting was not emulated.");
  if (!emulated) {
    const before = [...transitions(), ...transitions("::after")];
    assert(before.some(item => /transform/.test(item.property)), "The check is vacuous: no transform transition exists before reduced motion.");
    root.dataset.reduceMotion = "true";
  }
  try {
    const active = [...transitions(), ...transitions("::before"), ...transitions("::after")];
    const moving = active.filter(item => !fade.test(item.property));
    assert(!moving.length, `Reduced motion leaves movement: ${JSON.stringify(moving.slice(0, 5))}`);
    assert(active.some(item => fade.test(item.property)), "Reduced motion removed the fade feedback too.");
    checkSwitchFade(emulated);
  } finally { if (!emulated) { if (original === null) root.removeAttribute("data-reduce-motion"); else root.setAttribute("data-reduce-motion", original); } }
}

function checkSwitchFade(emulated) {
  const checkbox = document.querySelector('.settings-pane input[type="checkbox"]:not(:disabled)');
  assert(checkbox, "The real settings switch is missing from the motion fixture.");
  const original = checkbox.checked;
  const finish = () => checkbox.getAnimations({ subtree: true }).forEach(animation => animation.finish());
  try {
    checkbox.checked = false;
    getComputedStyle(checkbox).backgroundColor;
    finish();
    const before = getComputedStyle(checkbox).backgroundColor;
    const beforeThumb = getComputedStyle(checkbox, "::after").transform;
    checkbox.checked = true;
    getComputedStyle(checkbox).backgroundColor;
    const animations = checkbox.getAnimations({ subtree: true });
    const colour = animations.find(animation => animation.transitionProperty === "background-color");
    const duration = colour?.effect.getComputedTiming().duration;
    assert(colour && duration > 0 && duration <= 200, "The real settings switch has no short colour fade.");
    assert(!animations.some(animation => movement.test(animation.transitionProperty || "")), "The real settings switch still animates movement.");
    const thumb = getComputedStyle(checkbox, "::after").transform;
    assert(thumb !== beforeThumb, "The real settings switch did not change position.");
    colour.pause();
    colour.currentTime = duration / 2;
    const during = getComputedStyle(checkbox).backgroundColor;
    colour.finish();
    const after = getComputedStyle(checkbox).backgroundColor;
    assert(during !== before && during !== after, "The real settings switch has no intermediate colour during its short colour fade.");
    assert(getComputedStyle(checkbox, "::after").transform === thumb, "The real settings switch thumb moved during the colour fade.");
    acceptance("reduced_motion", { route: emulated ? "system" : "app", control: "real settings checkbox", injected: false, duration_ms: duration, before, during, after, thumb_moved_immediately: true, movement_animations: 0 });
  } finally { checkbox.checked = original; getComputedStyle(checkbox).backgroundColor; finish(); }
}

const danger = "#control-danger-probe";
const toggle = "#control-toggle-probe";
const addProbe = element => { document.querySelector(".settings-pane").prepend(element); return element; };
/** These visibly labelled synthetic probes supplement the real controls covered below. */
export function markToggle() {
  const label = Object.assign(document.createElement("label"), { className: "row", textContent: "Synthetic off-switch probe " });
  label.append(Object.assign(document.createElement("input"), { id: "control-toggle-probe", type: "checkbox" }));
  addProbe(label);
  window.__controlResting = { toggle: background(toggle) };
}
export function rememberFields() {
  addProbe(Object.assign(document.createElement("button"), { id: "control-danger-probe", className: "btn danger", textContent: "Synthetic danger probe" }));
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

const appearanceOf = element => {
  const style = getComputedStyle(element);
  return { background: style.backgroundColor, color: style.color, opacity: style.opacity, decoration: style.textDecorationLine };
};
const selectionOf = element => Object.fromEntries(["aria-checked", "aria-pressed", "aria-selected", "aria-current"].map(name => [name, element.getAttribute(name)]));
const acceptance = (name, value) => { (window.__PENNEL_CONTROL_ACCEPTANCE__ ??= {})[name] = value; };
function colourChannels(colour) {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const context = canvas.getContext("2d");
  context.fillStyle = colour;
  context.fillRect(0, 0, 1, 1);
  return [...context.getImageData(0, 0, 1, 1).data].slice(0, 3);
}
function textContrast(text, fill) {
  const luminance = colour => {
    const channels = colourChannels(colour).map(value => {
      const channel = value / 255;
      return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
    });
    return channels[0] * .2126 + channels[1] * .7152 + channels[2] * .0722;
  };
  const a = luminance(text), b = luminance(fill);
  return (Math.max(a, b) + .05) / (Math.min(a, b) + .05);
}
function checkProviderTextContrast(state, interaction) {
  const secondary = state.element.querySelector(".provider-kind");
  if (!secondary) return;
  const ratio = textContrast(getComputedStyle(secondary).color, getComputedStyle(state.element).backgroundColor);
  assert(ratio >= 4.5, `Provider secondary text contrast is ${ratio.toFixed(2)}:1 while ${interaction}; expected at least 4.5:1.`);
  acceptance("provider_text_contrast", { ...window.__PENNEL_CONTROL_ACCEPTANCE__?.provider_text_contrast, [interaction]: ratio });
}

/** Measures the real provider removal control rather than the synthetic danger-style probe. */
export function checkDestructiveContrast(interaction) {
  const state = control();
  assert(state.element.matches(".providers > :not(.list) > .row > button.btn.danger"), "The destructive contrast check needs the real provider removal button.");
  const style = getComputedStyle(state.element);
  const ratio = textContrast(style.color, style.backgroundColor);
  assert(style.color === colorOf("var(--danger)"), `The ${interaction} destructive label lost the Danger role.`);
  assert(parseFloat(style.opacity) === 1, `The ${interaction} destructive control dims its label.`);
  const [red, green] = colourChannels(style.backgroundColor);
  assert(red > green, `The ${interaction} destructive control lost its red tint.`);
  assert(ratio >= 4.5, `Destructive label contrast is ${ratio.toFixed(2)}:1 while ${interaction}; expected at least 4.5:1.`);
  acceptance("destructive_text_contrast", { ...window.__PENNEL_CONTROL_ACCEPTANCE__?.destructive_text_contrast,
    [interaction]: { label: state.element.textContent.trim(), color: style.color, background: style.backgroundColor, font_size: style.fontSize, contrast: ratio } });
}
function control() {
  const state = window.__controlTarget;
  assert(state?.element.isConnected, "The control under test was replaced.");
  assert(JSON.stringify(selectionOf(state.element)) === JSON.stringify(state.selection), `${state.selector} lost its selected state.`);
  return state;
}

/** Targets real app controls. Only the disabled check temporarily changes their disabled property. */
export function rememberControl(selector) {
  const element = document.querySelector(selector);
  assert(element, `No control for ${selector}.`);
  element.dataset.controlTarget = "true";
  element.scrollIntoView({ block: "center", inline: "nearest" });
  window.__controlTarget = { element, selector, resting: appearanceOf(element), selection: selectionOf(element) };
}

export function checkControlHover() {
  const state = control();
  assert(state.element.matches(":hover"), `${state.selector} did not receive real pointer hover.`);
  state.hover = appearanceOf(state.element);
  checkProviderTextContrast(state, "hovered");
  assert(JSON.stringify(state.hover) !== JSON.stringify(state.resting), `${state.selector} does not change on hover.`);
  acceptance("control", { selector: state.selector, resting: state.resting, hover: state.hover, selection: state.selection });
}

export function checkControlPressed() {
  const state = control();
  assert(state.element.matches(":active"), `${state.selector} did not receive real pointer press.`);
  const pressed = appearanceOf(state.element);
  checkProviderTextContrast(state, "pressed");
  assert(JSON.stringify(pressed) !== JSON.stringify(state.hover), `${state.selector} looks the same pressed as hovered.`);
  acceptance("control", { ...window.__PENNEL_CONTROL_ACCEPTANCE__.control, pressed });
}

export function checkControlFocus(selector) {
  const element = document.querySelector(selector);
  assert(element, `No control for ${selector}.`);
  element.focus();
  const ring = ringOf(element);
  assert(ring.visible && ring.color === colorOf("var(--focus-ring)"), `${selector} lost its keyboard focus ring.`);
}

export async function disableControl() {
  const state = control();
  assert(state.element instanceof HTMLButtonElement, "The disabled check needs a real button.");
  state.element.disabled = true;
  await new Promise(resolve => setTimeout(resolve, 180));
  state.disabled = appearanceOf(state.element);
  acceptance("disabled_probe", "Existing app button temporarily disabled by the fixture; no injected product control.");
}

export function checkDisabledControl() {
  const state = control();
  assert(state.element.disabled, "The disabled button became enabled.");
  assert(JSON.stringify(appearanceOf(state.element)) === JSON.stringify(state.disabled), `${state.selector} changes on pointer input while disabled.`);
  acceptance("disabled", state.disabled);
}

/** Deliberately bypasses the reduced-motion CSS to prove the checker rejects a surviving animation. */
export function checkReducedMotionRejectsAnimation(emulated) {
  const style = document.createElement("style");
  style.textContent = "@keyframes control-negative-movement { from { transform: translateX(0); } to { transform: translateX(12px); } }";
  const probe = document.createElement("span");
  probe.textContent = "Synthetic movement negative control";
  document.head.append(style);
  document.body.append(probe);
  const rejected = [];
  try {
    for (const animation of ["control-negative-movement 2s linear infinite", "none 0s, control-negative-movement 2s linear infinite"]) {
      probe.style.setProperty("animation", animation, "important");
      let failed = false;
      try { checkReducedMotion(emulated); }
      catch (error) { if (/Reduced motion leaves movement:.*animation/.test(String(error))) failed = true; else throw error; }
      assert(failed, `${emulated ? "System" : "App"} reduced-motion check accepted a surviving movement animation.`);
      rejected.push(animation);
    }
  }
  finally { probe.remove(); style.remove(); }
  acceptance("animation_negative_control", { injected: true, rejected, route: emulated ? "system" : "app" });
}

/** Disables only the real switch's transition: declarations elsewhere must not satisfy the fade check. */
export function checkReducedMotionRejectsMissingFade(emulated) {
  const checkbox = document.querySelector('.settings-pane input[type="checkbox"]:not(:disabled)');
  assert(checkbox, "The fade negative control needs a real enabled settings switch.");
  const original = checkbox.getAttribute("style");
  checkbox.style.setProperty("transition-duration", "0s", "important");
  let rejected = false;
  try { checkReducedMotion(emulated); }
  catch (error) { if (/real settings switch.*short colour fade/.test(String(error))) rejected = true; else throw error; }
  finally { if (original === null) checkbox.removeAttribute("style"); else checkbox.setAttribute("style", original); }
  assert(rejected, `${emulated ? "System" : "App"} reduced-motion check accepted a missing real colour fade.`);
  acceptance("fade_negative_control", { injected: true, rejected, route: emulated ? "system" : "app" });
}
