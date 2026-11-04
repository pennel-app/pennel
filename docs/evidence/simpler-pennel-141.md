# Simpler Pennel implementation — 4 November 2026

Implementation: [Simplify Pennel into Dictation and Meetings](https://github.com/pennel-app/pennel/issues/141). Decision map: [Make Pennel simple from first use to daily work](https://github.com/pennel-app/pennel/issues/142). This records a review candidate, not a production release or completed human study. Exact local counts and source fingerprint are in the [JSON receipt](simpler-pennel-141.json); hosted checks are tracked separately in the pull request.

## Resulting behavior

Desktop has two primary task spaces, contextual search, History/pad under Dictation and five Settings groups. Recording opens a quiet notes editor; Transcript and Questions are optional companions. The same meeting continues through Stop, processing, readable Summary, editing and reopening. More retains secondary tools. Settings detours, source returns and older-message pagination preserve their reading context. An already active new recording opens promptly while an old failed note draft remains recoverable.

Summary automation requires an explicit Unanswered / Manual / Automatic choice; a saved key does not enable it. Owner summaries may use typed notes and transcript. Exact Markdown edits, history and source revision checks protect newer writing, including concurrent sync. Automatic attempts are durably admitted once per local meeting/source revision, cancel with consent changes and do not implicitly retry failed inference. Final capture-owned notes are retained natively after audio quiesces.

Shareable summaries use only permitted finalized transcript sources and a standard safe template, never personal notes, private contact annotations, personal chat or private custom instructions. Legacy owner summaries never become shareable by being hidden, edited or marked reviewed. Copy/export previews exact prepared output; intentional personal export remains separate and may contain private material. Manual source review needs no provider.

iPhone retains its warm branding and three native tabs, with Questions inside Meetings, task-based Settings, truthful audio/cleanup routes and recording return controls. Review sheets dismiss before returning to the active recording. Keyboard, capture and final note ownership remain native; Android implementation stays gated.

## Local verification

| Check | Result |
| --- | --- |
| Rust Release core, formatting, strict default/lean Clippy | Passed |
| Core / typing | 1,066 passed, seven existing ignored / 17 passed |
| Desktop frontend / backend | 330 passed and production build / 270 passed |
| Synthetic fixture / packaging / theme checks | 54 passed / 61 passed / reproducibility passed |
| Owned browser matrix | 272 scenarios × EN/NL × light/dark: 1,088 captures, zero runtime errors, unknown commands or overflow |
| Mac services / MobileKit | 146 passed / 116 passed |
| Mobile TypeScript / UI / plugins / widgets | Passed / 158 passed / 17 passed / seven passed |
| Clean and repeated CNG / actual pod ownership | Passed / six checks passed |
| Hosted native runtime / keyboard | 144 passed / 36 passed |
| Release simulator and unsigned device builds | Passed; these are not signed-device acceptance |

Commands include `make core`, `make test`, core `cargo fmt -- --check` and `cargo clippy --lib --tests -- -D warnings`, desktop `npm test` / `npm run build` and backend `cargo test`, both Swift package tests, mobile type/UI/plugin/widget checks, `make mobile-prebuild`, `make mobile-native-test` on the separate iOS 27 test simulator, and simulator/device `npm run build:ios`. The existing bundled Rust toolchain was reused; large native commands ran through the worktree-storage guard. No dependency was added.

Retained logs/manifests are under ignored `build/simpler-pennel/`. The full accepted browser manifests are `complete-{en,nl}-{light,dark}/manifest.json`. Failures found during development were repaired without removing their draft, source, focus, navigation or privacy assertions. Additional focused regressions cover capture-local setup disposal during a held Apply, Settings-origin isolation, old-draft/new-capture isolation, summary source races and private-output sentinels.

## Visual and native evidence

Baseline browser images in `before/` and the complete matrix use the same deterministic synthetic input. They render the actual frontend with a mock backend. Separate native Mac captures use the packaged Tauri frontend, actual window chrome and Liquid Glass over an owned synthetic backdrop. Only the capture application's verified window IDs were captured. Capture/provider IPC is synthetic; no microphone or system audio source was opened.

All 24 owned Mac captures passed locale, appearance and width checks. The app and reused window helper belong to personal Apple team **B538Q2K76C**. Timestamped signing encountered a clock mismatch; the preview was signed explicitly with the personal Developer ID and `--timestamp=none`, then passed strict verification. No work certificate, notarization or release deployment was used.

iPhone captures use the assigned iPhone 17 / iOS 26.5 simulator and a separate Preview QA profile and credential service. Before images embed `origin/main` JavaScript in the same native candidate host as the after images: a UI comparison with identical invented meeting data, not acceptance of the old native runtime. Notes/transcript and displayed summary fingerprints remained unchanged after explicit owner review; no shareable artifact was created. The default shareable preview refused to reuse that reviewed personal summary, while an explicit personal preview remained available. No copy, share, inference or audio capture was requested in this native UI check.

The Impeccable detector returned no findings on the inspected changed desktop targets. Independent native default agents performed the finish and documentation roles because named skill roles are not advertised in this environment. The finish reviewer checked 13 before/after images; its sole navigation-hierarchy finding was repaired and scored resolved in the two corresponding views. That verdict is scoped visual evidence, not whole-product or human acceptance. Existing Glaze tokens and mobile identity were preserved; no new raster assets or design-system regeneration were required.

## Acceptance still open

Keep implementation and the existing [summary editing](https://github.com/pennel-app/pennel/issues/111) and [private output](https://github.com/pennel-app/pennel/issues/117) owners open while review/merge and their remaining acceptance are pending. The [navigation prototype](https://github.com/pennel-app/pennel/issues/144), [meeting prototype](https://github.com/pennel-app/pennel/issues/145), [setup prototype](https://github.com/pennel-app/pennel/issues/146) and [human release decision](https://github.com/pennel-app/pennel/issues/147) cannot be resolved by these automated results.

Real calls, audible hardware Stop latency, physical keyboard/IME and VoiceOver, installed Windows/Linux, signed iPhone upgrades, keyboard cold start and mixed-version sync still require their owning acceptance evidence. Preparing and building this candidate completes none of the participant rounds in the [study protocol](../ux-adoption.md). Older clients can read personal Markdown but do not enforce the new automation and output policy; mixed-version behavior must not be inferred from a successful build.
