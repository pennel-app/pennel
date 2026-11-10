# Scoped independent review of the completed T11b fixes

Pinned range: `5d569c8cc0fc8be0aacfe6172d77a812cbe39aaa` → `78caeaaf9c94bdfc91abbb55115e34bfffc7ad63`. Reviewed only the prepared completed-feature correction diff, appended T11b report and pinned source against the two P2 findings in `t11b-review.md`. Restored T11c appearance/geometry work and later X11 dependency corrections were excluded. No source/index/HEAD changes, subagents, builds, suite reruns or CI were performed.

**Verdict: both original findings are ADDRESSED. No new actionable regression was found in the correction scope. Ready for parent integration.**

| Original finding | Status | Pinned correction evidence |
| --- | --- | --- |
| Capture A's retained model/high generation suppresses capture B's generation-zero presentation | ADDRESSED | `apps/desktop/src-tauri/src/pill.rs:98–132` clears model metadata only after accepting a genuinely different preview session, and generation ordering is restricted to the same model session. Same-session repeated begin remains non-destructive, while current-preview identity still rejects old captures. The backend regression combines A generation 2 → Working → B generation 0, verifies immediate model clear and B acceptance, rejects late A generation 99 and preserves repeated B. |
| The Settings row calls every unavailable existing Next binding a missing shortcut | ADDRESSED | `apps/desktop/src/views/dictation.ts:112–126` consumes the actual capability reason through the existing localization path, uses a readable fallback for absent/unknown runtime reasons, and reserves missing-binding guidance for a genuinely absent binding. Existing unavailable bindings no longer fall through to the assignment instruction. New EN/NL aliases cover the expected permission/native/platform reason codes, including the saved switch being off. |

## Correction-scope behavior

The frontend preview handler also resets model metadata/confirmation ownership and cancels the old model timer when the accepted preview's session changes (`apps/desktop/src/pill.ts:216–230`). It repaints the current status and chip immediately instead of waiting for the next model response. Ordinary same-session caption revisions do not reset the model or acknowledgement. A null preview clears model state. This closes the related visible stale-chip gap without changing Working/Problem priority, provider selection, native listening readiness or click-through behavior.

The settings correction leaves the binding and toggle editable and does not save a new setting merely to display a reason. Available bindings retain the actual shortcut label; fewer-model/missing-binding states retain their respective guidance when no higher-priority concrete capability reason is supplied. Missing reason metadata on an existing unavailable binding receives a general unavailability explanation rather than a false assignment instruction. The localizations are parallel English/Dutch and use app-owned status channels.

No core model-selection, final audio/provider request, capture target, credentials, public wire shape, native focus/cursor behavior or appearance geometry changed in this correction. The production changes remain bounded to metadata ownership, timer reset and truthful Settings recovery copy.

## Inspected regression receipts

No tests were rerun. The appended report documents three observed reds: retained backend model metadata, the browser's stale chip after preview rollover, and the immutable original source's wrong unavailable-binding guidance. I inspected the supplied green receipts:

- `/tmp/pennel-t11b-review-state-green.log`: **16 focused pill tests passed**, including the combined capture/model regression. As the report states, this command ran in the current working tree; it is not relabeled here as a fresh full suite at the immutable correction pin.
- `/tmp/pennel-t11b-review-browser-final.log`: **8 focused EN/NL × light/dark owned mock checks**, zero failures, exercising rollover and unavailable-reason presentation. The interaction source covers hook, Accessibility, native-key and Wayland reasons, an existing binding with no reason, and the actual switch-off setting.

These are focused regression/synthetic UI receipts. Native input, signed-device and appearance acceptance were not revisited; the prior T11b review's disclosed limits remain.

Original findings: **2 addressed, 0 unaddressed**. New correction-scope findings: **0**. No further correction is requested by this review.
