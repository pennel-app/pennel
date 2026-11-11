# T12 final motion and Share polish handoff

Status: presentation implementation frozen for parent-owned independent review and combined verification. This is not a full-redesign, native-device or release acceptance claim.

Presentation baseline: `d1f36579347aa4ae9fec6e390d7bec6c6d85e7b3` (all integrated feature UI). Motion head: `d7ed60eb4a4db7dae302c8499e57f28417a39326` (`feat: keep state changes legible without delaying work`),23 files,459 insertions/53 deletions. Tracked worktree is clean; commit secret scan and final diff check passed. Branch/worktree: `codex/redesign-motion-pass`, `/Users/lander.parren/Developer/pennel-app/pennel/.claude/worktrees/agent-ad503c1c97f14bf16`.

The parent explicitly directed freezing T12 before the independent native demo admission/hide-generation correction. That native-only dependency has no frontend interface coupling and remains parent-owned for integration and combined native verification.

## Scope and decisions

The pass applies the approved feedback120ms, switch150ms, move220ms and reveal320ms roles and existing cubic-bezier(.2,.8,.2,1). Move exit remains130ms, approximately60%; the explicit pill220ms/8px enter,120ms state crossfade and140ms leave remain. Pill model confirmation900ms and backend Inserted900/Copied2400/Problem3500 lifetimes were not changed. Existing native timers and outgoing ARIA-hidden pill status ownership remain in place.

Recent rows now separate a320ms reveal,220ms room-making and the existing1600ms tint. Direct row Copy feedback still owns its1200ms reset. Real shortcut observation drives a waiting pulse and new keycap pops, without replacing/reanimating identical observed keys. The existing measured recording capsule width path is retained, with220/130ms resize and a real120ms color fade under reduced motion. Audio levels remain exclusively driven by the existing session-owned actual level events.

New transcript/caption arrivals reveal only on new rows or empty-to-content arrival; clock refreshes and partial revisions do not restart them. Companion opening/closing animates while logical close and focus return remain immediate. Its inert, ARIA-hidden shadow-host visual copy sits outside functional selectors, strips IDs/roles/live-region references/data action hooks, and is superseded on reopen or disposed with its owner. Notes are never cloned. Wide pane size motion cancels on direct pointer/keyboard input, so divider resizing and editing remain immediate.

When the caret leaves a Markdown line, its rendered line receives220ms color/tint settling. Canonical Markdown, CodeMirror source decorations, selection and composition updates remain synchronous; text entry is not animated. Summary working placeholders pulse only during an actual request, then actual sections reveal90ms apart. Actual speaker timeline spans draw after the section arrival window, without redrawing unchanged timeline data. Accepted review cards settle over220ms and their nonzero count ticks only on an actual count change. Sidebar selection slides on actual task changes.

Both system `prefers-reduced-motion` and the app's existing native-environment `data-reduce-motion` route remove movement/pulsing and retain actual120ms opacity/color feedback. The pill now consumes the same existing desktop-environment event/API, while keeping its own Figtree stylesheet and tokens. No main desktop typography/theme stylesheet enters the pill.

Share title uses the approved20px `font-title` role, and its two equal scope segments stretch across the reading width. Minimum/enlarged dialogs retain their scrollable reading body and reachable footer.

## Changed files and why

- `apps/desktop/src/motion.ts`, `motion.css`: small shared presentation trigger/cleanup and semantic desktop animations, including reduced-motion fades and safe visual-only companion exit.
- `apps/desktop/src/main.ts`: imports the desktop-only motion sheet.
- `apps/desktop/src/styles.css`: reuses timing/easing tokens on existing controls; keeps capsule color feedback when movement is reduced.
- `apps/desktop/src/task-shell.css`, `views/pad.ts`: split result reveal/room/tint and animate the existing Copy acknowledgment without changing its timer.
- `apps/desktop/src/views/library.ts`: slides the selection paint for actual primary-task changes.
- `apps/desktop/src/shortcut-recorder.ts`: avoids identical keycap DOM replacement and applies key feedback only to newly observed keys.
- `apps/desktop/src/live-workspace.ts`, `meeting-workspace.css`: state-driven companion/transcript/count motion; replaces the former overlay-only enter rule and dead generic summary-reveal CSS with the shared presentation rules.
- `apps/desktop/src/inline-notes-editor.ts`: animates rendered line settling when a non-composing caret leaves Markdown; no input transaction changes.
- `apps/desktop/src/meeting-summary-reading.ts`, `meeting-summary-reading.css`: section/timeline motion follows actual content/visibility changes; replaces static animation declarations and avoids unchanged timeline redraws.
- `apps/desktop/src/reviewed-insights.ts`: settles a retained card only on a committed transition to accepted.
- `apps/desktop/src/pill.ts`, `pill.css`: existing environment motion route, shared pill easing, opacity-only reduced enter/leave, and correct real hide cleanup for the fade animation name.
- `apps/desktop/src/meeting-export-preview.css`: the two approved Share title/scope corrections.
- `apps/desktop/tests/motion.interactions.mjs`, `scripts/fixtures/visual-cases.json`:30 focused owned-fixture runtime cases for the actual required motions and both reduced-motion routes; records keyframe/timing/fade evidence in the existing control acceptance channel.
- `apps/desktop/tests/{meeting-companion,recording-capsule,structured-summary,summary-consent}.interactions.mjs`: existing geometry assertions wait for an intentional arrival before measuring; reduced-motion assertions accept the required real fade while rejecting movement.

