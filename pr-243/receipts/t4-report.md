# T4 — retained inline Markdown notes

Ready for parent independent review. Branch `codex/redesign-inline-notes`, clean worktree `/Users/lander.parren/.codex/worktrees/redesign-dictation-home/pennel`.

- Frozen integration base: `a463cdabec735510c0f60e9d66d79a9e91449d42` (`codex/redesign-editor-foundations`). Parent foundation includes the resolved T5b/T9d state/read guards and T6a Review layout.
- Independent component checkpoint: `85fb65c0bac902a26d643d64d5e98b60f22567b1`. Its pre-rebase identity was `5070413c48e76e4e79c3ea8def67fed09a098693`; exactly this one T4 commit was rebased from `93d23cc` onto a463.
- Integration/final head: `93b4e0a630d7d6738f9e7b27a6a3259a3fd54815`.
- Isolated full T4 diff: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t4-review.diff` (a463..93b4e0a).

## Behavior and preserved contracts

The actual `.live-notes` element is now a retained div containing CodeMirror's real editable `.cm-content`; there is no hidden textarea. One editor survives recording, finished review, refresh, localization, Settings return and source inspection. Canonical Markdown stays byte-for-byte source text, with UTF-16 selection ranges matching existing core/source contracts. The caret and selected lines reveal raw markers, while other lines resolve headings 1–3, bold, bullets and tasks. Fenced/indented/inline code, escapes and unsupported/ambiguous markers remain readable literal text. Inline task controls use their actual task text as their accessible name.

The footer exposes Saved plus H/B/List/Task directly. Removed source/preview toggle state, separate preview DOM and obsolete styles. All four toolbar formats, task toggles, explicit people mentions and suggestion inserts are genuine isolated user transactions with undo/redo. The existing input listener owns draft revision and autosave once. People commands already save their note bytes; their editor transaction is acknowledged immediately, avoiding a second backend write. Suggestion insertion receipts and before-context guards stay intact, and undo/redo does not create another insertion attempt. Accepting an insight still does not insert it automatically.

Quiet `.value` assignments do not emit input or enter undo history. They preserve numeric selection/direction and allow CodeMirror's normal viewport maintenance; no queued scroll restoration can overwrite a newer explicit scroll. Deliberate source jumps call `revealSelection()` separately. Source return restores the original ranges and actual scroller/document bookmarks. Deep live scrolling transfers into the continuous finished document while retaining the focused composing selection.

The facade exposes `value`, `selectionStart/End/direction`, `setSelectionRange`, `disabled`, `readOnly`, `composing`, `contentDOM`, `scrollDOM`, `focus`, `blur`, scroller aliases, `applyUserChange(value, selection?)`, `applyFormat(format)`, `revealSelection()`, `setDocumentMode()`, undo/redo and destroy. Actual focus checks use `contentDOM` or containment. Native form controls elsewhere remain ordinary inputs/textareas. Updated affected interaction fixtures and the two native verification/performance scripts to use the actual facade/user transaction path.

CodeMirror's installed history extension already supports native `beforeinput` historyUndo/historyRedo. Its existing keymaps provide editor history and navigation. The editor yields effective app chords to the existing `desktopShortcut` / capture dispatcher; no second app command table or custom native menu plumbing was added. A browser regression assigns Find to Meta+Shift+Z and proves the actual app dispatcher receives it instead of editor redo.

T6a `insightHost.notes.flush/current/stage`, `changed(openCount)`, `sourceLabel`, source/bookmark APIs and the summary APIs remain intact. The meeting author's T6b dismissal hook can be combined directly in `insightHost`; T4 does not add or modify it. No backend/schema/settings/mobile changes.

## Packages and notices

Only the three explicitly approved runtime packages were added, pinned exactly:

| Package | Version | License |
| --- | --- | --- |
| @codemirror/state | 6.7.6 | MIT |
| @codemirror/view | 6.43.15 | MIT |
| @codemirror/commands | 6.11.1 | MIT |

Their normal locked transitive graph adds @codemirror/language6.13.1, @codemirror/streamparser6.0.0, @lezer/common1.5.5, @lezer/highlight1.2.5, @lezer/lr1.4.12, @marijn/find-cluster-break1.0.4, crelt1.0.7, style-mod4.1.4 and w3c-keyname2.2.8. All are MIT. No Markdown language/parser dependency was added or configured. `apps/desktop/public/third-party-codemirror.txt` contains all12 full installed notices and is copied by Vite into dist. `cmp` verified the distributed file equals the source notice bundle. npm install reported zero vulnerabilities.

Official references consulted before implementing: https://codemirror.net/examples/decoration/ and https://codemirror.net/docs/ref/ (direct requests403); current official GitHub state/view/commands source and the installed pinned types/source supplied the exact working API, history and composition contracts.

## Red → green evidence

1. DOMless parser and transaction tests were written first. Initial missing behavior failed before implementation. The later multiline-code regression reproduced `1 !== 0` for a heading prefix inside a multiline code span, then passed after literal-range protection and CRLF blank-paragraph handling. Final focused parser/state run: **13/13**.
2. Integrated browser regression before wiring failed exactly: **“Workspace still uses a textarea instead of the actual inline Markdown editor.”** Command: `PENNEL_CHROME=<browser> node scripts/capture-desktop-reference.mjs --case t4-notes-check --locale en --appearance light --output build/t4-red`. Red artifact directory: `/Users/lander.parren/.codex/worktrees/redesign-dictation-home/pennel/build/t4-red`.
3. A real browser regression exposed a queued scroll snapshot replacing a newer explicit scroll (72→0). Strengthened independent assertion failed **“A deferred programmatic selection snapshot overrode the newer actual scroll.”** Removed queued snapshots, kept explicit source reveal separate; the component check now requires a genuinely nonzero (>100px) retained scroll. A second 72→98 difference occurred during the initial user edit's asynchronous layout before any concurrent update; the existing concurrency case now freezes its protected context after two actual animation frames. Subsequent answer/transcript updates preserve that settled context.
4. Integrated t4-notes-check is green for exact stored Markdown, one formatting input/save, native beforeinput undo/redo, task save, info-refresh identity, finalization identity and app-shortcut precedence. Deep finalization separately confirms line70 of120 remains visible with the same actual content focus and backward canonical selection during composition.
5. People mention and failed-save suggestion insertion cases now also exercise real editor undo/redo and exact persisted source. Existing duplicate-note source/Share-return case passed after migrating its already-moved export-preview selector to the actual Share dialog.

## Verification receipts

All paths below are under `/Users/lander.parren/.codex/worktrees/redesign-dictation-home/pennel/` unless absolute.

- `node --experimental-strip-types --test apps/desktop/tests/inline-notes-markdown.test.mjs apps/desktop/tests/inline-notes-state.test.mjs` —13 passed.
- Owned actual-content component harness `node build/t4-component/run.mjs` —green; it sends real CDP `Input.insertText` with emoji, then checks actual content focus, quiet selection/scroll, one formatter/task transaction, native beforeinput history, readonly/disabled, locale identity and composition lifecycle. Receipt `build/t4-component/result.json`. Harness is an ignored local runner; reusable assertions are committed in `tests/inline-notes.interactions.mjs`.
- `(cd apps/desktop && npm test)` — **508 passed, 0 failed/skipped**, `build/t4-final-unit.log`.
- `(cd apps/desktop && npm run build)` — **TypeScript + Vite green**, `build/t4-final-build.log`. Vite reports its >500k minified chunk warning; no build error. No unsolicited bundling/dependency changes were added for this warning.
- `python3 -m py_compile scripts/verify-live-workspace-native.py`; `node --check scripts/native-performance/benchmark.js`; `git diff --check`; notice `cmp` —green.
- `build/t4-final-after/manifest.json` —24 images, all green: t4-notes-check and t4-notes-deep-finalize across EN/NL × light/dark, plus the four public-facing pairs below.
- `build/t4-final-min-check/manifest.json` —4 green minimum-window actual-editor interaction checks across EN/NL × light/dark.
- `build/t4-integration-stable-paint/manifest.json` —7 green EN/light focused regressions: concurrent notes/composition + frozen question, people mention, stale composing mention, suggestion failed save, suggestion insertion race, minimum companion, finished document.
- `build/t4-final-ownership/manifest.json` —4 green EN/light final regressions: people mention undo/redo, suggestion insertion retry/undo/redo, composing finalization with stale status suppression, duplicate-note exact source and Share return.
- `build/t4-final-retention/manifest.json` —2 green NL/dark cases: failed-storage Settings return and capture Stop retaining the dirty draft.

The capture tool uses the owned synthetic fixture seam with `/Users/lander.parren/.t3/tools/chrome-headless-shell/mac-arm64/154.0.8037.92/chrome-headless-shell`. No whole catalogue, Rust, Swift, mobile, non-Mac builds, remote CI or shared Cargo target usage.

## Before / after pairs and visual verdict

Before: `build/t4-before`, `--baseline-ref a463cdabec735510c0f60e9d66d79a9e91449d42`.
After: `build/t4-final-after`.

Both use the same synthetic fixture input and exact scenario SHA for all16 paired captures (verified by manifest comparison):

- `{en,nl}/{light,dark}/t4-notes-live.png` —1280×860.
- `{en,nl}/{light,dark}/t4-notes-min.png` —760×520.
- `{en,nl}/{light,dark}/t4-notes-zoom.png` —1280×860 at150% zoom.
- `{en,nl}/{light,dark}/t4-notes-finished.png` —1280×860 continuous finished document.

All final24 captures have zero horizontal overflow, runtime errors or unknown fixture commands. Before/after scenarios use the same text; baseline naturally shows literal Markdown in the old textarea while the new editor resolves it.

Visual verdict **93/pass**, compared against HANDOFF screens16 and22: correct neutral document surface, heading hierarchy, resolved strong/bullet/square task treatment and quiet footer in live/finished/min/enlarged variants. Canonical Inter scale is smaller than the scaled handoff image; localized List/Task text is used for the two small footer controls. No visual expansion outside T4. Verdict persisted at `.omx/state/t4/ralph-progress.json` in this worktree.

## Changed files and limits

Production: three new parser/state/editor modules plus scoped CSS; `live-workspace.ts`; small notes/footer and obsolete-mode rules in `meeting-workspace.css`/`styles.css`; exact package manifests and bundled MIT notices. Documentation: `docs/desktop.md`. Tests: two new unit modules, actual component/integration module and7 one-line fixture cases; existing affected interactions now use actual content focus/user transactions/composition selection helpers. Native scripts: saved/live notes selectors and user transaction calls migrated; their execution is not claimed.

Physical IME input, actual macOS menu dispatch, native WebKit rendering and native performance measurements remain device/native acceptance limits. Synthetic composition lifecycle checks establish retention/guards, not a physical IME session. Real browser text input and beforeinput history were exercised. Parent owns independent review, combined integration verification and publication. No push, tracker write, PR creation or merge was performed by this lane.

## Independent review correction — explicit reveal ownership

Correction head: **`210f3edcd81c2bffcbe6e6d28f63df0ef9b77b76`**, directly above preserved T4 milestone `93b4e0a630d7d6738f9e7b27a6a3259a3fd54815`. The worktree is clean. Isolated correction diff: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t4-correction-review.diff` (93b..210f). The optional dependency-only duplicate backend-test correction `da4cbae` was not cherry-picked; this commit contains only this Notes correction and its approved DESIGN documentation update.

