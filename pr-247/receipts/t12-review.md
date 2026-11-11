# T12 final presentation pass independent review

Reviewed pinned `d1f36579347aa4ae9fec6e390d7bec6c6d85e7b3` → `d7ed60eb4a4db7dae302c8499e57f28417a39326`, `t12-review.diff`, report, constraints and HANDOFF motion/Share requirements. The separate native demo lifecycle correction was excluded. No source/index/HEAD changes, suites/builds/native checks, subagents, CI or publication.

**Spec: the reviewed timing, reduced-motion and Share contracts are implemented. Quality: one small P3 exit-copy fidelity defect; no proven operation, privacy, editor or focus regression.**

## Finding

### [P3] Retain nested scroll offsets in the outgoing companion paint copy

**Location:** `apps/desktop/src/motion.ts:27–32`.

`companionExit()` deep-clones the companion and immediately mounts that copy, but does not transfer the source elements' `scrollTop`/`scrollLeft`. Those live view properties are not DOM attributes copied by `cloneNode(true)`, so the new scrollers start at zero. The companion contains real scrolling surfaces (`styles.css:829,839` for Transcript/Ask and `meeting-workspace.css:159` for Review). Closing one while reading deep content therefore paints its beginning in the outgoing copy for the 130ms exit, or the 120ms reduced-motion fade, rather than fading the view that was just visible. The retained real pane remains correct; this is a visible close discontinuity, not data loss or a logical scroll-ownership failure.

**Smallest fix:** pair the source/copy element trees, capture nonzero scroll offsets, and apply them to the mounted copy before its first paint. Keep the copy inert, ARIA-hidden, sanitized and outside functional selectors; do not defer logical close or move the real pane.

**Focused regression:** start with an actually overflowing Transcript/Ask or Review scroller at a nonzero offset, close, hold the outgoing animation at an intermediate frame and assert that its corresponding shadow-copy scroller/visible passage matches the source offset. Confirm rapid reopen still removes the copy and preserves the actual source position. The current companion regression checks draft/focus/semantics/reopen ownership, but never compares a deep-scrolled outgoing copy.

## Remaining spec/quality assessment

Feedback120ms, switch150ms, move220ms, reveal320ms and the existing single cubic-bezier curve are used for the reviewed transitions. Move exit130ms matches the approximate60% rule. Pill enter220ms/8px, state crossfade120ms and leave140ms remain separate approved roles; model confirmation and backend result lifetimes were not changed. Both system and app reduced-motion routes remove movement/pulsing and retain actual short opacity/color feedback. The pill consumes the existing environment route while retaining its Figtree/font/token stylesheet boundary.

Logical companion close, focus return, primary-column inertness and operation lifetimes remain immediate. The outgoing host is inert/ARIA-hidden, lives outside app selector roots in a shadow tree, strips IDs/roles/live-region references/data hooks, and is removed by reopen/disposal or its presentation cleanup. Notes are not cloned. Direct pointer/keyboard interaction cancels wide pane-size motion. Source/reading restoration and the actual CodeMirror transaction/composition paths are retained.

Motion triggers follow new transcript/caption arrival, non-composing Markdown-line resolution, actual summary content/visibility changes, changed timeline data, committed acceptance/count changes, observed keycaps and primary task changes. Clock/partial refreshes and identical observed keys do not repeatedly replace or animate retained controls. Summary sections keep90ms staggering; actual map spans draw after the section window and unchanged timeline data is retained. Placeholder pulses depend on actual work. No content, policy, provider or credential authority was added to presentation code.

Share uses the approved20px title role and two equal scope segments across the reading width. Inspected NL/dark minimum and EN/light enlarged final captures: privacy explanation, scrollable reading area and footer actions remain reachable without horizontal overflow. No broader visual redesign is requested.

## Evidence and limits

Read the supplied **524 unit/build** and **23 theme/static** results, the complete **30-case frozen-head runtime** manifest and extracted29 timing/keyframe/fade/control receipts, **10** affected behavior cases, and **32+32** before/after manifests with matched fixture evidence. These manifests record no runtime/surface errors, unknown commands or overflow. The actual duration/keyframe/midpoint-fade checks and negative controls substantiate motion separately from static captures; they do not currently exercise the outgoing copy's scroll fidelity. No reviewer reruns were performed.

Actual WKWebView/compositor/native focus, OS menus, screen reader/hardware/physical IME, real capture/provider delivery, signed-device/keyboard/Cloud/store and packaged/release acceptance remain the documented limits. The native demo correction and final combined verification remain parent-owned and outside this delta.

**Correction scope:** outgoing visual-copy scroll offsets and one focused close/reopen regression only.
