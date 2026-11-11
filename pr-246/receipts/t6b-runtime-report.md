# T6b — automatic suggestions desktop runtime prerequisite

Status: standalone helper/bindings implementation and assigned automated verification complete. The meeting owner wires the one postprocess call after T6a; no caller/UI change is included here.

Base: `3419e021ac72b086066d9c95fdd79678fb7883f1`.
Branch: `codex/redesign-automatic-suggestions-runtime`.
Head: `25f4054f82208a4e58ae77823ddcf4cec91b5cd9`.
The completed `codex/redesign-summary-provenance` ref remains `26892114fe1b1f01f02181eb0ea9426a4387b096`; T9 reports remain unchanged in this task.

## Exact integration hook

```
crate::automatic_suggestions::run_after_summary(app.clone(), shared.clone(), id.clone());
```

Invoke once on the existing postprocess worker AFTER speaker recognition and AFTER `summarize_meeting_automatically` returns, including its error/skip result, before the caller's final refresh. Do not invoke from availability, storage retry, failed-transcription recovery or retranscription. `run_after_summary` is synchronous only on that worker; capture/finalization and the summary have already returned. It skips while quitting/updating and owns a LibraryJob while monitoring. The entry point has a narrowly annotated staged dead-code allowance, which can be removed when the caller is wired.

Agreed directly with `/root/build_recording_capsule`; that agent owns `meeting_runtime.rs`, processing UI and the single caller insertion.

## Behavior

- Calls the real core automatic admission API. Credentials are read ONLY after `Ok(Some(admission))`, and after a second stop/ownership check. No adopted registry record is added and no qualification bypass exists.
- Starts the existing `begin_meeting_insight_analysis_json` job. Core retains qualification, frozen language/setup/source/credential/policy guards, <=90-second deadline, <=512 utterances, busy limits, durable attempt markers and commit ownership.
- Polls actual owned core job metadata, coalesces unchanged progress, and keeps monitoring a logically cancelled job while core reports `physical_running = true`.
- Quitting/updating cancels only the owned job ID. A returned job must match the meeting UID before it becomes owned; unexpected begin identifiers are never cancelled. Subsequent mismatched/regressing responses cannot publish or cancel replacement work. Folder rename follows the stable meeting UID.
- Publishes the existing `meeting-insights-changed` metadata notification for real job changes. Core response fields such as items, notes, endpoint and error text are discarded before publishing.
- Emits `meeting-updated` and pokes cloud only for the guaranteed durable admission marker or observed changes to the owned store revision/attempt receipt. Transient job progress alone does not poke sync.
- Adds no listener/scheduler/retry trigger and never changes policy/settings, notes or summary Markdown.

## Progress event contract

`meeting-postprocess` is emitted to the main window only:

```
{
  id: string,
  meeting_uid: string,
  operation_id: string,
  stage: "suggestions",
  state: "started" | "finished" | "failed" | "skipped" | "cancelled"
}
```

The operation ID is process-local and unique across helper invocations. Job notifications contain only job ID, meeting UID, revision, state and physical-running metadata. No transcript, proposal, note, provider error body or credential enters these events.

## Read-only availability binding

`api.automaticInsightsAvailability(id): Promise<InsightAvailability>` forwards Tauri command `automatic_insights_availability` to core `meeting_automatic_insight_availability_json`. Existing availability fields include `resolved_language` and the real configured destination/model. Reading readiness neither admits work nor accesses credentials or contacts a provider. IPC failures reuse the existing English/Dutch `insights.operation_failed` copy.

## Files

- New `apps/desktop/src-tauri/src/automatic_suggestions.rs`: helper, core adapter, safe metadata DTOs and regression tests.
- `apps/desktop/src-tauri/src/main.rs`: module and read-only command registration.
- `apps/desktop/src/api.ts`: thin availability getter using the existing type.
- `apps/desktop/src-tauri/src/summaries.rs`: parent-authorized one-line `too_many_arguments` allowance with rationale, preserving the existing Tauri command's approved named wire fields and injected app/state. No wire or behavior change.

No edits to `meeting_runtime.rs`, live workspace, UI, core domain logic, Settings, mobile or dependency manifests.

## Verification

- Test-first red compile established the missing helper/flow.
- Initial six new runtime tests passed. The added foreign-begin ownership regression then failed at its expected assertion, and passed after removing cancellation of an unowned returned ID.
- Final native/backend suite, using fresh checkout inputs: **370 passed, 0 failed**, including **9 new tests** (6.89 s execution).
- New cases verify admission-before-keys, actual core fresh/manual/unqualified non-dispatch and non-networking, safe metadata publication, cancellation/physical drain, superseded response ownership, unowned begin response, shutdown after admission, stable UID after rename, and no sync poke or retry from transient progress.
- Real-core privacy tests use the unchanged bundled empty qualification registry and a nonblocking local TCP listener. They assert an actually configured but unqualified finalized transcript sends no connection and reads no host keys. Controlled job-response adapters exercise positive host lifecycle behavior only; they do not claim a qualified provider run.
- `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --bin pennel --tests -- -D warnings`: **passed**, without global lint exceptions (26.44 s).
- `npm run build`: **passed** TypeScript and Vite.
- Rust formatting and `git diff --check`: **passed**.

