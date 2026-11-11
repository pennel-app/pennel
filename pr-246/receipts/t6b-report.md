# T6b — suggestion consent, states and post-summary caller

Branch `codex/redesign-suggestion-consent`, combined reviewed foundation `a463cdabec735510c0f60e9d66d79a9e91449d42` (`codex/redesign-editor-foundations`). Final SHA appended below. T5b/T6a source refs remain frozen. Parent owns independent review, PR/assets/tracker and integration with T4.

## Foundation and commit boundary

The reviewed helper producer/correction/test trio is already in a463. Initially integrated locally as c424caa/639984e/39038c1, then **not replayed** when parent supplied a463. T6b-only checkpoint2a63605 was rebased as `af2027731a621b9282b091aa2975c9b6f58f7d08`. Its one conflict was the metadata writer: retained `.then(validated)` before acknowledgement, all `invalidateReads()` guards and the combined T9d `deferredPresentation` clearing.

Parent's inherited duplicate Cancel test correction `da4cbae1b196b8fa03d65a4a80bc83225f3ade5c` was imported as `1dc1ae7`. The context conflict was resolved by deleting exactly the second byte-identical22-line test block in pill.rs. No unrelated later pill test or production behavior was imported. Root already owns that correction, so skip duplicate application during integration. The final feature commit follows those two checkpoints.

## Delivered behavior

- Suggestion consent remains independent of Summary consent. A qualified finished-meeting empty state names the real endpoint/model and transcript-only inputs, with an initially unchecked checkbox, Find and Not now. Checked Find acknowledges Automatic; unchecked Find acknowledges Manual; only checked consent persists an explicitly requested automatic-language fallback. The current request uses the existing guarded manual analysis path after that acknowledgement.
- Not now uses its own durable `pennel_suggestion_prompt_dismissed` meeting metadata field. It leaves both global policies alone, preserves the saved summary body/record/source/edited state and survives rename/reopen/profile isolation. The quiet Find action runs once without saving policy. No product localStorage-only policy is used; the existing fixture profile only emulates durable files for synthetic reload QA.
- The core reuses the existing metadata CAS helper with a trusted flag-name parameter. Public summary APIs remain unchanged; separate suggestion APIs use the same meeting edit, revision and source barriers. Summary owns acknowledgement of this meeting.md metadata write, preserving current typing, candidate base, record validation and deferred reader presentation.
- A read-only optional fallback preview uses the same frozen-language, exact qualification and foreground-impact gate. It never writes settings/markers, reads credentials or contacts a provider. Frozen en/nl wins; unsupported explicit fr cannot be relabelled by a fallback. The UI first reads actual frozen/saved resolution and previews only an unresolved local choice. A newly saved fallback reaches an already open meeting without granting consent or starting work.
- Unknown/unqualified setup has no consent card; reason, language and Connection settings remain reachable. Review keeps existing cards/claims/edits/evidence/insertion/export. Empty, consumed, failed, interrupted, stale and damaged-attempt states remain meaningful. Missing credentials and other known receipt failures have safe coded explanations and reachable repair. There is no automatic retry.
- Settings places Summary and Suggestions under After a meeting, with separate Automatic/Manual controls and the explicit fallback. It displays local configuration and a conditional qualification explanation. It does **not** call the generic inference capability probe, which can contact Pennel Cloud; exact per-meeting readiness remains in the read-only Review/core path. The saved policy uses the existing selection token.
- The reviewed runtime helper is called exactly once after the summary block returns, including error/skip. It retains admission-before-credentials, owned job cancellation/draining and final receipt barrier. A metadata-only `postprocess` span begins before speaker processing and ends after the helper/early return, so UI preparation is distinguishable from not-run. Existing summary/suggestions stage contracts are unchanged; all stages carry only id, UID, operation, stage and state. Competing Find is disabled while preparation/native work runs; Cancel appears only for an actual job.
- Consent reads have generation ownership before choices and acknowledgements. A late value or error cannot undo Not now. Read failures retain a reachable Retry even after an earlier successful read, and cached status refreshes on the existing presentation cadence to observe external durable dismissal.

## Tests first and bounded regressions

