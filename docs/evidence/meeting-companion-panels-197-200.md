# Meeting companion clipping and Review status

11 November 2026. Screenshot follow-up for #197 and #200, based on released main `27b75bec05bd78370dab4d8f277bbfae25b06367`.

## Changes

Companion transcript rows reset inherited negative inline margins, retain a flexible speaker column and timestamp column, and wrap long speaker labels. Review presents the consent status once, hides missing connection/model destinations, and shows coverage or an empty result only when that state has meaning. Existing saved-card review, accepted export, source navigation and qualification/consent gates remain.

## Verification

The added browser regressions first failed against the old source: Review duplicated its primary unavailable reason; the 340px transcript pane had `scrollWidth=313` and `clientWidth=307`. Fixed-source geometry checks require an attached visible pane and bound text/timestamp rectangles, including resized splits, narrow overlays and doubled text.

- `npm --prefix apps/desktop test`: 525 passed.
- `npm --prefix apps/desktop run build`: TypeScript and Vite passed.
- `python3 scripts/generate-theme.py --check`: passed.
- Owned Chromium fixture interaction runs: 52 existing Review/consent/card cases plus 28 focused panel cases; all 80 passed across English/Dutch and light/dark. An independent review found a terminal-result → disconnected-model transition that hid the new blocker; the correction adds current-availability precedence and passed another 64 bilingual/theme frames, including completed/failed/stale → unconfigured/unqualified transitions without inference.
- Matching before/after renders: two views × two languages × two appearances = eight pairs. Both use the same synthetic fixture inputs; old source loaded with `--baseline-ref 27b75bec`. Images are published in the PR's `pr-assets` directory, outside code history.

Local receipts are retained under `build/meeting-panels-248/`: `receipt.json`, `review-regressions/manifest.json`, `panel-regressions/manifest.json`, `final-before/manifest.json` and `final-after/manifest.json`. Matched-render fixture SHA-256 is `6a18421d9adf97b53202c285ec48fd64a0e1725a0f677d7dd4b7b69e0e669646`.

These are synthetic renderer checks, not native split/Follow during an actual recording, VoiceOver or provider-quality acceptance. Those original issue criteria remain open. Experimental qualification remains independently tracked under #96; no record was adopted by this change.
