# Scoped independent review of the T11c demo lifecycle fix

Pinned range: `194a81e6b218f7be5a78b445ba872b88ac9a6529` → `39885c91bc8a32deabfb54afec36305a55012cd1`. Reviewed only the one-file `pill.rs` correction, prepared diff, appended T11c report and pinned production/test paths against the P2 in `t11c-review.md`. No broader feature/native audit, source/index/HEAD changes, subagents, builds, suite reruns or CI were performed.

**Verdict: the original P2 is ADDRESSED. No new actionable regression was found in the correction scope. Ready for parent integration.**

## Ownership and native-action ordering

`show_demo` now reserves a generation before the first idle check or window creation. Creation cannot mint a new generation after an intervening real capture has invalidated that reservation. Post-creation admission requires the same generation, no real owner and a fresh idle result. The independent `real_visible` marker records real ownership before content/native presentation work, so an old request remains invalid even when the real capture has already returned to idle.

Real-pill transitions invalidate pending/active demo intent. Queued cancellation hides and timer expiry carry a generation; stale operations cannot hide a later admitted preview. A failed old Show request cancels only its own generation. Creation errors still reconcile the resulting intended visibility and return the error rather than creating a capture or retry.

The new reconciliation path snapshots intended visibility, calls native show/hide without holding the demo guard, and compares the latest generation/visibility afterward. Reentrant ownership changes cause the native result to be reconciled to the latest intent. This covers both capture entering during native show and a newer Show entering during an old native hide. Preference refresh, admission, cancellation and expiry use that path rather than applying an unguarded native action.

The idle predicate still checks actual dictation activity and queued jobs. The existing real-owner state covers visible listening/working/notices without depending on a stale content snapshot. The correction does not change geometry, the 2.4-second timer duration, theme hierarchy, renderer interfaces, private content access or focus/nonactivation/click-through flags. It adds no new mutex or UI architecture.

## Inspected evidence

No tests were rerun. The appended report records three old-order controlled failures. The new tests exercise the production creation/native-call boundaries and cover:

- capture cancelling during creation, including capture returning idle before a late admission;
- a real-owner marker ahead of content/idle observations;
- stale queued cancellation against a newer valid demo;
- capture reentry during native show with the state lock demonstrably available;
- native hide reentry preserving a newer Show;
- existing stale timer rejection.

I inspected `/tmp/pennel-t11c-demo-race-verified.log`: **20 passed, 0 failed**, including the controlled ownership/reentry cases. Strict backend clippy completed successfully in `/tmp/pennel-t11c-demo-race-clippy-noincremental.log`. The report records format/diff/clean-status checks. Incremental compilation was disabled to avoid the reported ENOSPC condition; no shared cache deletion or broad suite repetition is inferred.

These are focused state/native-boundary tests rather than physical desktop interaction. Final integrated native checks remain the parent's verification scope.

Original finding: **1 addressed, 0 unaddressed**. New correction-scope findings: **0**. No further correction is requested.
