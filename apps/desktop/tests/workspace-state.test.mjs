import assert from "node:assert/strict";
import test from "node:test";
import { WorkspaceNavigation, historyEntryId, historyRoute, legacyWorkspaceRoute, parseWorkspaceRoute, resumesOtherCapture, routeKey, samePadRoute, settingsCategory, windowForSection, windowMode, workspaceRoute, workspaceTask } from "../src/workspace-state.ts";

test("all product sections and legacy window aliases belong to the main workspace", () => {
  assert.equal(windowForSection("settings"), "workspace");
  assert.equal(windowForSection("history"), "workspace");
  assert.equal(windowForSection("dictation"), "workspace");
  assert.equal(windowForSection("meetings"), "workspace");
  assert.equal(windowForSection("recording"), "workspace");
  assert.equal(windowMode("settings"), "workspace");
  assert.equal(windowMode("dictation"), "workspace");
  assert.equal(windowMode("history"), "workspace");
  assert.equal(windowMode("library"), "workspace");
  assert.equal(windowMode(null), "workspace");
  assert.deepEqual(legacyWorkspaceRoute("workspace"), workspaceRoute("meetings"));
  assert.deepEqual(legacyWorkspaceRoute("dictation", "pad"), workspaceRoute("dictation", "pad"));
  assert.deepEqual(new WorkspaceNavigation(workspaceRoute("history")).current, workspaceRoute("history"));
});

test("primary navigation restores the task context while History never repeats an exact-target request", () => {
  const nav = new WorkspaceNavigation();
  nav.set(workspaceRoute("dictation", "pad"));
  nav.set(historyRoute("a-saved-result"));
  nav.set(workspaceRoute("meetings", null, "a-meeting"));
  assert.deepEqual(nav.select("dictation"), historyRoute());
  assert.deepEqual(nav.select("history"), workspaceRoute("history"));
  nav.set(workspaceRoute("dictation", "home"));
  nav.set(workspaceRoute("settings", "models"));
  assert.deepEqual(nav.settingsBack(), workspaceRoute("dictation", "home"));
  assert.deepEqual(legacyWorkspaceRoute("dictation", "home"), workspaceRoute("dictation", "home"));
});

test("inline Settings returns to the originating task and remembers its last category", () => {
  const meeting = workspaceRoute("meetings", null, "2026-10-24 Projectoverleg");
  const nav = new WorkspaceNavigation(meeting);
  nav.set(workspaceRoute("settings", "models"));
  nav.set(workspaceRoute("settings", "appearance"));
  assert.deepEqual(nav.settingsBack(), meeting);
  nav.set(nav.settingsBack());
  assert.deepEqual(nav.select("settings"), workspaceRoute("settings", "appearance"));
  assert.deepEqual(new WorkspaceNavigation(workspaceRoute("settings", "models")).select("settings"), workspaceRoute("settings", "models"));
});

test("a recording finished during Settings updates the return destination without leaving Settings", () => {
  const settings = workspaceRoute("settings", "overview");
  const nav = new WorkspaceNavigation(workspaceRoute("recording", null, "pending-meeting"));
  nav.set(settings);
  nav.recordingFinished("saved-meeting");
  assert.deepEqual(nav.current, settings);
  assert.deepEqual(nav.settingsBack(), workspaceRoute("meetings", null, "saved-meeting"));
  const normal = new WorkspaceNavigation(workspaceRoute("meetings", null, "another-meeting"));
  normal.set(settings); normal.recordingFinished("saved-meeting");
  assert.equal(normal.settingsBack().meeting_id, "another-meeting");
  assert.deepEqual(new WorkspaceNavigation(settings).settingsBack(), workspaceRoute("dictation", "home"));
});

test("background meeting completion keeps History open and updates the remembered meeting", () => {
  const nav = new WorkspaceNavigation(workspaceRoute("recording", null, "pending-meeting"));
  nav.set(historyRoute());
  nav.recordingFinished("finished-meeting");
  assert.deepEqual(nav.current, historyRoute());
  assert.deepEqual(nav.select("meetings"), workspaceRoute("meetings", null, "finished-meeting"));
  nav.set(workspaceRoute("meetings", null, "another-meeting"));
  nav.set(workspaceRoute("dictation", "home"));
  nav.recordingFinished("unrelated-meeting");
  assert.equal(nav.select("meetings").meeting_id, "another-meeting");
});

test("legacy windows and settings aliases now land in one workspace", () => {
  assert.deepEqual(legacyWorkspaceRoute("settings", "meetings"), workspaceRoute("settings", "meeting-settings"));
  assert.deepEqual(legacyWorkspaceRoute("settings", "local"), workspaceRoute("settings", "local"));
  assert.deepEqual(legacyWorkspaceRoute("history"), workspaceRoute("history"));
  assert.deepEqual(legacyWorkspaceRoute("live", "meeting"), workspaceRoute("recording"));
});

