# T11c — pill preferences and display-only preview

Status: implementation and verification complete; source is frozen for the parent's independent review at **`194a81e6b218f7be5a78b445ba872b88ac9a6529`**. The worktree is clean. Exclusive worktree `agent-a65210de8afb33f96`, branch `codex/redesign-pill-models`. T11c is based on **`9e3201117526364ed64eabdbaffcaa85618ac0f5`**, including the separately reported T11b corrections, the native X11 safety correction (`ddb9dc2` → `9f65729`) and owned Cancel/lifetime test (`d56237b` → `1ff83ae`). Parent owns review, draft PR/assets/tracker and final aggregate integration. No merge, release, click acceptance or device acceptance is claimed.

## Result

Core owns serde-defaulted `PillColour`, `PillSize` and `PillPosition`. Missing fields preserve Ink, Regular and Bottom centre without changing the existing speech choice. All deliberate variants round-trip. The optional desktop/mobile TypeScript mirrors preserve old payload compatibility.

Token source owns explicit Ink/Light themes, per-size geometry and Figtree label/content/caption sizes. The generator validates text/indicator contrast at 4.5:1, variant metric validity and enough room for two recent caption lines plus a structured recovery message. Compact uses **360×152**, Regular **420×164**, Large **500×200**; these include transparent frame allowance. The ordinary Regular capsule retains its existing type/padding and bottom inset. Increasing the frame allowance fixes actual clipped Dutch Problem+caption states. Tiny viewports suppress only caption display and retain readable recovery detail. Base mobile palette/fonts and artwork values remain unchanged.

Bottom centre, Top centre and Near the cursor use shared dimensions. AppKit operates in logical points; the portable implementation scales/clamps physical frames to the selected display work area. Geometry tests cover negative origins, scaling, tiny areas, all sizes, cursor bounds and fractional rounding. Wayland retains the position control disabled with a visible reason. No focusability, activation or cursor-event flags changed.

Settings contains the real preview, Colour, Size, Position, the shared live-text scope, existing real model-switch preference and Show the pill. The preview uses fixed Figtree; standard position choice uses the shared native popup to keep long labels bounded. Show creates/reuses a separate nonactivating click-through `pill-preview` window with a clearly labelled Preview state for 2.4 seconds. It does not subscribe to capture snapshots/recognition events or change capture, text, History or clipboard state. Response/timer ownership belongs to the view; native demo generations reject obsolete timers and real capture cancels the demo. Native preference refresh updates visible real/demo frames without changing capture ownership.

## Accepted appearance clarification

The following choice is **Match Pennel / Volg Pennel**, retaining the stored `system` value. It follows Pennel's existing global Appearance hierarchy: System follows the OS, and an explicit Light/Dark override applies to the following choice. Ink and Light stay fixed CSS choices. A short setting help explains this, and explicit app-appearance selectors keep the real/demo pill and inline preview consistent even when app appearance is opposite to the OS media query.

This small handoff clarification was directed by the parent after checking the installed primary SDK source. Tauri 2.12.5 `Window::set_theme` documents that the operation is app-wide on Linux/macOS; Tao's macOS implementation calls `NSApplication.setAppearance`. The temporary per-pill System override/routing-only test was discarded entirely. The existing global native appearance loop is unchanged; there is no raw OS probing, new appearance setting, metadata reader/event or dependency. `DESIGN.md` and `docs/desktop.md` document the hierarchy and constraint.

## Verification

All Cargo commands use the supplied warm-cache `with-cache-lock.py core|backend` wrapper, including its fresh-checkout input guard. No cache deletion, non-Mac build or CI wait.

