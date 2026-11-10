# T11b — configured speech models and current-capture switching

Status: core prerequisite implemented; desktop integration waits for the parent's T10e dependency integration. This is not the T11b completion handoff.

Exclusive source: `agent-a65210de8afb33f96`, `codex/redesign-pill-models`, base `d65fe624`. Core prerequisite commit `1d0e9e6`.

Core contract:

- `Settings.dictation_models` is a serde-defaulted explicit list. Missing list preserves only the existing saved `dictation_stt`. Selecting a speech setup in the existing atomic task chooser remembers the previous and new setup without reassigning any other task.
- `configured_dictation_models(ApiKeys)` deduplicates exact provider/model pairs in saved order, appends the selected setup if absent, and filters missing/unsupported providers, missing credentials, undownloaded/unsupported local models and unavailable system recognition. It performs no provider requests, download or token refresh. Provider model IDs remain truthful fallback labels.
- `select_dictation_model` and `next_dictation_model` validate core readiness and persist the actual selection without provider fallback.
- `LiveDictationSession.select_speech_model` / `next_speech_model` preserve the capture ID, original language/dictionary/mode, target ownership and final audio buffer. The selected provider account is captured atomically with the choice, so later provider changes cannot retarget the final pass.
- Presentation restart cancels obsolete local work; generation-aware system callbacks reject obsolete updates after switching. Existing hosts using the older callback remain safe for the initial presentation and cannot publish stale text after a switch.

Verification so far:

- Red test compile observed missing model-list and session APIs before implementation (`/tmp/pennel-pill-core-red.log`).
- Configured model tests: 3/3 passed (`/tmp/pennel-pill-core-green.log`).
- Existing and new live session tests: 16/16 passed (`/tmp/pennel-pill-session-green.log`).
- Strong system-to-batch switch regression: exactly one final request contains the new model ID; original system transcriber receives zero calls; capture ID is unchanged (`/tmp/pennel-pill-switch.log`).
- Desktop TypeScript/Vite build passed (`/tmp/pennel-pill-frontend-build.log`).
- Mobile mirrored optional settings field: `npm ci --ignore-scripts`, `npm run typecheck`, `npm run test:ui` passed, 160/160 (`/tmp/pennel-pill-mobile*.log`). Existing lock audit reports remain; no dependencies changed. No native/mobile UI/generation changes.
- Core strict clippy passed (`/tmp/pennel-pill-core-clippy.log`).
- First full default core run: 1213 passed, 2 failed, 7 ignored. The two failures were the existing equality tests for applying/concurrently applying a speech setup, whose expected settings did not include the newly remembered choices. Expectations corrected to assert the complete settings including the explicit list; final full rerun passed: 1215 passed, 0 failed, 7 ignored (`/tmp/pennel-pill-core-full-final.log`).

Cargo commands use the supplied `with-cache-lock.py core` wrapper. No non-Mac builds or CI waiting. Native pill remains click-through/nonactivating; click acceptance has not been attempted or claimed.

## Independent core review corrections — 8b1cd69

Both reported P2 regressions were reproduced before correction and fixed together in a core-only commit. The active backend/UI/toggle working tree is outside this source pin.

1. `engine::dictation_models::tests::switching_materializes_the_current_choice_omitted_from_a_partial_saved_list`: start from valid `dictation_models=[B]`, `dictation_stt=A`. Both appear ready before switching. Old direct selection of B loses A (observed red: expected two models, found one). The regression also exercises repeated next-model cycling from the original partial list. Both direct selection and next-model save now materialize the prior current choice before replacing it.
2. `live_dictation::tests::model_switch_clear_observer_can_read_the_updated_session_without_deadlock`: a real observer attached to an active session receives a nonempty caption, then reads `mode()` and `speech_model()` during the switch's empty-caption notification. Old code times out after one second because the getter mutex remains held. The corrected switch installs the new presentation, releases the getter lock, publishes the clear while new previews remain paused, then resumes preview acceptance. The observer reads the actual updated model and mode. The finishing gate and generation still serialize capture completion.

Exact commands (run from the exclusive worktree with the cache wrapper):

