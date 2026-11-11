# Desktop redesign implementation — 11 November 2026

Owner: [desktop redesign #220](https://github.com/pennel-app/pennel/issues/220). This receipt covers the complete T0–T12 software implementation and its review handoff. The integration branch is `codex/redesign-integrated`; the source candidate is [`d3f00e70419d2d8b70e1bf1c1c02d3f08932e8c1`](https://github.com/pennel-app/pennel/commit/d3f00e70419d2d8b70e1bf1c1c02d3f08932e8c1). Later receipt-only commits do not change the tested application. No PR or main branch was merged, and no release was published.

The owner approved the recommendations in the structured-summary, automatic-suggestion and shortcut ADRs, including canonical Markdown with revision-owned structure, independent opt-in policies, qualification gates and compatibility fallbacks. “Not now” is durable per meeting without changing an Unanswered global policy. Home practice uses the saved Result setting and existing History retention; onboarding practice remains separate. A speech-model switch applies to the current capture and its final buffered pass. Page and onboarding titles use 26 px, dictation instructions 30 px, setup headings 20 px, and the pill retains Figtree.

## Implemented scope and review branches

The shared TypeScript/Tauri application and Rust core implement the following. Each feature branch has its own review and correction receipts; the combined source received a separate frontend/spec and native/core integration review. These are prepared review branches, not a sequence of already merged changes. Preserve their declared bases when reviewing or later integrating the stack.

| Plan tasks | Result | Review handoff |
| --- | --- | --- |
| T0 | Handoff, task issues and roadmap tracking | [#220](https://github.com/pennel-app/pennel/issues/220) |
| T1a | Live and speaker roles, motion tokens, chip/keycap helpers, icons and reference fixtures | [#221](https://github.com/pennel-app/pennel/pull/221) |
| T1b / T1c | Actual hover/pressed/focus feedback, both reduced-motion routes and chosen title sizes | [#227](https://github.com/pennel-app/pennel/pull/227), [#225](https://github.com/pennel-app/pennel/pull/225) |
| T2 / T7 / T8 | Owned Dictation home practice, recent results, task Settings and first-run polish | [#229](https://github.com/pennel-app/pennel/pull/229), [#230](https://github.com/pennel-app/pennel/pull/230), [#233](https://github.com/pennel-app/pennel/pull/233) |
| T3a / T3b / T3c | Recording capsule/levels, immediate start, finished meeting document and responsive companion | [#228](https://github.com/pennel-app/pennel/pull/228), [#231](https://github.com/pennel-app/pennel/pull/231), [#236](https://github.com/pennel-app/pennel/pull/236) |
| T4 | CodeMirror inline Markdown editor with source offsets, composition, mentions, autosave and scroll ownership | [#243](https://github.com/pennel-app/pennel/pull/243) |
| T5a / T5b | Frozen Share dialog/personal notes and durable Summary consent/progress | [#234](https://github.com/pennel-app/pennel/pull/234), [#239](https://github.com/pennel-app/pennel/pull/239) |
| T6a / T6b | Review, Undo and accepted-item Copy; independent suggestion policy, postprocessing and consent | [#240](https://github.com/pennel-app/pennel/pull/240), [#232](https://github.com/pennel-app/pennel/pull/232), [#246](https://github.com/pennel-app/pennel/pull/246) |
| T9a / T9b / T9c / T9d | Accepted summary contract, revision sidecars/history, provenance, speaker timeline and structured reader | [#222](https://github.com/pennel-app/pennel/pull/222), [#235](https://github.com/pennel-app/pennel/pull/235), [#238](https://github.com/pennel-app/pennel/pull/238) |
| T10a / T10b / T10c / T10d | Accepted shortcut contract, core validation/gestures, shared commands and Mac capture/withholding | [#224](https://github.com/pennel-app/pennel/pull/224), [#237](https://github.com/pennel-app/pennel/pull/237) |
| T10e / T10f / T10g | Portable adapters and truthful capabilities, recorder, dynamic labels and Shortcuts Settings | [#241](https://github.com/pennel-app/pennel/pull/241), [#242](https://github.com/pennel-app/pennel/pull/242) |
| T11a / T11b / T11c | Structured notices/delivery ownership, current-capture model switch, pill preferences and guarded preview | [#226](https://github.com/pennel-app/pennel/pull/226), [#244](https://github.com/pennel-app/pennel/pull/244), [#245](https://github.com/pennel-app/pennel/pull/245) |
| T12 | Final timing/reduced-motion pass, safe companion leave, Share hierarchy and combined verification | Final integration draft linked from [#219](https://github.com/pennel-app/pennel/issues/219) |

The main changed boundaries are `core/src/{status,shortcuts,live_dictation,dictation_models,meeting_summary,summary_structure,meeting_timeline,meeting_suggestion_automation,meeting_insight_engine}.rs`; desktop dictation/pill/owned-session/Settings/shortcut adapters; the home, live workspace, reader, consent, Review, Share and editor components; Swift hotkey/shortcut services; shared catalogs and generated tokens. `DESIGN.md`, `GLOSSARY.md` and platform documentation record the resulting contracts.

Only the three approved direct frontend dependencies were added: `@codemirror/state` 6.7.6, `@codemirror/view` 6.43.15 and `@codemirror/commands` 6.11.1. Their normal infrastructure dependencies and full MIT notices are included. The editor replaces the textarea/preview interaction rather than maintaining a second editing engine. Core retains automation/policy ownership; desktop presents state and native effects. Existing menus, token roles, localization and provider/cancellation boundaries are reused.

## Automated verification

The complete Core, Mac service and mobile checks ran against the integrated source with the fingerprints below. The last two corrections affected only the desktop Settings boundary and localization: the full backend/static checks were repeated after both, as were frontend build/tests and the affected Core catalog tests. The final catalog change adds two translations without changing Core Rust or mobile/native service source.

| Check | Command | Observed result |
| --- | --- | --- |
| Core release library | `make core` | Passed |
| Full Core and keyboard | `make test` | 1,295 Core passed, seven intentional ignores; 17 keyboard passed |
| Core static analysis | `cargo fmt -- --check`; `cargo clippy --lib --tests -- -D warnings` in `core` | Passed |
| Desktop frontend | Exact lock install; `npm run build`; `npm test` in `apps/desktop` | TypeScript/Vite passed; 524 tests passed, repeated after final catalog correction |
| Final desktop backend | `cargo test` in `apps/desktop/src-tauri` | 407 passed, zero failed/ignored, repeated after final catalog correction |
| Backend static analysis | `cargo fmt -- --check`; `cargo clippy --bin pennel --tests -- -D warnings` | Passed after both final corrections |
| Independent-policy regression | `cargo test --bin pennel commands::settings_inference_tests -- --test-threads=1` | Four expected failures before fix; six passes after fix |
| Final Core catalogs | `cargo test localization::tests` in `core` | 11 passed |
| Mac services | `swift test --package-path apps/macos-platform` | 217 passed |
| Mobile compatibility | Exact lock install; `npm run typecheck`, `test:plugins`, `test:widgets`, `test:ui` in `apps/mobile` | Typecheck passed; 17 plugin, seven widget and 160 UI tests passed |
| Theme | `python3 scripts/generate-theme.py --check`; `--self-test` | Fresh outputs and 23 self-tests passed |
| CI selector and language guard | `python3.11 -B -m unittest discover -s scripts/tests -p 'test_ci_*.py'`; corresponding `test_language_policy.py` discovery | 162 selector and two policy tests passed |
| Source/static receipts | Full-range `git diff --check`; native workspace Python and benchmark JS syntax checks | Passed; syntax checking is not native runtime/performance evidence |

The local selector attempt under macOS's Python 3.9.6 failed because `tarfile.extractall(filter=...)` is unavailable there. It passed with the existing supported Python 3.11. The same 162 tests already passed on the hosted selectors; their actual shared failure was the two Dutch runtime literals, now routed through the existing catalog without a policy exception. Both English/Dutch forms and the configurable key label remain unchanged.

The frontend build retains Vite's existing chunk-size advisory above 500 kB. The limit was not raised and the warning was not presented as a build failure. The first mobile lock install encountered a full shared volume; after removing only disposable generated incremental compiler files under their cache lease, installation and all listed mobile checks passed. No source, saved work, stash or final evidence was deleted.

Source fingerprints at `d3f00e7`:

```text
core                         85e7d5a9fed47d388e12175432671fc345ab35df
assets                       9ee3c89cc4cc8b865a8d0d20897697e944c9a059
apps/desktop/src             69211f6b85d6669a4613dd2a2716e083da7a6017
apps/desktop/src-tauri/src   e5243ff345c5773c98b8095ceb11408f97667748
apps/macos-platform          efa12d5a817e643842ace810c85588be0153f62b
apps/mobile                  533b1388421a1e32fceb37e7511db747c779f122
```

The retained execution package is the main checkout's ignored `.omx/plans/full-redesign/`: accepted ADRs, per-task/source reviews, final integration reports, final-fix red/green/full-backend logs and publication manifests. Final combined command logs use `/tmp/pennel-final-*.log`; the last catalog checks use `/tmp/pennel-final-locale-*.log`. Cargo commands used the origin-aware, whole-command cache lease with three jobs and incremental compilation disabled, so another worktree's cached artifact did not substitute for this source.

## Browser, visual and interaction evidence

All new captures use Pennel's deterministic synthetic loopback fixtures, the actual production frontend and an owned headless Chrome profile. No user screen/profile, real microphone or user provider was captured. Fixture controls inserted only for feedback verification are labeled probes in their review evidence. Images remain on `pr-assets/pr-<number>/`, outside code history, and the corresponding PR bodies embed their matched comparisons.

- Each feature handoff contains its affected English/Dutch, light/dark and size evidence. The corrected T1c set has 64 distinct before/after pairs; its original stale baseline capture was discarded as evidence.
- The final combined browser pass contains 48 successful frames: 12 actual editor/source-return, consent ownership/race, shortcut, back-to-back pill and preference/privacy flows in English/Dutch and light/dark. Its manifest is `build/final-integrated-browser/manifest.json`; it reports complete with no runtime errors, unknown fixture commands or horizontal overflow. Fixture SHA-256: `3be2933b7d4fa7415ebdad10cfbae2fbeb364e5aa549c40a9cf68464dcb410c5`.
- The T12 matrix passed 30 actual timing/keyframe/fade cases, including negative controls and both system/app reduced-motion routes, plus ten affected focus/resize/source/interaction cases. The 32 matched before/after pairs cover English/Dutch, light/dark, minimum and enlarged Share layouts and the pill's fixed geometry. Static identical pairs outside changed paint are explicitly retained; PNGs alone do not prove animation.
- A real deep-scroll failure showed the companion's outgoing visual copy resetting from 7,266 px to zero. All seven correction cases passed after preserving public offsets, including 7,266 px to 7,266 px with the same visible passage under normal and both reduced-motion routes. Reopen preserves the real reader, draft, selection and focus.
- The final pill integration passed 36 joint cases and a stricter 12-case privacy pass. Settings performs exactly one initial shortcut-recorder state read and no later activity snapshots; the standalone demo makes no activity snapshot or capture/text/clipboard mutation. This does not establish native nonactivation.

T12's bounded visual verdict is 94/pass against the approved Share and motion references. The independent source reviews passed after correction: the broad frontend/spec review, native/core review, native demo race review, outgoing-scroll correction review and final catalog correction review have no remaining actionable finding in their assigned scope.

The final native review found and corrected one integration defect: a desktop Settings save cancelled unrelated inference before Core could apply independent policy epochs. [`b10c122`](https://github.com/pennel-app/pennel/commit/b10c122eba41e1b083304a43c85f48b172910ecd) bypasses global invalidation only for actual edits confined to Summary policy, Suggestions policy or Suggestions fallback language. Core still revokes work owned by its changed policy; provider/model/configuration, mixed, other and no-op saves retain the previous broad guard. Real registered-token and admitted Summary regressions cover this boundary. No qualified Suggestion was fabricated for the tests.

## Product clarifications and acceptance still open

“Match Pennel” / “Volg Pennel” follows the saved global Appearance. It follows the OS when the app uses System; forced Light/Dark wins. The serialized `system` preference remains compatible. Tauri/Tao's theme setter has application-wide effects, so the pill does not impose an unsafe independent theme policy on other windows. Ink and Light remain fixed choices.

Keyed global shortcuts on X11 fail closed because the available grab backend cannot establish a safe release boundary. Modifier-only dictation/listening actions, in-app shortcuts, buttons, tray and CLI routes remain; unsupported rows keep localized explanations. Windows and X11 use active-layout VK/keysym identity, while macOS uses physical codes, as the accepted ADR specifies. Shortcut settings remain per machine. Shared source and capability behavior were reviewed; **Windows and Linux product builds were deliberately skipped at the owner's instruction**. Routine CI remains macOS-first.

Production suggestion qualification remains closed: the current registry has no qualified production model. Policy, consent, scheduling, cancellation and UI paths are implemented and tested without bypassing that gate or claiming successful qualified production inference. Manual fallbacks and setup/reason states remain visible.

The pill keeps its nonactivating, click-through contract and the keyboard model-switch route. Its click/menu route remains behind the original actual-Mac acceptance decision. Real external paste/clipboard and focus, global key withholding, VoiceOver/IME, microphone/provider quality, meeting meters on hardware, multiple displays/cursor placement and native compositor/menu behavior require device evidence. Browser fixtures, synthetic transports and successful builds do not satisfy those gates. Signed iPhone upgrades, keyboard cold start, Cloud deployment, store and release acceptance also remain separate.

Hosted CI is recorded per PR rather than inferred from local tests. The shared language-policy source failure was fixed and propagated to its affected heads. PR #239's first Mac run built/signed the application but failed DMG creation with `hdiutil: create failed - Resource busy`; a failed-job retry was requested on the same commit. New or retried hosted jobs must complete before those PRs can be merged. The coordinator's final handoff records the latest observed check status. Prepared issues remain open/In review; no review result closes a merge or device-acceptance criterion.