The review identified CodeMirror's remaining asynchronous explicit `scrollIntoView` target. A selection-only restore did not revoke it. The held-real-RAF regression failed before the correction with **“Older explicit reveal replaced the restored bookmark: 640→3127.”** The fixture uses 180 real canonical note lines, the actual contentDOM focus, a nonzero bookmark and an active composition lifecycle; it queues the deep reveal, restores the previous backward selection and scroll in the same turn, then releases measurement. Red artifact: `build/t4-reveal-review-red` (EN/light, exit1).

The facade now gives each explicit reveal a generation and captured public selection/scroll offsets. New document/selection transactions and deliberate facade scroller restores supersede older owners. The public `EditorView.scrollHandler` consumes a stale owned centered reveal; normal current source reveals and ordinary caret scrolling retain CodeMirror's default behavior. Actual scroller offsets are checked too: a direct native scroll assignment can be read by measurement before its scroll event is delivered (the added regression observed 880→3127 before that guard). Native scroll capture also advances ownership. No private view state, editor remount, custom menu table, extra dependency or workspace/summary hook changes.

`DESIGN.md` now states the approved retained inline Markdown behavior, caret-line source markers and Saved + H/B/List/Task footer. The old sentence no longer puts formatting under More; people, speaker setup, templates, suggestions and file actions remain there. No broader prose changes.