test("new Settings opens at task readiness while old advanced links stay valid", () => {
  assert.deepEqual(legacyWorkspaceRoute("settings"), workspaceRoute("settings", "dictation"));
  assert.deepEqual(legacyWorkspaceRoute("settings", "models"), workspaceRoute("settings", "models"));
  assert.deepEqual(parseWorkspaceRoute({ section: "settings", tab: "overview" }), workspaceRoute("settings", "overview"));
  assert.deepEqual(new WorkspaceNavigation().select("settings"), workspaceRoute("settings", "dictation"));
  assert.equal(parseWorkspaceRoute({ section: "meetings", meeting_id: "overview" }), null);
});

test("History belongs to Dictation and recording belongs to Meetings", () => {
  assert.equal(workspaceTask(historyRoute("legacy-recovery")), "dictation");
  assert.equal(workspaceTask(workspaceRoute("dictation", "pad")), "dictation");
  assert.equal(workspaceTask(workspaceRoute("recording")), "meetings");
});

test("only a verified different active capture may bypass old retained draft persistence", () => {
  const active = { recording: true, id: "new-meeting", session_id: "new-session" };
  const route = workspaceRoute("recording", null, active.id);
  assert.equal(resumesOtherCapture(route, "old-session", active), true);
  assert.equal(resumesOtherCapture(workspaceRoute("recording"), "old-session", active), true);
  for (const destination of [workspaceRoute("settings"), workspaceRoute("meetings", null, active.id), workspaceRoute("dictation"), workspaceRoute("recording", null, "other-meeting")]) {
    assert.equal(resumesOtherCapture(destination, "old-session", active), false);
  }
  assert.equal(resumesOtherCapture(route, active.session_id, active), false);
  assert.equal(resumesOtherCapture(route, null, active), false);
  assert.equal(resumesOtherCapture(route, "old-session", { ...active, recording: false }), false);
  assert.equal(resumesOtherCapture(route, "old-session", { ...active, session_id: null }), false);
  assert.equal(resumesOtherCapture(route, "old-session", { ...active, id: null }), false);
});

test("six settings categories retain every old setup destination", () => {
  for (const tab of ["dictation", "styles", "overview", null]) assert.equal(settingsCategory(tab), "dictation");
  assert.equal(settingsCategory("dictionary"), "dictionary");
  for (const tab of ["meeting-settings", "recording", "meetings"]) assert.equal(settingsCategory(tab), "meeting-settings");
  for (const tab of ["models", "providers", "local", "advanced"]) assert.equal(settingsCategory(tab), "advanced");
  assert.equal(settingsCategory("appearance"), "appearance"); assert.equal(settingsCategory("account"), "account");
});

test("remembered workspace survives a launch with its Settings return and task contexts", () => {
  const nav = new WorkspaceNavigation(workspaceRoute("dictation", "pad"));
  nav.set(workspaceRoute("meetings", null, "Chosen meeting", 1234));
  nav.set(workspaceRoute("dictation", "pad"));
  nav.set(workspaceRoute("settings", "appearance"));
  const restored = WorkspaceNavigation.restore(JSON.parse(JSON.stringify(nav.snapshot())));
  assert.deepEqual(restored.current, workspaceRoute("settings", "appearance"));
  assert.deepEqual(restored.settingsBack(), workspaceRoute("dictation", "pad"));
  assert.deepEqual(restored.select("meetings"), workspaceRoute("meetings", null, "Chosen meeting"));
  assert.deepEqual(restored.select("dictation"), workspaceRoute("dictation", "pad"));
});

test("persisted navigation excludes source content and one-shot History recovery requests", () => {
  const nav = new WorkspaceNavigation(workspaceRoute("meetings", null, "Owned meeting", 1234, { kind: "notes", start_ms: null, text: "Private source text" }));
  nav.set(historyRoute("one-shot-recovery"));
  const saved = JSON.stringify(nav.snapshot());
  assert(!saved.includes("Private source text")); assert(!saved.includes("one-shot-recovery"));
  const restored = WorkspaceNavigation.restore(JSON.parse(saved));
  assert.deepEqual(restored.current, historyRoute());
  assert.deepEqual(restored.select("meetings"), workspaceRoute("meetings", null, "Owned meeting"));
  assert.equal(WorkspaceNavigation.restore({ version: 1, current: { section: "invalid" } }), null);
});