The first broad compile detected a stale shared-target core artifact from another worktree, not missing source APIs. Refreshing this checkout's local input mtime under the cache lease corrected it. The parent then made the cache wrapper origin-aware; final logs explicitly show this worktree's core/backend paths. A later fixture assertion initially reached `no_transcript`; adding one canonical saved transcript ensured it actually tested `unqualified` readiness.

Logs: `t6b-runtime-backend-test.log`, `t6b-runtime-clippy.log`. The temporary baseline-allowed clippy diagnosis is superseded by the final strict pass. No repeated core full suite, mobile/native-generation, Swift or non-Mac checks were run; those sources/settings did not change.

## Remaining integration/acceptance gates

The postprocess call and its ordering test belong to the meeting owner's T6a/T6b integration. Review UI/consent/Settings work is not claimed by this helper commit. Production qualification remains closed until an exact reviewed record is adopted, so no live qualified automatic provider or signed device acceptance is claimed. Parent owns independent milestone review, tracker/PR integration, merge and release. No pushes, tracker changes, merges, releases, subagents, dependencies or content logging were performed.


## P2 receipt-finalization correction

Correction head: `c0e5dcb701120f201792c6ee886b6546fb1eb1e0`, based on reviewed helper `25f4054f82208a4e58ae77823ddcf4cec91b5cd9`.

The independent review correctly identified that logical terminal state and `physical_running = false` can precede the automatic-attempt receipt write in `AdmittedInsightAnalysis::drop`. The original fake backend finalized its receipt too early and hid this ordering.

Core now exposes additive ephemeral job metadata `finalization_finished`, initially false. Drop sets it once, with a revision increment and notification, only after receipt handling and inference-registration cleanup. This signals that the attempt to write the receipt has finished; it does not claim that an unsuccessful write persisted an outcome. Job busy accounting remains active through that boundary. Immediate logical cancellation and the actual physical-request count retain their existing semantics.

The native helper requires terminal state, no physical request and `finalization_finished = true` before its final durable refresh and final progress event. It does not wait for a marker outcome and adds no guessed delay or automatic retry. An unwritable receipt therefore finishes monitoring while the initial attempt remains consumed. Core producer and native consumer belong in the same integration commit. No Settings, dependency, UI, persisted schema or qualification change is included.

Regression evidence:

- Backend red: both new regressions observed zero waits before the receipt latch where one was required; the old helper returned early.
- Core red: the controlled terminal/physically idle snapshots had no finalization barrier. Test latches were released before assertions so no failed test abandoned an owned worker.
- Core green: a real qualified test transport and actual storage gate hold receipt handling after terminal publication/provider drain, then prove the final receipt and revision appear after release. Covers completed, failed and cancelled outcomes. An unwritable marker-directory fault proves finalization ends and re-admission stays refused without a retry.
- Runtime green: independent late-receipt and failed-write responses verify final durable notification after the latch, and termination even when the pending outcome remains unwritten.
- `cargo test --manifest-path core/Cargo.toml --lib --no-default-features meeting_insight_engine::tests`: **46 passed, 0 failed** (6.95 s).
- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml automatic_suggestions::tests`: **11 passed, 0 failed** (0.17 s).
- Strict default core clippy and strict backend binary/tests clippy with `-D warnings`: **passed** (11.55 s / 29.89 s).
- Core/native formatting and `git diff --check`: **passed**.

Correction diff: `t6b-runtime-receipt-fix.diff`. Logs: `t6b-runtime-receipt-core-tests.log`, `t6b-runtime-receipt-core-clippy.log`, `t6b-runtime-receipt-backend-tests.log`, `t6b-runtime-receipt-backend-clippy.log`. Source changes are limited to `core/src/meeting_insight_engine.rs` and `apps/desktop/src-tauri/src/automatic_suggestions.rs`. The parent still owns correction-scope review, caller integration and the final integrated full run. No whole-core, unrelated backend/UI, mobile or non-Mac suites were repeated for this private lifecycle-field change.


## Test-only returned-snapshot correction

Head: `25d5ca61dfed625deead8d6ad96e69fce920e120`, based on `c0e5dcb701120f201792c6ee886b6546fb1eb1e0`.

The correction review accepted the production barrier and identified a separate test waiter race: an older JSON snapshot could be followed by a newer live `!job.busy()` observation. `wait_job` now also requires its captured/returned `state["finalization_finished"] == true`. The existing eight-second deadline, physical-running check and live retention check are unchanged. Exactly one test-helper code line was added; no production code changed.

Covering check: `cargo test --manifest-path core/Cargo.toml --lib --no-default-features terminal_receipt` using the origin-aware core cache lease: **2 passed, 0 failed** (0.84 s), covering completed/failed/cancelled receipt ordering and unwritable-receipt retirement. Formatting and diff checks passed. No broad core/backend/UI/mobile suites were repeated.

Receipt log: `t6b-runtime-waiter-tests.log`. Exact one-line correction diff: `t6b-runtime-waiter-fix.diff`. Parent owns correction closure and final integrated validation.