Focused green receipts, under this worktree:

- `build/t4-reveal-review-green/manifest.json`: **4/4 EN/light** — the new actual-content reveal race (bookmark restore, newer facade scroll, direct actual scroller change, composition/focus/canonical source, normal explicit reveal and caret scrolling), the real frozen note citation with an immediate answer return while RAF measurement is held, existing deep composing finalization and existing duplicate-note exact source/Share return. The actual citation/return harness also passed before the patch; the explicit reproducible red is the component bookmark race above.
- `build/t4-reveal-review-nl-dark/manifest.json`: **2/2 NL/dark** — both new focused race/source-return checks.
- Existing owned component harness `node build/t4-component/run.mjs`: green again, including real CDP emoji input, genuinely nonzero quiet scroll, backward source selection, actual focus, history, tasks, readonly and composition/localization retention. Receipt `build/t4-component/result.json`.
- `node --experimental-strip-types --test apps/desktop/tests/inline-notes-markdown.test.mjs apps/desktop/tests/inline-notes-state.test.mjs`: **13/13 passed**.
- `(cd apps/desktop && ./node_modules/.bin/tsc --noEmit)`, new interaction module syntax and `git diff --check`: green.

No full screenshot matrix, production build, backend/native suite, physical IME or menu validation was repeated for this focused correction. The public layout is unchanged; prior paired visual evidence still applies. The owned temporary Vite server was stopped. Parent owns scoped re-review and integration.