test("recovery opens a specific History result without putting its id in a meeting path", () => {
  const id = "019c6e27-e55b-73d1-87d8-4e01f1f75043";
  const route = workspaceRoute("history", `entry:${id}`);
  assert.deepEqual(historyRoute(id), route);
  assert.equal(historyEntryId(route.tab), id);
  assert.deepEqual(legacyWorkspaceRoute("history", route.tab), route);
  assert.deepEqual(parseWorkspaceRoute(route), route);
  assert.deepEqual(historyRoute(), workspaceRoute("history"));
  for (const tab of ["entry:", "entry:../history", "entry:<script>", "entry:%3Cscript%3E", "entry:result\n", `entry:${"x".repeat(129)}`, "other"]) {
    assert.equal(historyEntryId(tab), null);
    assert.equal(parseWorkspaceRoute({ section: "history", tab }), null);
    assert.deepEqual(legacyWorkspaceRoute("history", tab), workspaceRoute("history"));
  }
});

test("targeted navigation validates meeting identifiers and timestamp jumps", () => {
  assert.deepEqual(parseWorkspaceRoute({ section: "meetings", meeting_id: "2026-10-25 Productoverleg", jump_ms: 1234 }), workspaceRoute("meetings", null, "2026-10-25 Productoverleg", 1234));
  for (const value of ["../meeting", "a/b", "a\\b", "a:b", "a\n", "..", "settings"]) assert.equal(parseWorkspaceRoute({ section: "meetings", meeting_id: value }), null);
  assert.equal(parseWorkspaceRoute({ section: "other" }), null);
  assert.equal(parseWorkspaceRoute({ section: "settings", tab: "unknown" }), null);
});

test("switching primary destinations preserves the selected meeting and settings category", () => {
  const nav = new WorkspaceNavigation();
  nav.set(workspaceRoute("meetings", null, "Productoverleg", 5000));
  nav.set(workspaceRoute("settings", "models"));
  nav.set(workspaceRoute("history"));
  assert.deepEqual(nav.select("meetings"), workspaceRoute("meetings", null, "Productoverleg"));
  assert.deepEqual(nav.select("settings"), workspaceRoute("settings", "models"));
  assert.notEqual(routeKey(nav.select("meetings")), routeKey(nav.current));
});

test("recording compatibility routes do not erase the remembered meetings route", () => {
  const nav = new WorkspaceNavigation();
  nav.set(workspaceRoute("meetings", null, "Projectoverleg"));
  nav.set(parseWorkspaceRoute({ section: "meetings", tab: "recording" }));
  assert.equal(nav.current.section, "recording");
  assert.equal(nav.select("meetings").meeting_id, "Projectoverleg");
});

test("reselecting the active Pad keeps its session while same-route provider selection can remount", () => {
  assert.equal(samePadRoute(workspaceRoute("dictation"), workspaceRoute("dictation")), true);
  assert.equal(samePadRoute(workspaceRoute("settings", "providers"), workspaceRoute("settings", "providers")), false);
  assert.equal(samePadRoute(workspaceRoute("dictation"), workspaceRoute("history")), false);
  assert.equal(samePadRoute(workspaceRoute("history"), workspaceRoute("dictation")), false);
});

test("the optional Pad has its own route and is remembered across workspace navigation", () => {
  const nav = new WorkspaceNavigation();
  const pad = workspaceRoute("dictation", "pad");
  assert.equal(samePadRoute(nav.current, pad), false);
  nav.set(pad);
  nav.set(workspaceRoute("meetings"));
  assert.deepEqual(nav.select("dictation"), pad);
  nav.set(pad);
  assert.deepEqual(nav.select("dictation"), workspaceRoute("dictation", "home"));
  nav.set(historyRoute("exact-result"));
  assert.deepEqual(nav.select("dictation"), workspaceRoute("dictation", "home"));
  assert.equal(samePadRoute(pad, workspaceRoute("dictation")), false);
});

test("a background meeting rename updates return metadata without changing the user's destination", () => {
  const nav = new WorkspaceNavigation(workspaceRoute("meetings", null, "Original"));
  nav.set(workspaceRoute("meetings", null, "Original"));
  nav.set(workspaceRoute("settings", "models"));
  const settings = { ...nav.current };
  nav.meetingRenamed("Original", "Renamed");
  assert.deepEqual(nav.current, settings);
  assert.equal(nav.settingsBack().meeting_id, "Renamed");
  assert.equal(nav.select("meetings").meeting_id, "Renamed");
  nav.set(workspaceRoute("recording", null, "Another meeting"));
  nav.meetingRenamed("Renamed", "Renamed again");
  assert.equal(nav.current.meeting_id, "Another meeting");
  nav.set(workspaceRoute("meetings", "review", "Renamed again"));
  nav.meetingRenamed("Renamed again", "Final name");
  assert.deepEqual(nav.current, workspaceRoute("meetings", "review", "Final name"));
});