- Core three new regressions: separate durable dismissal, unchanged content/record/policies, stale document/source rejection, read-only language preview and frozen-language guards. `/tmp/t6b-core-red.log` records absent API compile errors; `/tmp/t6b-core-green.log` passes3/3. Existing summary CAS tests cover the reused helper as well.
- Pure presentation/choice/progress tests: policy/availability/attempt states, consent ordering, failed save prevents start, one-off and Not now, fallback persisted only with checked consent, and independent whole-postprocess span. Red `/tmp/t6b-ui-state-red.log`, `/tmp/t6b-progress-choice-red.log`; green8/8 `/tmp/t6b-ui-state-green.log`.
- Two fixture preview/durability regressions: `/tmp/t6b-fixture-{red,green}.log`; final run below includes both.
- Production browser readiness found Not now retaining its initial Summary-loading state after the Summary was ready. Forwarded Summary readiness into retained Review; dismissal/reload then passes. No note/source facade was changed.
- Metadata progress initially allowed competing Find before a job snapshot existed. `build/t6b/progress-gate-red` fails; `read-progress-green` passes with real metadata state, no fake Cancel, no inference.
- Settings initially used a network-capable generic readiness probe. `build/t6b/settings-read-red` fails; `read-progress-green` verifies configuration-only rendering and explicit independent policy/fallback writes.
- Held stale false/error responses around Not now are tested through the real production component with a stable owner and actual fixture API writes, controlling acknowledgement-before-notification order. Temporarily disabling only the new write invalidations produces both expected failures (`build/t6b/late-{value,error}-red`); restored source passes `late-prompt-green` and `final-async`.
- Read failure after a changed language initially had no Retry; external dismissal was not periodically observed; a newly saved fallback left an open meeting's manual language unknown. Concrete reds are `read-retry-red`, `external-status-red`, `fallback-changed-red`; final production-owner and full-workspace checks pass in `final-async`.
- A durable missing-key receipt initially had a generic explanation/no repair. `missing-key-red` fails; `repair-final` verifies the coded reason, reachable Connection settings and no retry on render.
- Settings' saved policy initially lacked a selected background; `selected-state-red` fails, final light/dark checks pass after reusing Summary's selection token. The apparent enlarged clipping failure was a test selecting the entire state section instead of its Manual button; selectors now target actual buttons, with no layout weakening.

## Final verification

All commands ran in `/Users/lander.parren/.codex/worktrees/redesign-recording-capsule/pennel`.

| Check | Actual result / evidence |
| --- | --- |
| `npm --prefix apps/desktop run build` | Pass TypeScript +Vite; `/tmp/t6b-build-final.log`. Existing nonfatal >500kB shortcut-presentation chunk warning remains; no unrelated bundle redesign. |
| `npm --prefix apps/desktop test` | **502 passed**,0 failed/skipped; `/tmp/t6b-frontend-final.log`. |
| Wrapper core `cargo test --manifest-path core/Cargo.toml` | **1292 passed,7 ignored**,0 failed; bin/doc suites pass. `/tmp/t6b-core-final.log`. |
| Wrapper core `cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings` | Pass; `/tmp/t6b-core-clippy.log`. |
| `cargo fmt --manifest-path core/Cargo.toml -- --check` | Pass; `/tmp/t6b-core-fmt.log`. |
| Wrapper backend `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | **392 passed**,0 failed/ignored; `/tmp/t6b-backend-corrected-final.log`. |
| Wrapper backend `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --tests -- -D warnings` | Pass; `/tmp/t6b-backend-clippy-corrected-final.log`. |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check` | Pass; `/tmp/t6b-backend-fmt-corrected-final.log`. |
| `python3 scripts/generate-theme.py --check`; `git diff --check` | Pass; `/tmp/t6b-theme.log`, `/tmp/t6b-diff-check.log`. |

Wrapper is `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py`, invoked as `python3 <wrapper> core|backend cargo ...`. It supplied lease/origin refresh, SDKROOT, deployment14.4 and job limits. First backend compile/clippy was blocked solely by the inherited duplicate pill test; correction-only checks above are the final evidence. One accidentally started check during the cherry conflict was stopped as this agent's own process tree before the final fresh run. No cache deletion, unrelated process termination or broad native rerun. All leases are released.

## Focused owned-browser evidence

Executable `/Users/lander.parren/.t3/tools/chrome-headless-shell/mac-arm64/154.0.8037.92/chrome-headless-shell`. Synthetic fixture content/qualification only, owned temporary profiles, no user screen or real credentials/provider.

