# Desktop UX owner-review evidence

Implementation head: `cab5f89c9179e4f45ca57bb23821bb95a163537f`. Product/native build: `2bf192afc661acfd63618c1487ae271ab1c514c8`; the final commit changes only review fixtures, their tests, the launcher and documentation. A source comparison confirms that core, desktop frontend/backend, dependencies, native services and assets are unchanged since that native build.

## Saved behavior

The launcher supplies neutral navigation, meeting and setup goals with isolated synthetic profiles and scoped reset. Capture/search/recovery simulations now support the real frontend paths, including the primary practice action, separate permission repair, retained results and successive meetings. Cross-window service updates merge with the latest saved main workspace; stale tabs and delayed completions cannot recreate a reset profile. Stored key presence is boolean only.

Shareable transcript input and optional transcript output retain timestamp, canonical speaker attribution and capture source, excluding private notes, contact aliases and unknown legacy provenance. Legacy original speaker labels retain an explicit unknown source.

## Local verification

- Core Release build, 1,075 core tests (seven existing ignored), 17 typing tests, core formatting and strict Clippy passed.
- Desktop build, 407 frontend/fixture tests, 270 backend tests, 61 packaging tests and theme validation passed.
- Broad browser matrix: 1,216 captures. Final affected owner interactions: 20 captures. Final human starting states: 48 captures. Focused minimum/enlarged checks: 20 captures. All cover EN/NL and light/dark, with no reported runtime errors, unknown commands, overflow or locale mismatch.
- The broad matrix precedes the final owner-profile/popup fixes; the affected final flows and starting states were rerun afterward. Do not describe the earlier matrix as an exact final-head rerun.
- Eight same-input before/after export images use the actual frontend and deterministic synthetic IPC. The before backend fixture is from `8ad573f`; after rendering is from the implementation candidate. These are browser UI evidence, not a real provider or participant study.
- Independent review reproduced and verified repairs for reload persistence, cross-tab reset fencing and popup writes preserving newer parent drafts. No material findings remain in the reviewed changes.

## Native verification

The personal Developer ID certificate/team were pinned before signing. Ordinary Pennel Preview and the separate Capture Preview passed strict signature, identifier, certificate and team checks. The original Tauri timestamp mismatch was repaired with the repository's existing Preview helper and the same certificate using `--timestamp=none`; no clock, credentials, trust or ACL settings were changed.

The Preview DMG was created and verified. Its SHA-256 is `aef49a3f54ad8b835c02bfff640aabba7ff7733fa8f4c8a0e0c33ca8f87bb4c2`.

Sixteen native captures passed on Mac16,8 / Apple M4 Pro / macOS 27.0.1: Dictation, pad, History and Settings across EN/NL and light/dark at 760×520. Native/frontend material metadata matched, with no horizontal overflow. Captures use dedicated synthetic data and verified owned app windows. Ordinary Preview was not launched; no real speech, provider generation, insertion or installed-data mutation was performed.

## Remaining acceptance

The owner reported that the navigation task has not been tried. No Wayfinder human decision was claimed or resolved. The navigation, notes-first meeting, setup and final acceptance decisions remain open. Native text zoom is unavailable in the pinned product source; VoiceOver, native menu/physical keyboard, real capture and clipboard acceptance remain open. Participant studies, adoption and iPhone work are separate follow-ups.

Hosted desktop/macOS/Windows/Linux, core and decision checks passed on `2bf192a`. The final review-only commit has its own hosted checks; consult the PR for the latest status. No release or deployment is claimed.
