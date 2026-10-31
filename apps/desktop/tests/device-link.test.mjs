import assert from "node:assert/strict";
import test from "node:test";
import { connectionFeedback, connectionRole, linkIsPending, qrImageSource, syncRequiresBackup, syncSetupAction } from "../src/device-link.ts";

test("setup uses validated library access, rather than the enabled switch, to select connection role", () => {
  assert.equal(connectionRole({ can_approve_devices: true, sync_enabled: false }), "trusted_device");
  assert.equal(connectionRole({ can_approve_devices: false, sync_enabled: true }), "new_device");
  assert.equal(syncSetupAction({ phase: "locked", sync_enabled: false }), "unlock");
  assert.equal(syncSetupAction({ phase: "paused" }), "resume");
});

test("native pending backup survives missing frontend acknowledgment bookkeeping", () => {
  assert.equal(syncRequiresBackup({ phase: "ready", sync_enabled: false }, null), true);
  assert.equal(syncRequiresBackup({ phase: "ready", sync_enabled: false }, true), true);
  assert.equal(syncRequiresBackup({ phase: "paused", sync_enabled: false }, false), true);
  assert.equal(syncRequiresBackup({ phase: "paused", sync_enabled: false }, null), false);
  assert.equal(syncRequiresBackup({ phase: "active", sync_enabled: true }, false), false);
});

test("pairing stops polling for every terminal state and retains provisioning", () => {
  for (const phase of ["idle", "complete", "cancelled", "expired", "failed"]) assert.equal(linkIsPending(phase), false, phase);
  for (const phase of ["waiting_peer", "confirming", "awaiting_approval", "waiting_approval", "provisioning"]) assert.equal(linkIsPending(phase), true, phase);
});

test("QR SVG stays an inert bounded image instead of relay-controlled DOM", () => {
  assert.equal(qrImageSource(null), null);
  assert.equal(qrImageSource("<script>bad()</script>"), null);
  assert.equal(qrImageSource("<svg>" + "x".repeat(262144)), null);
  const source = qrImageSource('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>');
  assert.match(source, /^data:image\/svg\+xml;charset=utf-8,%3Csvg/);
  assert.ok(!source.includes("<svg"));
});

test("a connection failure has one message and suppresses its failed phase", () => {
  const cached = { phase: "failed", error_code: "pairing_unavailable", error_message: "private upstream body" };
  const failure = { code: "http_403", message: "latest private upstream body" };
  const display = connectionFeedback(cached, failure);
  assert.equal(display.status, null);
  assert.equal(display.error, failure);
  assert.deepEqual(connectionFeedback(cached, null), { status: null, error: { code: "pairing_unavailable" } });
  assert.deepEqual(connectionFeedback({ phase: "failed", error_code: null, error_message: null }, null), { status: null, error: { code: "error" } });
});

test("pending connection status survives a retryable transport failure", () => {
  const failure = { code: "network" };
  assert.deepEqual(connectionFeedback({ phase: "waiting_approval" }, failure), { status: "waiting_approval", error: failure });
  assert.deepEqual(connectionFeedback(null, null), { status: null, error: null });
  assert.deepEqual(connectionFeedback({ phase: "complete" }, null), { status: null, error: null });
});