No dependencies, mobile/native source, provider logic, stored source content, credentials, tracker, PR, push, merge or release changes.

## Verification receipts

Browser: `/Users/lander.parren/.t3/tools/chrome-headless-shell/mac-arm64/154.0.8037.92/chrome-headless-shell`. All browser work uses the owned synthetic loopback fixtures and temporary owned profiles, never a user screen/profile, microphone or real provider.

1. Regression-first runtime: `t12-companion-normal` failed on baseline with `No actual220ms animation on meeting-companion`. Red receipt: `build/t12/red/manifest.json` and failure PNG. Subsequent runtime checks caught and corrected the active-line Markdown parser seam, the capsule's inherited reduced transition-property override and the direct-resize/open timing interaction.
2. Final production frontend: `(cd apps/desktop && npm run build && npm test)` passed typecheck/Vite build and524/524 tests. Vite retained its existing large-chunk advisory; there were no build/type/test failures.
3. Theme/static: `python3 scripts/generate-theme.py --check`, `python3 scripts/generate-theme.py --self-test` (23/23), and `git diff --check` passed. The exact approved desktop lock was installed with `npm ci --ignore-scripts --no-audit --no-fund`.
4. Runtime matrix: all30 `t12-*` cases passed EN/light using `node scripts/capture-desktop-reference.mjs` with one `--case` per `t12-*` item, `--locale en --appearance light --browser <browser> --output build/t12/runtime-final-2`. This covers normal/system/app routes for pill, companion/transcript/Markdown, shortcut, summary/map, home/Copy, accepted review/count, capsule, request placeholders and sidebar; plus two movement/missing-active-fade negative controls and Share computed roles. `build/t12/runtime-proof/manifest.json` repeated this focused matrix successfully at the frozen head with29 persisted actual keyframe/duration/delay/intermediate-opacity/control receipts in `control_acceptance` (Share is the30th computed-layout-only case). Extracted receipts: `build/t12/motion-receipts.json`.
5. Affected behavior:10/10 EN/light cases passed in `build/t12/affected-behavior-final/manifest.json`: `meeting-companion-keyboard-reopen`, `meeting-companion-resize`, `t9d-record-presentation-review`, `t9d-source-race`, `t9d-reduced-motion`, `summary-consent-reduced-motion`, `t2-home-change-refresh`, `control-states-reduced-motion-system`, `control-states-reduced-motion-app`, `t10g-shortcuts-interaction`.
6. Focused matched renders:32 before and32 after images, all completed without runtime errors/unknown commands/overflow, with matching fixture SHA256. Commands use the same eight `--case` values listed below; before alone adds `--baseline-ref d1f36579347aa4ae9fec6e390d7bec6c6d85e7b3`. No baseline override is used for after. Local manifests: `build/t12/before/manifest.json`, `build/t12/after/manifest.json`; paired paths and SHA256: `build/t12/paired-evidence.json`.

The static cases are `t5a-export-share`, `t5a-export-personal`, `t5a-export-minimum`, `t5a-export-enlarged`, `t2-home-before-ready`, `t9d-summary`, `meeting-companion-live-reference`, `pill-listening-empty`, each in EN/NL and light/dark. This is a focused comparison, not a whole-app catalogue. The four Share cases include760×520 minimum and enlarged text.12 non-Share pairs are pixel-identical; four summary pairs intentionally sample the new map draw at the fixed fixture capture time;16 Share pairs contain the approved title/scope correction.

Representative matched image paths relative to this worktree:

- `build/t12/{before,after}/en/light/t5a-export-share.png`
- `build/t12/{before,after}/en/dark/t5a-export-personal.png`
- `build/t12/{before,after}/nl/dark/t5a-export-minimum.png`
- `build/t12/{before,after}/en/light/t5a-export-enlarged.png`
- `build/t12/{before,after}/en/light/t9d-summary.png`
- `build/t12/{before,after}/nl/dark/pill-listening-empty.png`

## Visual verdict and remaining gates

Visual-verdict94/pass, threshold90, persisted at `.omx/state/redesign-motion/ralph-progress.json`. Compared actual owned frames with HANDOFF11/12 and motion references41/42/43 plus section5/prototype timings. Share hierarchy, full-width scope, minimum/enlarged reachability and inherited desktop surfaces pass this bounded review. Fixture meeting text/background companion data differ from reference content; they were not redesigned. PNGs demonstrate layout; the runtime animation objects, midpoint fades and negative controls establish motion timing and reduced-motion behavior separately.

Parent owns the independent T12 review, native-only demo correction, final combined core/backend/Swift/native suite, tracker/PR publication and assets. This work did not repeat native/Cargo/mobile/non-Mac or whole-app catalogue checks. Actual WKWebView/compositor/native focus, OS menus, screen reader/hardware/IME behavior, real capture/provider delivery, signed-device/keyboard/Cloud/store and packaged/release acceptance remain unproven by these synthetic renders. The fallback250ms companion paint cleanup is presentation-only and does not own any operation; real native timing still needs the parent's device acceptance.

Artifact recovery: the extra receipt repeat initially hit shared-volume ENOSPC. Only this lane’s disposable intermediate renders were removed; final before/after, red failure and verification artifacts were retained. The successful frozen-head repeat completed all30 cases after space recovered.
