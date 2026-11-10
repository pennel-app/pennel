# Scoped independent review of the T11b observer-drain correction

Reviewed pin: `9e3201117526364ed64eabdbaffcaa85618ac0f5`, using the exact one-file correction diff, appended T11b report and pinned `core/src/live_dictation.rs`. The scope is the recognition/emission lock inversion during an in-flight observer callback. T11c working-tree appearance work, UI architecture and unrelated feature/dependency changes were excluded. No source/index/HEAD changes, subagents, builds, test reruns or CI were performed.

**Verdict: the observer-drain lock-order issue is ADDRESSED. No new actionable regression was found in this correction scope. Ready for parent integration.**

## Lock order and serialization

`select_speech_model` still owns the finishing gate for the complete operation. It reads the current model through the existing getter, releases that short recognition read, performs the same readiness-validated atomic choice/account selection, and retains the existing no-op path (`live_dictation.rs:404–414`).

For a real switch, `PreviewState::stop(false)` now drains the emission boundary before acquiring the recognition mutation guard (`415–416`). An already-emitting observer can therefore finish its allowed `mode()` / `speech_model()` reads; it is no longer blocked by a recognition guard held by the selector waiting on that observer's emission mutex. Once the drain returns, preview acceptance is paused. Old local work is then cancelled/joined before the unchanged generation advancement and presentation replacement. The later clear notification remains outside the recognition guard, and new preview acceptance resumes only after that clear.

The finishing gate still serializes another selection and final processing against the switch. No new mutex or alternate state owner was added. Provider snapshot selection, original mode/language/vocabulary, final buffer/request, capture/history identity and owned/external target are unchanged by this movement. Cancellation checks and old-generation rejection remain in place.

The new actual concurrent observer regression pauses an old-provider emission, starts a switch, resumes the observer's public getters and verifies they can read the original presentation while the switch is draining. It then checks successful switch completion and the selected model. This complements the earlier switch-clear callback re-entry test rather than replacing it.

## Inspected verification receipts

No checks were rerun. The appended report records the old lock order timing out in the new regression. I inspected the completed green logs:

- `/tmp/pennel-pill-observer-drain-green.log`: **45 passed, 0 failed, 1 ignored**, including concurrent drain/getter access, switch-clear getter re-entry, current-provider final-pass/identity and stale-generation coverage.
- `/tmp/pennel-pill-observer-drain-clippy.log`: strict core clippy completed successfully.
- `/tmp/pennel-pill-observer-drain-build.log`: release library build completed successfully.

These remain focused correction receipts; no full UI/native/device acceptance is inferred. Final integrated verification belongs to the parent.

Original lock-order issue: **1 addressed, 0 unaddressed**. New correction-scope findings: **0**. No further correction is requested.