- `python3 /Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py core cargo test --manifest-path core/Cargo.toml --lib --no-default-features switching_materializes` — red, one failed assertion; `/tmp/pennel-pill-list-red.log`.
- Same prefix through `--no-default-features`, then `model_switch_clear_observer` — red, one bounded timeout; `/tmp/pennel-pill-observer-red.log`.
- Same prefix through `--no-default-features`, then `dictation` — green, **42 passed, 0 failed, 1 ignored**, including both regressions, configured-model validation, current-capture final-provider POST and obsolete generation rejection; `/tmp/pennel-pill-core-review-fixes.log`.
- `cargo fmt --manifest-path core/Cargo.toml` and staged `git diff --cached --check` passed; commit hook reported no staged secrets.

No repeated full core run was added for this bounded review wave. Earlier full-default 1215/0/7 and strict-clippy evidence remains recorded above; later full T11b integration has its own verification scope.

## Desktop model presentation milestone — 1afb3ba

Source is committed at `1afb3ba`, based on the corrected core plus cherry-picked T10a–d and native callback/focus correction `6543037` (local integration `06fdd57`). This is ready for bounded backend/UI review. **T11b remains open for the T10e native readiness setter integration; no working keyboard registration or actual native focus acceptance is claimed yet.**

The backend accepts explicit Settings selection through `Msg::SelectSpeechModel` on the actual dictation controller. Its current `recording_session` is the common session used by global, Pad and home-practice captures, so selection preserves the Recorder, OwnedCapture, external destination and preview cursor. It restarts Apple preview with a captured generation; core local preview uses the same generation guard. The ready configured-model list is real core output. Model-only metadata has no recognition text, is bound to the capture/generation, clears with the capture, and boot snapshots do not replay confirmations. The real serde-defaulted `pill_model_switch_enabled` preference gates the chip; readiness registration will gate the key after T10e. The native Settings menu chooses an actual ready setup or opens the retained setup route. Provider setup, keys and downloads remain explicit.

The keyboard-only chip shows the actual model and local/provider icon. The 900 ms acknowledgement belongs to its capture and generation: duplicates do not extend it, old events do not replace it, and Working/new capture/hide cancels its timer. No pill click handler, focus flag or cursor-event flag changed.

Evidence:

- Backend full `with-cache-lock.py backend cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`: **361 passed**, 0 failed; `/tmp/pennel-t11b-backend-full.log`. Model-metadata red compile preceded implementation; focused pill suite **12 passed** (`/tmp/pennel-pill-backend-red.log`, `/tmp/pennel-pill-backend-green.log`).
- New preference red compile preceded the field. `with-cache-lock.py core cargo test --manifest-path core/Cargo.toml --lib --no-default-features pill_model_switch_upgrade_defaults`: **1 passed**; `/tmp/pennel-t11b-toggle-green.log`.
- Desktop `npm run build` passed and `npm test`: **451 passed**; `/tmp/pennel-t11b-build-final.log`, `/tmp/pennel-t11b-tests-final.log`. New pure model-state regression suite has four tests. One old fixture assertion initially compared nested model metadata by reference; corrected it to compare cloned values, then full suite passed.
- Mobile mirrored optional preference: `npm run typecheck`, `npm run test:ui`: **160 passed**; `/tmp/pennel-t11b-mobile-type.log`, `/tmp/pennel-t11b-mobile-ui.log`. No dependency or generated native changes.
- Focused synthetic captures: `t11b-after-verified` has **16 images**, EN/NL × light/dark, zero failures, covering model ownership/timer lifecycle, model chip, actual Settings selection/toggle and 760 px/doubled-text settings. The row-label correction is verified in `t11b-settings-final` (**8 images**, zero failures). Baseline `06fdd57` captured from identical input with the harness baseline flag in `t11b-before-verified` (**8 images**, zero failures). These are mock renders of owned app pages, not native device evidence.
- Browser interaction checks verify configured menu selection persists, the actual toggle persists, no capture/clipboard action is triggered by model preferences, model confirmations last 900 ms despite duplicates, stale captures are rejected and Working is preserved. Initial interaction failure was a test querying listbox options while the menu correctly used menu-item roles; selector corrected, then passed.
- Visual verdict `.omx/state/t11b/ralph-progress.json`: **94/pass** after fixing the duplicated accessible switch label using the existing `visually-hidden` utility. No images entered code history.

