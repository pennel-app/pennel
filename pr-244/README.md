# Current-capture model switching review evidence

PR #244. Synthetic owned-Chromium headless fixtures, never user-screen/profile captures.

- `before/` has eight baseline frames from exact source 06fdd57.
- `matched-milestone-after/` preserves the earlier supplied static milestone with eight exact scenario/viewport matches plus four lifecycle frames. Its original provenance predates finishing/correction commits; it is not described as final-source rendering.
- `verified-after/` has sixteen later presentation/interaction frames. Only the four same-named pill cases match the baseline; changed Settings viewport/interaction cases are separate evidence.
- `settings-final/` and `live-hints/` preserve later label/native-readiness checks.
- `correction-protocol/` contains eight scoped session/recovery cases captured in a working tree that also contained T11c. They are protocol evidence, not acceptance of B appearance/geometry.
- Original manifests, eight pair hashes, source checkpoints and file hashes are in `evidence.json`. Whole fixture hashes are not used as a substitute for scenario/viewport matching.
- `receipts/` retains the implementation/review reports and existing core/backend/frontend/build/static/focused logs. No new broad suite or native run was performed.

Current-capture/final-pass/identity behavior is supported by real core provider tests, while microphone delivery, receiving-app insertion, signed focus/native picker/hook acceptance remain open. The pill stays keyboard-only and click-through. X11 keyed globals remain unavailable.
