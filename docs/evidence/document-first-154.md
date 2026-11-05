# Document-first desktop refinement — 4 November 2026

Implementation owner: [Put the desktop document first](https://github.com/pennel-app/pennel/issues/154). This follows the merged two-task workspace at `b4c52d7`, preserving its Glaze identity and existing core interfaces. The [Wayfinder map](https://github.com/pennel-app/pennel/issues/142) and its human prototype decisions remain open.

## Resulting behavior

- Summary privacy/processing detail starts collapsed. Missing setup, stale sources, save failures and recovery remain visible. The compact unanswered automation reminder opens the explicit choice; opening it never saves settings or starts inference.
- More opens one selected secondary tool. Back returns to the same document, selection and reading position. Read results, action errors and pending status remain owned by their originating tool and operation, including across Settings detours.
- Export review defaults to safely rendered Markdown, with an explicit Markdown source view and reachable actions. Both views use the same prepared output that Copy/Save receives; changing presentation does not generate or prepare another artifact. Personal/shareable scope and provenance gates are unchanged.
- Questions keeps its meeting/library scope beside the composer. Three optional starters prepare durable editable drafts without sending. Canonical question text follows the configured content/speech language with English fallback; interface labels never translate a draft or control prompt language.
- Dictation promotes setup while blocked, shortens ready-state instructions and keeps processing details available. Recent results have direct Copy. Refreshes retain known rows/focus, show loading failures separately and clear them on recovery. History retains existing recovery behavior.

No new dependency, core API, storage schema, provider route, calendar integration, inference policy or mobile UI was introduced. Shared locale additions were checked with the mobile workspace.

## Local verification

| Check | Result |
| --- | --- |
| Desktop lockfile install, production build and unit suite | Passed; 332 tests |
| Core Release build and tests | Passed; 1,066 tests, seven existing ignored |
| Typing tests | Passed; 17 tests |
| Desktop backend | Passed; 270 tests |
| Core formatting and strict Clippy | Passed |
| Mobile TypeScript and UI tests | Passed; 158 tests |
| Synthetic platform fixtures and theme reproducibility | Passed; 54 fixture tests |
| Impeccable source scan on affected UI modules/styles | No findings |

Commands: desktop `npm ci`, `npm test`, `npm run build`; root `make core`, `make test`, `cargo fmt --manifest-path core/Cargo.toml -- --check`, `cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings`, and `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`; mobile `npm run typecheck` and `npm run test:ui`; root `node --test scripts/fixtures/desktop-platform.test.mjs` and `python3 scripts/generate-theme.py --check`. Rust commands reused the existing bundled toolchain through task-local environment variables. The Release build used the worktree-storage guard; no global target/debug configuration was changed.

The full browser command is run once per English/Dutch and light/dark variant:

```sh
node scripts/capture-desktop-reference.mjs \
  --locale <en|nl> --appearance <light|dark> \
  --output build/document-first/complete-<locale>-<appearance>
```

The matrix contains 291 scenarios, including the 19 added document-first cases. Per-run manifests and logs are retained under ignored `build/document-first/`; the PR records the final complete-matrix result and hosted checks. Before/after PR images use the same deterministic synthetic inputs and the actual frontend with a mock backend.

Focused interactions cover explicit automation consent, visible failed-save recovery, unchanged drafts/selection, tool-switch read races, action errors and held success/failure across Settings, clipboard failure/retry and history-read recovery, prompt language independent of UI locale, no-send starters, explicit single submission with citations, prepared-output immutability and stale preparation. Export tests include literal active-HTML payloads and private-content sentinels.

Minimum and enlarged Questions/export checks use actual clipping-ancestor bounds, keyboard focus where applicable and center hit-testing. At 200% text, export deliberately permits scrolling: content and each action are reachable, but are not all simultaneously visible.

## Acceptance boundaries

These checks and captures establish synthetic browser behavior, not human task completion, real audio, physical clipboard/insertion, native menus, VoiceOver or installed-platform acceptance. No production deployment, release, signed-device test or new native-window run was performed for this refinement. Existing device/accessibility owners and the [meeting](https://github.com/pennel-app/pennel/issues/145), [navigation](https://github.com/pennel-app/pennel/issues/144), [setup](https://github.com/pennel-app/pennel/issues/146) and [human release](https://github.com/pennel-app/pennel/issues/147) decisions retain those requirements.