- `build/t6b/visual-green`: **16/16**, EN/NL ×light/dark for wide question, minimum closed default,200% minimum question/actions and200% Settings controls.
- `build/t6b/before`: **4** matched Review reference inputs using `--baseline-ref a463cda`; compare with reference files in visual-green. Case input hashes match.
- `build/t6b/settings-before` and `settings-after`: **2+2** matched EN/light+dark Settings inputs, baselinea463cda.
- `build/t6b/states`: **8/8** qualification/error/retry/no-result/failure/stale/damaged/progress cases.
- `build/t6b/actions-final`: **5/5** foreground-use refusal, explicit Manual/Automatic language choice, durable Not now/reload and quiet one-off Find.
- `build/t6b/preservation-green`: **13/13**, existing review/edit/dismiss/source-return/export, minimum, unqualified, read-only, insertion race, generation while editing, stale source, external protected edit, cancel, recording→finished, Summary Not now, T9d held-read and protected presentation. Helpers select the visible Analysis language control rather than the hidden consent copy.
- `build/t6b/final-async`: **7/7**, fresh saved fallback, read Retry, external durable status, actual unknown-language consent and both held-value/error acknowledgement guards.
- `build/t6b/repair-final`: **3/3**, missing-key repair, unqualified setup and saved fallback.
- `build/t6b/selected-state-light` and `selected-state-dark`: **1+1**, actual selected policy, including NL/dark minimum200%.
- Earlier targeted `late-prompt-green`2, `read-progress-green`2 and `status-final`7 retain the narrower red→green receipts. Failure/debug captures are diagnostics, not passing evidence.

Read-back of final manifests confirms complete, zero unknown commands, runtime errors, horizontal overflow or surface errors. Visual verdict **94/pass**, threshold90, `.omx/state/suggestion-consent/ralph-progress.json`. No PNG enters code history. Parent publishes assets.

## Simplifications and integration notes

Reused core CAS/summary generation metadata, exact qualification, existing job model/service, retained cards, native menus, tokens and current read generations. No dependency, duplicate policy epoch, provider fallback or new content authority. Global Settings no longer needs a status request at all.

The T4 workspace merge is small: `insightHost` adds suggestion metadata dismiss/canDismiss/reason callbacks, and Summary's `changed` callback forwards `insights?.readiness()`. Keep T4's actual contentDOM/composition/source/insert facade changes around these callbacks. Summary `dismissPrompt(kind)` retains `.then(validated)`, invalidates reads before write/ack and acknowledges metadata through the combined T9d state that clears deferred presentation. No Notes editor internals were changed here.

## Limits

The bundled qualification registry still has no adopted production record, so production analysis remains unavailable until exact evidence is adopted. Synthetic flags prove UI state/ownership, not model quality. No actual provider/credential, physical capture, signed package, VoiceOver, iPhone/device/mobile runtime, Windows/Linux or release acceptance is claimed. No Swift/capture source, generated native project, dependency or live-suggestion policy changed. Shared core API additions are additive; desktop wiring only, iPhone retains its existing manual flow. Parent owns remaining integration/review/release gates.

## Changed files from a463 (including the parent-authorized inherited test correction)

- `DESIGN.md`
- `GLOSSARY.md`
- `apps/desktop/src-tauri/src/automatic_suggestions.rs`
- `apps/desktop/src-tauri/src/main.rs`
- `apps/desktop/src-tauri/src/meeting_postprocess.rs`
- `apps/desktop/src-tauri/src/meeting_runtime.rs`
- `apps/desktop/src-tauri/src/pill.rs`
- `apps/desktop/src-tauri/src/summaries.rs`
- `apps/desktop/src/api.ts`
- `apps/desktop/src/live-workspace.ts`
- `apps/desktop/src/meeting-postprocess-state.ts`
- `apps/desktop/src/meeting-suggestion-consent.ts`
- `apps/desktop/src/meeting-summary-editor.ts`
- `apps/desktop/src/meeting-workspace.css`
- `apps/desktop/src/reviewed-insights.ts`
- `apps/desktop/src/suggestion-consent-state.ts`
- `apps/desktop/src/views/settings.ts`
- `apps/desktop/tests/insights.interactions.mjs`
- `apps/desktop/tests/meeting-postprocess-state.test.mjs`
- `apps/desktop/tests/suggestion-consent-state.test.mjs`
- `assets/locales/meetings.en.json`
- `assets/locales/meetings.nl.json`
- `core/src/meeting_insight_engine.rs`
- `core/src/meeting_summary.rs`
- `scripts/fixtures/desktop-platform.js`
- `scripts/fixtures/desktop-platform.test.mjs`
- `scripts/fixtures/visual-cases.json`
- `apps/desktop/src/meeting-suggestion-settings.ts`
- `apps/desktop/tests/suggestion-consent.interactions.mjs`

## Frozen source

Final head **`6ca385426413c2cda08122dc90baa013718eac8e`**; working tree clean after commit. On a root already containing a463 and its own duplicate-test correction, integrate **af2027731a621b9282b091aa2975c9b6f58f7d08 +6ca385426413c2cda08122dc90baa013718eac8e**. The first is the T6b core/native/API and initial UI checkpoint; the latter completes UI/fixtures/docs. Skip1dc1ae7 when root already has da4cbae. Do not replay the helper trio. Independent review is parent-owned and pending at this handoff.
