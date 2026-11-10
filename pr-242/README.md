# Shortcut recorder and Settings review evidence

PR #242. These are synthetic owned-Chromium headless fixture renders, never user-screen captures. The original manifests and their fixture hashes are retained.

- T10f: 20 matched before + 20 after (English/Dutch, light/dark, wide, minimum, enlarged, custom and cleared Dictate).
- T10g: 12 matched before + 12 after (English/Dutch, light/dark, wide, minimum and enlarged).
- `evidence.json` records the source checkpoints, final reviewed source, immutable PR base/head, image hashes and approved screen references. The matched captures predate the later Cancel/layout/lifecycle corrections; separate final protocol manifests and logs document those checks.
- `receipts/` contains the source and scoped independent-review reports, full milestone backend/frontend/build receipts, and focused correction evidence. The full 375 backend/469 frontend suites preceded the final bounded correction, which has 32 focused tests and four browser cases.

No hardware, secure-input, signed-permission, receiving-app delivery or Windows/Linux build acceptance is established by these fixtures.