Still pending for this milestone: T10e's stable `Listener::set_bindings_with_next_model(&Settings, bool)` and `record_shortcut::next_model_enabled()`, X11 refresh propagation, exact Next-model runtime reasons, focused hook/controller checks and final source freeze. T11c independent native position helper/tests are separate uncommitted files, not part of this milestone source pin.

## Native key and presenter finishing — 0c60b6d

Integrated stable T10e `f5ec040` as `14d679e`, then committed the isolated pill finishing source `0c60b6d`. It contains no T11c Colour/Size/Position changes. The controller now calls `set_bindings_with_next_model` with the real ready-model count and saved switch preference, refreshes X11 grabs afterward, and refreshes at capture start and settings/download/account events. Action4 requires the live native listening route; global, Pad, home-practice and onboarding-practice still share the same active controller session.

`cancel_shortcut` and model metadata's `next_shortcut` are optional formatted core labels supplied only when `record_shortcut::listening_action_ready` reports the actual hook/grab route ready. They clear outside listening. The UI never infers Esc or live Next availability from cached idle AppInfo. Concrete Next reasons are `next_model_switch_disabled`, `next_model_second_model_required`, `next_model_shortcut_cleared`, `next_model_hook_unavailable`, with EN/NL backend-source mappings. Valid cleared/switch-disabled bindings retain editability; the other UI lane consumes these reasons. The minor nested-if clippy warning in the prior observer regression was corrected without changing behavior.

Fresh checks: full backend **370 passed** (`/tmp/pennel-t11b-final-hook-backend.log`); frontend **452 passed** and build (`/tmp/pennel-t11b-hook-frontend.log`, `/tmp/pennel-t11b-hook-build.log`); strict core/backend clippy both passed (`/tmp/pennel-t11b-hook-core-clippy.log`, `/tmp/pennel-t11b-hook-backend-clippy.log`). Four EN/NL/light/dark executable live-hint mock captures passed in `t11b-live-hints` (`/tmp/pennel-t11b-live-hints.log`), including custom Ctrl+X Cancel and Ctrl+M Next labels and removal when native metadata becomes unavailable. The Shortcuts UI owner extracted the minimal identical Cancel seam while retaining its effective Done helper; parent integrates that overlap.

## Actual engine readiness audit — 5d569c8

Final T11b source pin is **`5d569c8`**, following `0c60b6d`. Two additional real readiness gaps were corrected inside the original configured/supported/ready requirement:

- A registered Apple adapter alone does not prove supported OS or granted Speech access. An additive volatile core host fact now gates system choices. The shared desktop reads the canonical Apple capability without prompting before listing/selecting. Existing callback hosts keep their previous default availability; no Settings schema, mobile trait or credential changes. Unavailable choices are filtered/rejected while the saved selection stays unchanged; recovery restores readiness with zero inference calls.
- A model file alone does not prove a compiled local recognizer. The list now reuses `PennelEngine.local_models` and the existing CPU-support gate. A real no-default-features test with a synthetic existing model file failed before this correction and passed after it.

The real new-provider final-pass regression now runs the global/Pad/home-practice/onboarding-practice matrix (the last uses the core practice flag), preserving each capture ID, exact history result ID and synthetic captured app ID, with exactly one final POST to the new model and zero original system-provider calls. This proves core provider/identity behavior; it does not claim actual external application insertion or microphone acceptance.

Red/green receipts: `/tmp/pennel-t11b-host-red.log` (missing host-fact API), `/tmp/pennel-t11b-local-feature-red.log` (existing local file falsely reported ready), `/tmp/pennel-t11b-readiness-final.log` (**44 passed, 1 ignored**), `/tmp/pennel-t11b-readiness-clippy-final.log` (strict core clippy passed). Fresh host-integration backend **370 passed** in `/tmp/pennel-t11b-host-backend.log`; host-fact changes preserve the default-feature local behavior. Final aggregate full core/build/static checks follow after restoring T11c so overlapping compilation is performed once for the final branch.

T11b implementation and its focused evidence are ready for the parent's bounded independent review at source pin `5d569c8`. Native click support remains deliberately gated; actual microphone/target/focus and signed acceptance are not claimed. T11c work is safely preserved in the exact owned stash `e62c7c19172093766e31cb1977f2f7a871d31a5f`, not an index-based stash reference, and resumes next. No unrelated stash or worktree was changed.

