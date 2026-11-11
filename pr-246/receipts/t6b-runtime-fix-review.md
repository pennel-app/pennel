# Scoped independent review of the T6b receipt barrier correction

Production correction range: `25f4054f82208a4e58ae77823ddcf4cec91b5cd9` → `c0e5dcb701120f201792c6ee886b6546fb1eb1e0`. Final test-only closure: `c0e5dcb701120f201792c6ee886b6546fb1eb1e0` → `25d5ca61dfed625deead8d6ad96e69fce920e120`. Reviewed only the prepared correction diffs, appended reports, pinned producer/consumer paths and new regressions against the original terminal-before-receipt P2. No source/index/HEAD changes, subagents, builds, suite reruns or CI were performed.

**Final verdict: the original production finding and the test-only waiter finding are both ADDRESSED. No new actionable regression was found in either correction scope. Ready for parent integration.**

## Original finding status

The new `finalization_finished` flag starts false and is published with a revision increment only from `AdmittedInsightAnalysis::drop`, after the receipt write attempt, inference registration cleanup and cancellation-registration removal (`core/src/meeting_insight_engine.rs:840–870`). A failed receipt write or unavailable current meeting still reaches this finite completion barrier. The existing attempt remains consumed; completion does not imply that a failed write persisted a terminal outcome.

The helper now requires terminal state, no physical request and this separate finalization flag before its final durable refresh/progress (`apps/desktop/src-tauri/src/automatic_suggestions.rs:309–313`). Immediate logical cancellation and the real physical-request status are unchanged. Job busy accounting also remains active until finalization, preventing replacement/eviction through that gap. Begin/admission error paths drop their prepared admission and therefore reach the same cleanup barrier; successful, failed and cancelled worker outcomes all finalize through Drop.

The additive flag is ephemeral job metadata, not a persisted schema or policy change. Existing event filtering remains metadata-only. The helper does not poll for a receipt outcome that may never be writable and does not introduce a retry, guessed delay, qualification bypass or new caller.

## New correction-scope finding

### [P2, test only — ADDRESSED at 25d5ca61] Wait for the finalization flag in the returned snapshot, not only a separately observed live retirement

**Location:** `core/src/meeting_insight_engine.rs:2754–2757` in the new `terminal_receipt_boundary` regression, using `wait_job` at `3136–3151`.

**Evidence:** `wait_job` first reads a JSON job snapshot, then independently looks up the live job and tests `!job.busy()`. Its return condition checks terminal state, no physical request and that later live busy result; it does not check the captured snapshot's `finalization_finished`. The new test then asserts that the returned snapshot has `finalization_finished = true`.

An allowed scheduling order is: snapshot captures terminal/physically-idle with finalization false; worker completes Drop and closes retention; the separate busy check sees the now-retired job; the waiter returns the older false snapshot. The new assertion fails despite correct production behavior. The 46-pass receipt shows a passing run, but does not eliminate this interleaving. The worker is retired in this case, so this is a flaky assertion rather than a production barrier failure or abandoned-worker leak.

**Minimal fix:** include `state["finalization_finished"] == true` in `wait_job`'s existing terminal return condition, or obtain a fresh snapshot after the live retirement check before returning it. Keep the existing deadline and physical/retention checks. This is a bounded test-helper change, not a request for another product mechanism or broad audit.

**Confidence:** high; the snapshot and live-busy observations are separate, and the new assertion requires them to refer to the same finalization phase.

**Closure:** the single added condition at `meeting_insight_engine.rs:3149` requires `state["finalization_finished"] == true` in the captured snapshot before returning it. The eight-second deadline, terminal check, physical check, live retention check and bounded polling remain unchanged. This directly excludes the stale false-snapshot interleaving above. No production code changed, and no new test-helper regression was found in the one-line diff.

## Verification evidence and limits

No checks were rerun. I inspected the appended red/green report and completed logs:

- Producer regressions use an actual qualified test transport plus the real storage replacement gate to hold receipt handling while terminal/physically-idle metadata is visible. They cover completed, failed and cancelled outcomes and an unwritable marker fault. Latches are released before assertions, and the original attempt stays consumed.
- Consumer regressions separately delay receipt readiness and model a failed write with no outcome, checking one bounded additional wait, the durable notification and no re-admission/retry.
- `t6b-runtime-receipt-core-tests.log`: **46 passed, 0 failed**, 6.95 s.
- `t6b-runtime-receipt-backend-tests.log`: **11 passed, 0 failed**, 0.17 s.
- Strict core/backend clippy completed successfully in **11.55 s / 29.89 s**, with the assigned checkout paths in the logs. Formatting and diff checks are recorded as passed.

The closure receipt `t6b-runtime-waiter-tests.log` was also inspected: both `terminal_receipt` producer regressions passed (**2 passed, 0 failed**, 0.84 s), covering completed/failed/cancelled ordering and failed-write retirement. Formatting/diff checks are recorded as passed. No tests were rerun by this reviewer.

Original production finding: **1 addressed, 0 unaddressed**. Test-only follow-up: **1 addressed, 0 unaddressed**. New correction-scope findings: **0**. The production producer/consumer pair and stable waiter correction are ready together; no further correction is requested.
