import assert from "node:assert/strict";
import test from "node:test";
import { desktopCommand, desktopEnvironment, desktopShortcut, nativeChromeIsTransparent, resizedSidebarWidth, sidebarPresentation, SIDEBAR } from "../src/desktop-chrome-state.ts";

const environment = { material: "liquid_glass", appearance: "light", reduce_transparency: false, reduce_motion: false, increase_contrast: false, titlebar_height: 28, focused: true };

test("native transparency requires a complete successful environment and respects accessibility", () => {
  assert.deepEqual(desktopEnvironment(environment), environment);
  assert.equal(desktopEnvironment({ ...environment, titlebar_height: Infinity }), null);
  assert.equal(desktopEnvironment({ ...environment, reduce_transparency: undefined }), null);
  assert.equal(desktopEnvironment({ ...environment, material: "css_blur" }), null);
  assert.equal(nativeChromeIsTransparent(environment), true);
  for (const patch of [{ material: "unavailable" }, { material: "opaque" }, { reduce_transparency: true }, { increase_contrast: true }]) {
    assert.equal(nativeChromeIsTransparent({ ...environment, ...patch }), false);
  }
  assert.equal(nativeChromeIsTransparent({ ...environment, reduce_motion: true }), true);
});

test("sidebar preferences recover from invalid values and remain bounded for pointer and keyboard", () => {
  assert.deepEqual(sidebarPresentation(null), { width: 220, visible: true });
  assert.deepEqual(sidebarPresentation({ width: 230, visible: false }), { width: 230, visible: false });
  assert.deepEqual(sidebarPresentation({ width: -1, visible: false }), { width: 200, visible: false });
  assert.equal(sidebarPresentation({ width: 900 }).width, 320);
  assert.equal(sidebarPresentation({ width: NaN }).width, SIDEBAR.width);
  assert.equal(resizedSidebarWidth(230, "ArrowLeft"), 226);
  assert.equal(resizedSidebarWidth(320, "ArrowRight"), 320);
  assert.equal(resizedSidebarWidth(230, "Home"), 200);
  assert.equal(resizedSidebarWidth(230, "End"), 320);
  assert.equal(resizedSidebarWidth(230, "Enter"), null);
});

test("application shortcuts use platform modifiers and leave text navigation and composition alone", () => {
  const event = (key, changes = {}) => ({ key, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, isComposing: false, repeat: false, ...changes });
  assert.equal(desktopShortcut(event("n"), "macos"), "new_recording");
  assert.equal(desktopShortcut(event("s", { ctrlKey: true }), "macos"), "toggle_sidebar");
  assert.equal(desktopShortcut(event("s", { metaKey: false, ctrlKey: true, shiftKey: true }), "windows"), "toggle_sidebar");
  assert.equal(desktopShortcut(event("f", { metaKey: false, ctrlKey: true }), "linux"), "find");
  assert.equal(desktopShortcut(event("1"), "macos"), "dictation");
  assert.equal(desktopShortcut(event("n", { repeat: true }), "macos"), null);
  assert.equal(desktopShortcut(event("n", { isComposing: true }), "macos"), null);
  assert.equal(desktopShortcut(event("n", { ctrlKey: true }), "macos"), null);
  assert.equal(desktopShortcut(event("ArrowLeft"), "macos"), null);
  assert.equal(desktopCommand("settings"), "settings");
  assert.equal(desktopCommand({ command: "find" }), "find");
  assert.equal(desktopCommand("delete_all"), null);
});

test("effective command rows match logical keys exactly and suspend commands during capture", () => {
  const event = { key: "a", code: "KeyQ", metaKey: true, altKey: true, ctrlKey: false, shiftKey: false, repeat: false, isComposing: false };
  const rows = [{ id: "find", binding: { mods: ["alt", "meta"], code: "KeyQ", key: "a" }, capability: { available: true, reason: null } }];
  assert.equal(desktopShortcut(event, "macos", rows), "find");
  assert.equal(desktopShortcut({ ...event, key: "q" }, "macos", rows), null);
  assert.equal(desktopShortcut({ ...event, shiftKey: true }, "macos", rows), null);
  assert.equal(desktopShortcut({ ...event, keyCode: 229 }, "macos", rows), null);
  assert.equal(desktopShortcut(event, "macos", rows, true), null);
  assert.equal(desktopShortcut(event, "macos", [{ ...rows[0], binding: null }]), null);
  assert.equal(desktopShortcut(event, "macos", [{ ...rows[0], capability: { available: false, reason: "unavailable" } }]), null);
});


test("Space aliases use logical characters and preserve all command guards", () => {
  const row = { id: "find", binding: { mods: ["ctrl", "alt"], code: "Space", key: "Space" }, capability: { available: true, reason: null } };
  const event = { key: " ", code: "Space", ctrlKey: true, altKey: true, metaKey: false, shiftKey: false, repeat: false, isComposing: false };
  assert.equal(desktopShortcut(event, "windows", [row]), "find");
  assert.equal(desktopShortcut(event, "linux", [{ ...row, binding: { ...row.binding, key: " " } }]), "find");
  for (const patch of [{ shiftKey: true }, { repeat: true }, { isComposing: true }]) assert.equal(desktopShortcut({ ...event, ...patch }, "windows", [row]), null);
  assert.equal(desktopShortcut(event, "windows", [row], true), null);
});
