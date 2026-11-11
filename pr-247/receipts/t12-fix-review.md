# T12 outgoing-scroll correction review

Reviewed only pinned `d7ed60eb4a4db7dae302c8499e57f28417a39326` → `612a5bacbe995c41b91b40ea8621c9c73b886cf1`, `t12-fix-review.diff`, the motion worktree's appended report and supplied focused receipts. No source/index/HEAD changes, tests/builds/native checks, subagents, CI or publication.

**Original P3: ADDRESSED. Scoped spec/quality verdict: pass. No new actionable breakage found in this correction.**

`motion.ts:28–31` pairs source/copy element trees before sanitization and captures nonzero public vertical/horizontal scroll offsets. Restoration happens after the copy is mounted and laid out (`:36–44`), so the snapshot's scrollable dimensions exist. Linked stylesheet loads reapply the frozen offsets if production styling arrives later. The callback writes only copy elements and returns immediately after the host is removed, preserving rapid-reopen/disposal ownership.

The inert, ARIA-hidden shadow host, stripped IDs/roles/live-region/data hooks and placement outside functional selector roots remain unchanged. Logical close/focus return and real reader/draft/selection ownership are not deferred or rewritten. Exit timing, reduced-motion fades and cleanup timers are unchanged.

Read the actual red manifest: source **7266/0** becomes copy **0/0** at mid-exit before the correction. The complete green **7/7** manifest and frozen-head extracted receipts show **7266/0 → 7266/0** for normal, system-reduced and app-reduced motion, with the same visible passage within one pixel, immediate close/focus, retained draft/selection and rapid reopen. The same set retains companion keyboard reopen, divider resize, source-race and original companion lifecycle coverage; there are no recorded runtime/surface errors, unknown commands or overflow. Accepted the supplied typecheck/diff receipts without rerunning them.

The original native WKWebView/compositor/device and final combined acceptance limits remain unchanged. No further correction is requested for this P3.