- **Core full:** `cargo test --manifest-path core/Cargo.toml`: **1235 passed, 7 ignored**, plus bindgen/doc test targets passed; `/tmp/pennel-pill-complete-core-tests.log`. This preceded the isolated one-file observer-drain correction; its **45 focused dictation tests/1 ignored**, strict clippy and release library build are in the T11b receipt. Root's final aggregate checks include that extra regression.
- **Core resource/final static:** `cargo test --manifest-path core/Cargo.toml --lib localization`: **11 passed** after the label clarification; `/tmp/pennel-pill-follow-core-localization.log`. Strict `cargo clippy --lib --tests -- -D warnings` and release library build passed (`/tmp/pennel-pill-follow-core-clippy.log`, `/tmp/pennel-pill-follow-core-build.log`). The later observer correction also passed strict clippy/release at its exact source pin.
- **Backend full:** **376 passed** on the complete dependency/appearance stack; `/tmp/pennel-pill-final-backend-aggregate.log`. Focused final pill states/geometry/timers **16 passed**; `/tmp/pennel-pill-follow-backend-tests.log`. Strict backend clippy passed (`/tmp/pennel-pill-follow-backend-clippy.log`). The actual desktop application binary builds and links against the new Swift export and final core (`/tmp/pennel-pill-final-linked-build.log`); no bundle/release was published.
- **Frontend:** `npm run build` and full `npm test`: **452 passed**; `/tmp/pennel-pill-follow-frontend-build.log`, `/tmp/pennel-pill-follow-frontend-tests.log`. Existing fixture assumptions were updated for display-only demo entries and nested model metadata values. The earlier mismatched entry assertion was fixed and the full suite rerun.
- **Mac services:** full `swift test --package-path apps/macos-platform`: **216 passed**, zero failures; `/tmp/pennel-pill-final-native-integrated.log`. This includes the three new geometry tests; native Swift source has not changed since this run. Actual native event/focus/monitor acceptance is separate.
- **Generator:** `python3 scripts/generate-theme.py --check` and `--self-test`: **23 passed**; `/tmp/pennel-pill-follow-generator.log`. New tests failed first for missing variants, contrast/type handling and insufficient frame allowance; final checked-in outputs reproduce exactly.
- **Mobile mirror:** `npm run typecheck`, `npm run test:ui`: **160 passed**; `/tmp/pennel-pill-follow-mobile-type.log`, `/tmp/pennel-pill-follow-mobile-ui.log`. No mobile UI, native project/configuration or branding changes; no Prebuild/device acceptance is claimed.
- Core/backend `cargo fmt -- --check` and `git diff --check` passed.

Meaningful red→green behavior: settings upgrade/roundtrip, generator theme/size validation, native coordinate geometry, backend frame/timer ownership, and an actual stale-info race that let an older async settings response overwrite newer pill preferences. The latest-info generation now rejects it. A next-model label was also found persisting outside listening; its focused state test failed before the clear and now passes, and boot snapshots recompute live label availability.

## Focused visual evidence

All images are owned synthetic app/headless renders, never the user's screen. No PNG enters code history.

- `t11c-after-final`: **32 images, zero failures**, EN/NL × light/dark. Covers Compact Ink, Regular Light, Large following appearance, structured recovery, tiny viewport recovery, 760 px settings, doubled text and Wayland position explanation. The initial set exposed four clipped Dutch recovery states; shared frame allowances and the tiny-height policy corrected them.
- `t11c-before`: **16 baseline-flag images, zero failures**, exact `5d569c8` source and matched input for the selected pill/settings cases.
- `t11c-follow-appearance-final`: **16 images, zero failures** for the accepted label/help, minimum/enlarged controls, actual preference persistence and Show action.
- `t11c-global-appearance-verified`: **8 images, zero failures**; both the real pill and inline preview are checked under forced-app/opposite-OS inputs, switched back to System, then restored to the explicit override. No native theme policy is changed.
- `t11c-wayland-final`: **4 images, zero failures**; supported-state controls stay visible and native reason IDs do not leak into recovery copy.
- Display-only preview privacy passed in `t11c-demo-final` (**4 EN/NL light/dark images, zero failures**) as well as `t11c-functional-verified`; its production entry does not call `dictation_state` or mutate capture/text/clipboard state.

Final `.omx/state/t11c/ralph-progress.json`: **96/pass**, with the label clarification recorded explicitly.

## Changed areas and simplifications

