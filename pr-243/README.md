# Retained inline Notes editor review evidence

PR #243. Synthetic owned-Chromium headless fixture captures; no user screen/profile.

- `before/`: 16 baseline renders from a463cdab.
- `after/`: 16 same-input matched renders plus eight actual editor/deep-finalization interaction frames from the integration milestone.
- `final-scroll-en-light/` and `final-scroll-nl-dark/`: six selected frames from the final second scroll correction.
- `evidence.json`: original source checkpoints, exact paired case hashes, image hashes and accepted screens 16/22. Whole before/after fixture hashes differ because the after set includes extra cases; all sixteen paired scenario hashes match.
- `receipts/`: full 508 frontend/build receipts, implementation and independent-review reports, component result, focused interaction/recovery manifests and both explicit-scroll red/green correction evidence.

The matched visual matrix predates the behavior-only scroll corrections. Final correction frames and manifests are kept separate. Native scripts received syntax checks only; physical IME, native WebKit/menu dispatch, VoiceOver and native performance remain acceptance gates.
