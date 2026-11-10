# T10g — complete Shortcuts Settings category

Status: T10f/g source implementation, native T10e integration, live Cancel presenter seam and local verification are complete at `f948dd5742d6da09a24e601be02548b12e520bd6`, ready for parent independent review. No PR, merge, publication or native/device acceptance claim is made here.

Worktree: `/Users/lander.parren/Developer/pennel-app/pennel/.claude/worktrees/agent-aa92d718182538c4d`, branch `codex/redesign-shortcuts-page`.

Source sequence:

- Original full integration base: `3624fcec7bf0117b6ef6d17ef704dae640f2abad`.
- T10f source checkpoint: `a364042dc502174e9a4409fa57498817777ce412`.
- Reviewed native T10e source: `f5ec040ecd142da584ec3a22b726394ccf4c2f59`, integrated locally as `ac5698e` without losing the generic settings writer's serialization or active-input reservation.
- T10g page milestone: `c4c5ee608e5610a6a4b5f8da4f05afaa90dfc131`.
- Final track head, minimal live Cancel seam: `f948dd5742d6da09a24e601be02548b12e520bd6`.

Implemented six Settings categories with Shortcuts as a real frontend/native deep-link target. The page has the two approved groups, all thirteen product rows, the derived locked Shift-with-Dictate Command binding, fixed Close/Quit rows, click-to-record, clear, per-row reset and Reset all. Inline conflicts preserve saved bindings until Cancel or atomic Use it here. Reserved/rejected combinations explain the core result and offer no replacement. Reset all cancels capture, restores defaults through core and discards all staged conflict candidates. Live dictation disables editing, and generic/full settings writes cannot bypass the guarded core path. Next model remains disabled with a second-model/native-hook explanation, while the parallel T11b switch-disabled and cleared-key reasons can leave configuration editable. Keyboard focus survives shortcut writes and view navigation removes capture listeners and rejects late events.

Preserved T7 complete Dictionary/Styles subpages and deep links. Updated DESIGN's six-category contract and all fixed Find/New recording key claims to describe configured shortcuts with Cmd/Ctrl+F and Cmd/Ctrl+N as defaults. Reused h/controlButton/keycaps, existing theme roles and native capture; no dependencies, custom menu implementation, private color literals or mobile branding changes.

Changed files in T10g: new `views/shortcuts.ts` and focused unit/browser tests; `main.ts`, `workspace-state.ts`, capture state/presentation/recorder helpers, task-shell CSS, backend main/workspace route aliases, settings EN/NL catalogs, focused fixture adapter/cases, corrected platform-valid DOM fallback fixture input and DESIGN.md.

Evidence under worktree `build/shortcut-ui/`:

- `t10g-unit.log`: full frontend **469/469** tests, zero failures.
- `t10g-build.log`: TypeScript and Vite production build, exit0.
- `t10fg-native-final.log`: combined T10f + T10g + T10e full Mac backend **374/374** tests, then strict `cargo clippy --bin pennel --tests -- -D warnings`, then Rust format check; all pass. The only too-many-arguments allowance is the exact parent-authorized function-scoped Tauri summary command annotation, not a global exemption.
- `t10g-unit-red.log` / `t10g-unit-green.log`: missing category/derived-key behavior red, then **2/2** route and Command keycap tests green.
- `t10g-reset-red.log`: expected browser failure “Reset all retained a stale replacement candidate”; final browser checks prove the scoped reset event clears that candidate.
- `t10g-final-after/manifest.json`: **24/24** focused synthetic browser cases pass (EN/NL × light/dark, wide/minimum/enlarged/reduced-motion, native protocol, conflicts, Wayland capability/fallback). No horizontal overflow, unknown commands or runtime banners. Primary modifier and reserved-key fixture checks now use valid Mac Alt+Meta and Linux Ctrl+Alt candidates rather than accepting invalid bindings.
- `t10g-focus-final/manifest.json`: **4/4** additional focused protocol/focus cases pass, including retained focus after a central settings save, atomic replacement, Reset all from conflict, native Escape, view leave and disposed late events.
- `t10g-matched-before-final/manifest.json` and `t10g-matched-after-final/manifest.json`: **12 + 12** same-input source renders, identical fixture SHA. Before uses source `a364042` only; after has no baseline override. Actual Settings routes show the pre-category Dictation view before and the new Shortcuts category after. Cases: wide1280×860, minimum760×520 and enlarged body/label/content18px with reduced motion, EN/NL × light/dark. Earlier exploratory/mismatched-route directories are separate and not the paired acceptance artifacts.
- `.omx/state/shortcut-ui/t10g-progress.json`: visual verdict93 against screen27, with inline conflict layout also inspected against the reference family.
- `git diff --check` and clean worktree verified after commit.

Remaining gates: parent independent review and global integration with the parallel model/pill track; no native hardware fn/sides/layout/withholding, signed focus/permission, secure-input, VoiceOver or real-device acceptance. Windows/Linux source parity is retained and portable policy compiles in the Mac suite, but no non-Mac builds were run. Parent owns tracker, draft PRs/assets and merge/release. No external writes, push, merge or release were performed by this lane.

## Final Cancel seam receipt