Core settings/exports; desktop/mobile API mirrors; token/generator/generated CSS+metrics; pill renderer/styles and Dictation settings; backend pill windows/geometry and minimal command registration; macOS placement bridge plus one geometry helper; overlay event capability; focused tests/fixture harness; DESIGN and desktop implementation docs. Reused the existing native popup, controls, localization and nonactivating panel flags. Removed the now-unused Rust bottom-only wrapper and kept one shared token source; no second core, dependency or speculative OS theme service.

## Remaining acceptance and artifacts

Parent's one independent T11c review and final combined integration remain. Actual microphone/external-target insertion, multi-display pointer placement, signed/native focus and pill-click safety are not established by unit tests or mock renders. The pill remains click-through; its click route stays gated. Windows/Linux builds are paused by policy; portable source was kept/reviewed and actual keyed-X11 unavailability is preserved. Mobile native/signing/store gates remain open as before.

Owned applied backup stashes are retained by exact hashes (`e62c7c19172093766e31cb1977f2f7a871d31a5f`, `7e14b4fa10323500e023554896f32e6a26f20246`) because the stash reflog is shared across agents; no unrelated stash was removed. Parent can clean those obsolete backups after coordinated handoff.

Immutable T11c review diff: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t11c-final-review.diff` is `9e32011..194a81e`, excluding all previously reported T11b and native prerequisite commits. Staged-file secret hook passed; no image, dependency or generated native project entered code history.

## Independent-review demo lifecycle correction — 39885c9

The one remaining reviewed P2 is corrected in **`39885c91bc8a32deabfb54afec36305a55012cd1`**, directly above `194a81e6b218f7be5a78b445ba872b88ac9a6529`. The worktree is clean. Exact one-file review diff: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t11c-demo-lifecycle-correction.diff`.

Ownership protocol:

- A Show request reserves its generation before the first idle check and window creation. It becomes active only after creation and a same-generation admission check. A real-pill transition records real ownership and invalidates pending/active demo generations before content/native work. Returning to idle cannot revive the earlier request.
- Queued cancellation hides and the existing finite timer carry their own generation. They cannot hide a newer valid preview. Failed old admission only cancels its own generation.
- Native show/hide uses a short state snapshot with no state guard held across the native operation. It compares the latest generation/desired visibility after the operation and reconciles again if a native reentry changed ownership. Preference refresh, show, cancellation and expiry share this path. An old native hide finishing after a reentrant newer Show is therefore repaired to the newer visible state.
- Display-only privacy, Match Pennel hierarchy, renderer/motion interfaces, geometry, 2.4-second duration and all focus/nonactivation/click-through flags are unchanged. The retracted chip candidate was not changed.

Controlled regressions use the production creation/native-I/O boundaries. The extracted old ordering reproduced three red cases in `/tmp/pennel-t11c-demo-race-red.log`: capture cancels during creation but late Show still succeeds; an old queued cancel hides a newer valid demo; and an old native hide finishes after a reentrant newer Show and wins incorrectly. Green checks also cover capture reentry during native show, actual state-lock availability inside native callbacks, a real-owner marker ahead of stale content flags, capture returning idle before late admission, and the existing obsolete-timer behavior.

Final focused command: `CARGO_INCREMENTAL=0 python3 /Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py backend cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml pill::tests` — **20 passed, zero failed** (`/tmp/pennel-t11c-demo-race-verified.log`). Strict backend clippy with the same incremental setting and `--tests -- -D warnings` passed (`/tmp/pennel-t11c-demo-race-clippy-noincremental.log`). `cargo fmt -- --check`, `git diff --check`, staged-file secret hook and the clean status check passed.

The first incremental clippy attempt hit an environment ENOSPC writing the shared backend dep graph (`/tmp/pennel-t11c-demo-race-clippy.log`); no shared cache or other worktree was deleted. Disabling incremental compilation resolved the verification blocker. No broad core/frontend/native/visual matrix or dependency changes were performed for this correction. Parent should review only `194a81e..39885c9`, then integrate into its combined frontend/native stack.
