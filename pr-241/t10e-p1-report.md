# T10e P1 correction — no unsafe X11 keyed registration

Correction SHA `ddb9dc20059ec5d3ef4a2ec13ddca4cdb09f1a31`. Parent is the separate owned seam test `d56237b71ae0a8298660760dce9786cc3cf3574b`; the P1 diff itself does not contain that test or the duplicated T2/T3a integration commits. Root should cherry d562 and ddb9 independently as coordinated.

Scope: the one confirmed X11 buffered-ungrab P1 from `t10e-review.md`. The review's physical/sync P2 was withdrawn against the accepted ADR; no stored identity semantics were changed. Windows/X11 code remains the platform layout-named convention and settings remain per-machine/not synced.

The stock X11 API cannot guarantee release: global-hotkey0.8.0 queues ungrab.ignore_error with no flush/server acknowledgement; x11rb0.13.2 can buffer it. Repeated unregister could acknowledge the first request only for a stable map; it recomputes keysym→keycode and can still miss the original grab after mapping changes. The public plugin exposes neither the original keycode nor its connection. No double-unregister workaround, private internals or new dependency was shipped.

The correction fails closed at independent boundaries:

- Core capability/validation refuses X11 Record/Open and keyed Cancel/Next. Safe modifier-only Dictate/listening and application commands remain supported. Unknown/future stored settings still round-trip; unsupported saved bindings cannot activate a route.
- Portable planning never creates an X11 keyed registration, including defaults, saved overrides, listening and capture states. The plugin implementation/installation is Windows-only. Linux uses its existing XRecord modifier/focused-capture service without any passive grab.
- Raw routing is not a substitute: tests create hands-free from the actual tracker/gesture, press saved Escape/F5 and full Ctrl+Shift+R/Ctrl+Alt+O combinations, and observe no keyed action and no withholding. Safe bare Next remains functional and readiness remains gated.
- Capture admission checks the same capability before any capture/session/menu activation, including saved keyed bindings. In-app and safe modifier capture remain.
- Both AppInfo.shortcuts and the legacy recording capability expose `x11_keyed_shortcuts_unavailable`; backend EN/NL catalogs localize the same alias. Existing buttons, tray actions and CLI paths remain available.

Removed the now-unused X11 grab projection/variant/refresh code. Windows reservation, modifier handling, layout-named binding semantics and Mac behavior are preserved. The accepted ADR implementation note and docs/desktop.md record the exact gap and requirement for a future owned release transport.

Verification: the new core capability regression failed against the old available capability before the correction. Final focused suites passed: core shortcut14, localization/catalog11, portable6, capture5; the new raw fallback case covers hands-free and safe bare actions. Desktop type/build passed. Core lean and backend clippy --tests with -D warnings, Rust format and diff checks passed with the corrected cache-origin wrapper. Full milestone receipts before this bounded fix remain365 backend/213 Swift; no duplicate full suite or non-Mac build was run in this correction wave.

Affected source: core/src/shortcuts.rs; desktop capabilities, portable projection, registration, capture admission and Linux adapter; deletion of unused alias refresh calls; backend locale catalogs; implementation docs. Parent owns correction-scope-only independent review.

Remaining gates: Windows/Linux compilation/runtime delivery, Windows masking and X11 modifier/capture/layout behavior; real Mac fn/IME/layout/focus/secure-input/withholding/signed-permission acceptance. Keyed X11 support is deliberately unavailable and requires future implementation, rather than an unverified support claim.