Final commit `f948dd5742d6da09a24e601be02548b12e520bd6` extracts only the Cancel field/helper/presenter/locale/lifecycle hunks from the pill author's stable `0c60b6d64d0cf75b84ad158c94095306b58724c2`. It retains this lane's effective Dictate Done label and imports no model controller, color, size, position, row-decorator or pill-group stack. The optional Cancel label comes from `record_shortcut::listening_action_ready(shared,3)` plus core formatting in the current locale, and clears before Working/Hidden. The frontend uses the supplied live label rather than an Esc literal or idle settings availability. Existing phase publication and content locking are retained; no new cloned snapshot emission was added.

`cancel-hint-red.log` establishes the expected missing custom-label failure. `cancel-hint-final/manifest.json` has **4/4** EN/NL light/dark custom-label/unavailable/processing browser checks, with the final Dutch dark pill visually inspected and bounded. `cancel-hint-unit.log` has **469/469** full frontend tests, `cancel-hint-build.log` has the production build, and `cancel-hint-native-final.log` has **375/375** full Mac backend tests followed by strict Clippy and Rust format, all passing on the final minimal source. `git diff --check` and a clean worktree were verified after the commit. Parent should preserve both this effective Done helper and the model author's live Cancel/Next metadata when combining the parallel presenters.

## Bounded independent-review correction — final source a9b6bbf

The two proven T10f/g P2 findings are corrected in `a9b6bbf90fe45a6296767f3aa423d4a3b471ae15`, with a clean worktree. Its immediate base is `b38fbfbcf3df8850e178b6b4c943196e3ad6a270`, the separate integration of native X11 P1 source `ddb9dc20059ec5d3ef4a2ec13ddca4cdb09f1a31`. The isolated frontend correction artifact is `t10fg-correction-review.diff` (`b38fbfb..a9b6bbf`); it excludes that native dependency delta. The minimal Cancel presenter seam remains intact, and the reviewed locked Command derivation is unchanged.

1. Completed-but-uncommitted capture now has independent commit permission. Focus cancellation and Reset all revoke it immediately; completion checks it after native stop, so the old binding callback cannot overwrite current/default settings. A finishing session keeps input ownership until teardown, and the completed writer still performs idempotent cancellation without waiting on its own completion promise.
2. Capture and DOM dispatch share a base-key resolver using the optional browser Keyboard Map. Shift/Alt printable characters are not treated as base keys or inferred from US physical-code names. Without a safe map, ambiguous Windows/Linux app bindings stay stored and are visibly unavailable, and ambiguous native or DOM candidates are refused before a settings write. Users can record a function-key replacement; native Mac menu handling stays supported. Space, including Alt-generated non-breaking space, remains a named key. Layout refresh invalidates old maps and ignores stale asynchronous replies.
3. The exact X11 capability reason is consumed through the native backend's EN/NL alias. Focused fixtures reflect unsupported keyed globals and the legacy Record capability while retaining safe modifier-only/application controls. Native observation consumes DOM defaults without constructing a second binding. No new core action ID, dependency, UI palette or broader feature change was introduced.

Evidence in the assigned worktree's `build/shortcut-ui/`:

- `review-commit-revocation-red.log`: both held-stop cancellation/reset regressions failed before the fix; subsequent focused suite passes them and retains the completed-writer self-wait regression.
- `review-layout-red.log`: wrong shifted base key, unavailable-map save and shifted dispatch failures reproduced before the fix. `review-space-red.log` and `review-native-dom-red.log` also establish the narrow Space/native-observation regression boundaries.
- `review-focused-unit.log`: **32/32** focused capture/layout/dispatch/presentation/routes/localization checks pass.
- `review-fg-build.log`: TypeScript and Vite production build pass on the corrected source with the X11 pin integrated.
- `review-layout-browser-final/manifest.json`: **4/4** targeted English/light browser cases pass. They exercise real DOM capture events for Ctrl+Shift+Digit1 → base1, a supplied non-US KeyQ/Alt character → basea, central API writes plus actual Search dispatch, reserved-key rejection, unavailable-map DOM and native candidates preserving saved settings, function-key recovery, and exact X11 capability/observer behavior. These are synthetic owned-browser protocol checks, not hardware or non-Mac runtime acceptance.
- `git diff --check` and clean worktree verified after the correction commit. No broad matrix, full native suite, Windows/Linux build, dependency install, subagent, push or release was repeated. Native P1 focused evidence is owned by its author: core14/catalog11/portable6/capture5 plus build/strict lint.

Known truthful limitation: without safe browser layout data, Windows/Linux Shift/Alt printable app shortcuts are unavailable, including a default such as Ctrl+Shift+S where applicable. The saved setting is retained; buttons, unmodified bindings and function keys remain available. This is the explicit fallback requested by review, rather than claiming a binding the DOM handler cannot match. The browser API's base-key contract is documented in https://wicg.github.io/keyboard-map/ . Hardware, signed focus/permission, secure-input and non-Mac delivery acceptance remain separate.

Shared-stash recovery receipt: my temporary P2 work was restored by exact object `e2477aa9218574577f93badd517b170992c9d91e`. A later timestamp-selector drop reported that object but its readback still showed it and omitted pill stash `7e14b4fa10323500e023554896f32e6a26f20246`. I immediately restored that exact pill object with its original message using `git stash store`, then verified these three entries: `7e14b4fa10323500e023554896f32e6a26f20246` (Owned T11c overlap before X11 correction ddb9dc2), `e2477aa9218574577f93badd517b170992c9d91e` (this lane's temporary P2 checkpoint), and `e62c7c19172093766e31cb1977f2f7a871d31a5f` (Owned T11c in progress before native T10e integration). No worktree source was lost. Shared stash entries are now left untouched; do not use timestamp/index cleanup for them.
