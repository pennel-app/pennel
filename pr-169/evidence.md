# Merged implementation

PR169 merged to main as `fb614b65e7019ef5d0dc063cf316550beb5b5eb9` after all four final-head workflows passed. Reviewed head: `672533113f70cf21b7c9274a758d1a688f8b9cf8`. Code/preparation completion is separate from the still-open owner and device decisions.

# Final PR review evidence

Merge review head: `672533113f70cf21b7c9274a758d1a688f8b9cf8`. Shipping-code Preview build: `87824e4f9575097e16d827b4312e7ff3706c6107`. The later change is confined to an existing `cfg(test)` fixture; the 39,352-byte production prefix is identical and all other files are unchanged. The machine-readable source comparison is retained beside this ledger.

## Review and corrections

Standards: one P2 localization issue fixed. App-owned appended export labels now follow caller EN/NL, while provider messages, source text, canonical names and generated summary bodies remain literal. Rich and legacy actual-request regressions cover both export locales under both saved UI languages. Independent standards and spec re-reviews found no remaining blockers.

Fresh Windows CI passed its default suite, then exposed a pre-existing lean-test fixture race: the original one-shot packet could be dropped before physical admission or on queue contention. The first timeout poisoned the shared test lock, causing eight secondary failures. The failed job remains recorded at its original source/run. A deterministic negative control proved one dropped packet. The test-only correction parks the admitted worker before queue polling, feeds once and proves all samples were accepted, then executes every original configuration-change/no-retry assertion. It adds no packet retry, production change, raised inference deadline or poison recovery. All 33 lean speaker tests and 20 separate fail-fast exact-test processes passed; the full default suite again passed 1,075 tests. Both independent reviewers cleared the delta.

## Current local verification

Core Release build, 1,075 core tests (seven existing ignored), 17 typing tests, formatting and strict Clippy; desktop build and 409 frontend/fixture tests; 270 backend tests; mobile typecheck and 159 UI tests; language-policy/theme checks; 20 focused EN/NL light/dark export captures. All passed. Other earlier browser/native layout coverage below remains tied to its stated revisions.

The refreshed Preview app and DMG passed exact personal certificate/team, identity, deep/strict signature and disk-image checks. Executable SHA256: `81ffb6de66d471a177c88de81e7300bf2851da10cc6932351e2b39c4d7be2bce`. DMG SHA256: `401277a53704852b0c3ceeb440804811be634e82bdd63828d454d546c4a829a9`. The stalled owned signer was interrupted; its temporary component was retained, and the existing same-identity timestamp-free helper completed verification. Earlier apps, final products and receipts were hash-preserved. Ordinary Preview was not launched; no new real-audio, device, menu, VoiceOver, zoom or owner acceptance is implied.

Final-head hosted CI is the merge gate; its status snapshot is maintained separately. Owner UX decisions remain open.

## Historical implementation evidence

The following earlier evidence is retained verbatim at its original source revisions and is not an exact-head native test claim.

# PR review and merge verification

Review head: `87824e4f9575097e16d827b4312e7ff3706c6107`.

## Independent review

Standards found one P2: shareable export labels ignored the caller's interface language. The correction localizes only appended display labels using existing core/catalog helpers and an empty alias list. Canonical provider messages, exact source words, canonical manual names, saved transcript bytes and privacy/freshness checks remain unchanged. English and Dutch caller locales are tested under both saved UI languages. The independent standards and spec re-reviews found no remaining material blockers.

The corrected head passes 1,075 core tests (seven existing ignored), 17 typing tests, core Release build/format/strict Clippy, 409 frontend/fixture tests and frontend build, 270 backend tests, mobile typecheck plus 159 mobile UI tests, language-policy checks and 20 focused EN/NL light/dark export captures. Fresh hosted checks and refreshed personal-team Preview verification are required before merge; their current snapshots/receipt are recorded separately.

## Earlier implementation evidence

The following implementation and capture history remains at its original source revisions. It must not be read as an exact-head native verification of the locale correction.

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