## Completed-feature review correction — 78caeaa

The parent's bounded completed-feature review found two P2 issues. Both are addressed in **`78caeaa` on `5d569c8`**, using an index-only source correction that excludes all preserved T11c appearance/geometry work.

1. Back-to-back captures with no Hidden transition retained the older speech model. A switched capture A at generation 2 could prevent capture B's generation-0 metadata. `begin_preview` now clears model metadata for a genuinely new capture; generation comparisons apply only inside the same model session, and old-capture payloads remain rejected. The combined backend regression covers A→Working→B, immediate clear, B generation zero, late A generation 99 rejection and repeated-B preservation. The matching browser regression found and fixes a related presentation seam: the frontend now immediately clears the old chip/timer when preview ownership changes, without waiting for B's model response or rewriting it on ordinary caption updates.
2. The model-switch row used missing-shortcut guidance for an existing unavailable binding. It now preserves the actual localized capability reason and uses missing-binding guidance only when the binding is actually absent. Readable source aliases cover Accessibility, unsupported/native keys, Wayland and the pending keyed-X11 restriction, reusing the existing Shortcuts copy where it fits and task-specific use/capture wording where needed. The tests use two configured models plus an existing binding, cover unavailable hook/Accessibility/native-key/Wayland reasons, missing reason fallback, and the real saved switch being off, in English and Dutch.

Observed red receipts: `/tmp/pennel-t11b-back-to-back-red.log` (retained old `speech_model`), `/tmp/pennel-t11b-back-to-back-browser-red.log` (old chip still visible after new preview), `/tmp/pennel-t11b-reasons-red-verified.log` (immutable `5d569c8` source displayed **“Set a next-model shortcut in Shortcuts.”** for the actual unavailable hook). Green receipts: `/tmp/pennel-t11b-review-state-green.log` (**16 focused pill tests passed** in the current working tree) and `/tmp/pennel-t11b-review-browser-final.log` (**8 focused EN/NL light/dark mock checks, zero failures**), assets under `t11b-review-final`.

The staged commit was constructed from the immutable pre-appearance source, and its diff contains only the two corrections, their focused regressions and localization. The working tree still holds T11c, including native frames and previews; no geometry source was reverted or included in this correction commit. No broad test catalogue was repeated for this bounded review wave. Parent should use `5d569c8..78caeaa` for its correction-only review.

Correction review artifact: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t11b-completed-feature-fix-review.diff` is the immutable `5d569c8..78caeaa` diff. Native X11 safety correction `ddb9dc2` is now integrated separately as `9f65729`, and its independent owned Cancel/lifetime test `d56237b` as `1ff83ae`; neither changes the correction-only review boundary.

## In-flight observer lock-order correction — 9e32011

Isolated one-file correction: **`9e3201117526364ed64eabdbaffcaa85618ac0f5`**. Review diff: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t11b-observer-drain-review.diff`.

The finishing/generation gates remain unchanged. Selection now reads the current choice briefly before persistence, then drains old observer emissions **without** holding recognition; it acquires the mutation guard only after that drain. This avoids the reverse lock order between an in-flight observer emission and its allowed `mode()` / `speech_model()` getters, while retaining the existing no-op choice behavior. No mutex, architecture, UI or provider-selection semantics were added.

The actual concurrent observer regression, `switching_drains_in_flight_observers_without_blocking_their_read_only_getters`, timed out against the old order (`/tmp/pennel-pill-observer-drain-red.log`). The observer is paused inside a real emission, selection reaches the persistence/drain boundary, then the callback resumes its public getters. After correction, **45 focused dictation tests passed, 1 ignored** (`/tmp/pennel-pill-observer-drain-green.log`), including that concurrent case, the earlier clear-callback reentry, current provider/final buffered pass across all owner modes, capture/history/target identity, old-generation rejection and model-readiness cases. Strict core clippy and release library build passed (`/tmp/pennel-pill-observer-drain-clippy.log`, `/tmp/pennel-pill-observer-drain-build.log`). No broad suite was repeated for this bounded correction; final root aggregate core verification includes the new regression.

T11c's final source is based on this additional dependency. Its restored appearance work remains outside the one-file correction.
