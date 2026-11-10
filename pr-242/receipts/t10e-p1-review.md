# T10e scoped X11 release correction review

**P1 addressed by failing closed. No new actionable defect in this correction.**

Reviewed exact pinned `ddb9dc20059ec5d3ef4a2ec13ddca4cdb09f1a31^` → `ddb9dc20059ec5d3ef4a2ec13ddca4cdb09f1a31`, using `t10e-p1-review.diff`, `t10e-p1-report.md` and the T10e supersession note. The separate owned-seam test in parent `d56237b` was excluded from this correction assessment. The withdrawn physical-identity P2 was not reopened; platform-specific identities/per-machine storage remain unchanged.

The original buffered-ungrab path is no longer reachable on X11:

- Core capability/validation refuses Record/Open and keyed listening bindings with `x11_keyed_shortcuts_unavailable`. The row projection also checks raw saved candidates, so unsupported stored overrides cannot present a falsely available effective fallback.
- Portable planning creates no X11 registrations and includes only supported modifier-only listening actions. The registration module and plugin installation are Windows-only; Linux cannot install the unsafe plugin or call its register/unregister path.
- Linux raw XRecord routing cannot substitute a keyed action: the projection omits it and the adapter retains a keyed-action dispatch rejection. The focused hands-free trace covers saved Escape/F5 and full Record/Open chords without actions or withholding. Safe bare listening actions retain their existing path/readiness guard.
- Capture validates the action capability before session/menu/input activation, including saved keyed candidates. Supported modifier-only Dictate/listening and application capture remain within their existing ownership paths. Buttons, tray actions and CLI routes do not depend on a keyed X11 grab.
- Both effective shortcut rows and the legacy recording capability expose the explicit gap. Backend English/Dutch catalogs and their source alias provide the same localized explanation; implementation notes clearly supersede the earlier X11 support claim.

Accepted the recorded focused core 14, catalog/localization 11, portable 6 and capture 5 results plus strict clippy, build and formatting receipts without rerunning them. No source/index/HEAD changes, subagents, builds/tests, native interaction or CI actions were performed.

The P1 is closed through disabled keyed X11 support, not through a claimed fixed release transport. Re-enabling keyed globals still requires an owned, acknowledged release implementation and separate platform evidence. Windows/Linux compilation/delivery and the disclosed modifier/layout/native/device gates remain open.
