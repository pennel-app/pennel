# T4 inline Markdown editor independent review

Reviewed pinned `a463cdabec735510c0f60e9d66d79a9e91449d42` → `93b4e0a630d7d6738f9e7b27a6a3259a3fd54815` (component `85fb65c` plus integration), using `t4-review.diff`, the report, track/preparation and applicable constraints/design rules. T6b/T11c later work was excluded. No source/index/HEAD changes, subagents, tests/builds, native checks, CI or external writes beyond this requested report.

**Disposition: one P2 explicit-scroll ownership finding. Remaining reviewed spec/quality paths pass; no separate proven privacy/security defect.**

## Finding

### [P2] Supersede pending explicit source reveals when a newer bookmark is restored

**Location:** `apps/desktop/src/inline-notes-editor.ts:133`, together with `setRange` at lines 123–125 and the source return at `live-workspace.ts:981–986`.

`revealSelection()` dispatches CodeMirror's asynchronous `scrollIntoView` effect. A later selection-only transaction and direct scroller/document bookmark assignment do not revoke that pending effect. The exact pinned view 6.43.15 keeps `viewState.scrollTarget` across selection-only transactions, maps it through changes, and consumes it in the later measure pass (installed primary source `dist/index.js:8063–8078,6374–6375,8289–8291`; measurement is scheduled through RAF at 8399–8401).

Consequently, opening a deep note citation and returning before that measure pass can restore the original canonical selection and `documentSurface.scrollTop`, then have the old source reveal recenter the document afterward. The same gap applies to a newer explicit scroller assignment before an outstanding reveal settles. The existing quiet-assignment regression removes custom queued snapshots, but does not supersede this CodeMirror-owned explicit target. This violates the source-return/newer-scroll ownership contract without changing stored Markdown.

**Minimal fix:** give the facade's explicit reveal an ownership generation and supersede/cancel it on a newer selection/bookmark restoration or deliberate scroll. Use a public CodeMirror scroll-handler/measurement mechanism or equivalent; preserve ordinary user caret scrolling and avoid making all `setSelectionRange` calls reveal automatically.

**Focused regression:** delay the explicit reveal's measure, open a real deep note source, restore the prior selection/document bookmark or return to its answer before releasing that measure, then assert exact restored scroll/selection after all queued work settles. Retain the existing nonzero quiet-scroll and deep finalization cases. No broad/native rerun is needed for this correction.

## Remaining spec/quality assessment

The actual `.live-notes` surface contains CodeMirror's real editable contentDOM with no hidden textarea. Canonical Markdown remains the model; the facade exposes exact UTF-16 selection/direction and actual focus/scroller ownership. Parser/decorations preserve source text, reveal caret/selected-line markers, resolve the approved heading/bold/bullet/task subset and retain literal code/escapes/unsupported syntax. Generated user text is never executable HTML.

Quiet `.value` changes emit no typing event or undo step. Toolbar/task/people/suggestion changes use genuine isolated user transactions, and the existing input listener owns draft/autosave once. People writes are acknowledged without a duplicate backend save; insertion receipts/context checks remain. Undo/redo and native beforeinput history reuse CodeMirror's implementation. Composition/read-only/disabled guards, actual content focus, source offsets and one editor through finalization/refresh/Settings return remain coherent in the reviewed paths.

Effective app/capture shortcuts take precedence over editor keymaps through the existing dispatcher; there is no second app-command table. T6a review/source/notes APIs and summary boundaries remain intact. Affected native verification/performance scripts now use contentDOM/facade user transactions and retained workspace selectors; their execution is not claimed.

Only the three explicitly approved direct CodeMirror packages were added, exactly pinned. Their normal locked transitive infrastructure and all twelve bundled MIT notices are documented. No core/backend/schema/mobile behavior or extra parser dependency was introduced.

## Evidence and limits

Read the **508/508** test/build receipts, actual-content component result and all supplied manifests; accepted the recorded **13 focused parser/state tests** and script syntax/license checks without rerunning them. Manifests are complete with no recorded overflow, runtime/surface errors, unknown commands or banners: 16 before, 24 final after, four minimum interactions, seven stable-paint integrations, four final ownership and two recovery/retention cases. Existing browser evidence covers real CDP text, beforeinput history, app-shortcut precedence, people/suggestion undo/redo, exact sources, deep composing finalization and dirty recovery; it does not hold an explicit reveal across a newer return/bookmark before measurement.

Physical IME, native WebKit/menu dispatch and native performance remain disclosed acceptance limits. The Vite chunk-size notice is a reported warning, not a verification failure or additional finding. No native/device acceptance is inferred from the synthetic composition lifecycle.

**Correction scope:** explicit reveal/return scroll ownership plus its focused race regression; no broader editor redesign requested.
