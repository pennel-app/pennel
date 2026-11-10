# T4 explicit-reveal correction, round two

Reviewed only pinned `210f3edcd81c2bffcbe6e6d28f63df0ef9b77b76` → `4575c812a81bf55d4e55ccfded1d07470510fa8d`, `t4-correction-round2-review.diff`, the report append and supplied focused receipts. No source/index/HEAD changes, tests/build reruns, native checks, subagents, CI or external writes.

**Original P2: ADDRESSED. Scoped spec/quality verdict: pass. No new actionable breakage found in this correction.**

The explicit reveal now snapshots public offsets for its complete ancestor chain (`inline-notes-editor.ts:153–158`), including the actual finished `.meeting-document` reading owner. Its public CodeMirror scroll handler compares those offsets before allowing the pending centered reveal (`:115–121`), so a newer outer position wins even before the native scroll event is delivered. The pending-chain capture listener also records supersession when an observed reader moves and later returns; delayed events from an unchanged captured position do not invalidate a current reveal (`:103–106`). Existing document/selection generations and facade scroll setters continue to supersede older owners.

The listener is installed only while a reveal is pending and removed on scroll-target consumption or editor destruction (`:117,157,184`). A newer reveal replaces the prior snapshot without accumulating listeners. Current source reveals and ordinary caret scrolling retain CodeMirror's default path; there is no private CodeMirror state access, remount, draft/history/input change or composition/focus mutation.

Read the actual red manifest: the unchanged-selection finished-owner case fails before this correction with **1140→3343**. The committed regression holds real RAF measurement, changes the actual outer document offset while the inner scroller remains zero/visible, then checks exact latest outer scroll, canonical backward selection, retained editor/contentDOM, source bytes, actual focus and composition. It subsequently verifies a current finished reveal reaches its deep source normally.

Read the complete green manifests: **5/5 EN/light** and **1/1 NL/dark**, with no recorded runtime/surface errors, horizontal overflow or unknown commands. These retain the prior live-scroller supersession, immediate real citation/answer return, deep composing finalization and duplicate-note source/Share cases. The existing actual-CDP component result is green for real content focus, quiet selection/scroll, history, tasks, composition and localization. Accepted the supplied TypeScript/syntax/diff receipts without rerunning them; no broader suite/build/native acceptance is inferred.

The previously recorded native WebKit, physical IME/menu and performance limits remain unchanged. No further correction is requested for this P2.