## Same P2, round two — finished document scroll owner

The scoped review found that the first correction still observed only CodeMirror's inner scroller. In finished document mode that element uses `overflow: visible`, while `.meeting-document` owns reading. That remaining path is corrected in **`4575c812a81bf55d4e55ccfded1d07470510fa8d`**, isolated directly above `210f3edcd81c2bffcbe6e6d28f63df0ef9b77b76`. The worktree is clean. Correction-only diff: `/Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/t4-correction-round2-review.diff` (210f..4575).

A new held-real-measure regression uses the actual reopened finished meeting workspace with 180 canonical note lines, a deep backward selection, the actual contentDOM focus and composition lifecycle. It first establishes a nonzero outer document position, queues the deep reveal, then changes only `.meeting-document.scrollTop` to 1140 without another selection transaction. Before this patch it failed exactly **“Older finished reveal replaced the newer outer reader position: 1140→3343.”** Red receipt: `build/t4-outer-owner-red` (EN/light, exit1).

Each explicit reveal now captures the public scroll offsets of its full ancestor reader chain, including the inner scroller, `.meeting-document` and outer document elements. The existing public CodeMirror scroll handler checks that chain before allowing the owned reveal. A temporary document capture listener observes only the pending reveal's reader chain; it advances ownership when that chain actually moves, avoiding cancellation by delayed events from an earlier already-captured position. The listener is removed when the target is consumed or the editor is destroyed. Direct offset checks also protect changes whose native event has not arrived. No private CodeMirror state, host/workspace interface changes, remount, new dependency or broader architecture change.

Focused green evidence:

- `build/t4-outer-owner-green/manifest.json`: **5/5 EN/light** — new actual finished-owner race, previous live inner-owner race, real citation/immediate answer return, deep composing finalization and duplicate-note source/Share return. The new regression verifies the exact latest outer position, unchanged canonical backward selection, same editor/contentDOM, text/focus/composition retention, and a subsequent current finished source reveal reaching the deep line normally.
- `build/t4-outer-owner-nl-dark/manifest.json`: **1/1 NL/dark** — the new finished-owner regression.
- Existing actual-CDP component `node build/t4-component/run.mjs`: green again, including genuinely nonzero quiet scroll, actual input/focus, canonical selection, history, tasks and composition/localization retention. Receipt `build/t4-component/result.json`.
- `(cd apps/desktop && ./node_modules/.bin/tsc --noEmit)`, interaction-module syntax and `git diff --check`: green.
- All six focused captures are complete with no recorded runtime errors, horizontal overflow or unknown fixture commands. No broader suite, matrix, build, backend/native run or new acceptance claim. The temporary owned Vite server was stopped.

Only three files changed in this continuation: `inline-notes-editor.ts`, the existing focused `inline-notes-reveal.interactions.mjs` and one new one-line fixture case. The original milestones and first correction remain preserved; parent owns scoped re-review and integration.
