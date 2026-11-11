# Final integration correction — independent meeting policies

Sole final native/core review finding (P2) corrected locally. This report is the implementer's bounded correction receipt; the parent owns scoped re-review and user-worktree handoff. It is not a full-goal, merge, hardware or release acceptance claim.

- Base: `5757cc2eba55b544f462574510fcbcb3f084bd97`.
- Head: `b10c122eba41e1b083304a43c85f48b172910ecd` (`fix(desktop): keep meeting policy saves independent`).
- Branch: `codex/redesign-integrated`.
- Exclusive source worktree: `/Users/lander.parren/Developer/pennel-app/pennel/.claude/worktrees/agent-a0097254ec71636e7`.
- Review package: `final-fix-review.diff` (SHA-256 `054d594536ba90c5c48ddfa640ddd99536eaa1eb742e067e8374943efcc61afe`).
- Read first: `final-native-review.md`, `final-frontend-review.md`, then `constraints.md` and `accepted-suggestions.md`. The final frontend review had no actionable finding. No frontend change was made.

## Exact correction and ownership boundary

`apps/desktop/src-tauri/src/commands.rs:290` remains the production route reached by `save_settings → apply_settings → change_settings → apply_settings_locked`. The existing serialized settings gate remains in `change_settings`. `apply_settings_locked` now delegates its durable save/guard phase to `persist_settings_and_publish` (`commands.rs:330`) and supplies its existing native publication and final verification/info effects as callbacks. This is the one production seam, not a second settings implementation.

The behavioral fix is `commands.rs:346–353`: copy the current Settings, replace only `summary_automation`, `suggestion_automation` and `suggestion_language`, and compare that copy to the requested Settings. Only an actual edit confined to those preferences bypasses the broad `inference::mutation` guard. Core's existing policy revocation and cancellation counters therefore own the affected automatic job. A mixed policy/configuration update still opens the broad guard. Every other settings edit and no-op retains the old broad-mutation behavior, keeping this correction narrow.

The native publication callback still runs while the broad mutation is held; the mutation is released before verification refresh and AppInfo. The shortcut edit guard remains alive through both callbacks. Preview/library protections, recording exclusion, cloud-sync ownership, speaker-evidence revocation, durable Core settings update, shortcut/menu/appearance refresh and settings notification retain their production order and behavior. The pure previous-shortcut calculation now uses the retained old Settings inside publication. Dedicated credential/provider writers and `inference.rs` remain unchanged.

Only these backend source/test files changed:

- `apps/desktop/src-tauri/src/commands.rs` — narrow guard classification plus the shared native-effects seam and test module declaration.
- `apps/desktop/src-tauri/src/commands_settings_tests.rs` — six regression/guard tests using real Core settings, persistence, global epochs, automatic-summary attempt handling and registered cancellation tokens.

No Core, frontend, branding, dependency, qualification registry or public API change. Simplification: Core remains the sole policy engine; the shell only recognizes the three independent preference fields and retains its existing handling for all other settings.

## Regression evidence

The tests use the real production durable settings helper, replacing only AppHandle-dependent publication/refresh effects. Tauri's native AppHandle/window runtime is not constructed by this harness. It pauses actual automatic Summary work at the public production SystemLanguageModel adapter callback, saves Settings while Core's automatic token and policy are registered, then releases the same admitted candidate and checks its committed Markdown.

Covered behaviors:

1. Changing Suggestions from Automatic to Manual preserves a running automatic Summary, its global inference epoch and its original saved candidate.
2. Changing Suggestions fallback language from English to Dutch independently preserves the same Summary behavior.
3. Revoking Summary itself cancels the active automatic Summary without a global epoch change, writes no Summary, and retains the consumed automatic attempt after re-enabling Automatic.
4. Provider endpoint changes, model changes, real inference timeout changes, and a mixed Suggestions-policy/inference-config change still cancel the active Summary, advance the global epoch and write no Summary.
5. A real `PennelEngine::decide` request reaches a controlled local HTTP provider with an actually registered Core token; saving the independent Summary policy preserves its token and epoch and returns the original answer. This exercises the reverse global-invalidation context. It does **not** fabricate an admitted Suggestion: the production qualification registry is empty, its gate is untouched, and no test-only Core qualification API or bypass was introduced. Core's previously verified per-Suggestion revocation behavior remains unchanged.
6. Real new Core inference registrations see cancellation while a configuration save is still publishing native effects, and admission reopens before verification refresh. This protects the broad guard's lifetime through the production seam.

Before the behavioral fix, these regressions produced 4 expected failures: both unrelated Suggestions updates returned `Err(Cancelled)` from automatic Summary; the reverse decision token was cancelled; and own-policy revoke unnecessarily advanced the global epoch. The provider/model/config controls and native publication admission guard already passed. After the conditional fix all 6 pass.

## Exact commands, results and logs

All Cargo commands below ran from `/Users/lander.parren/Developer/pennel-app/pennel/.claude/worktrees/agent-a0097254ec71636e7/apps/desktop/src-tauri` on macOS. The existing shared wrapper acquired the backend cache lease and supplied its existing host SDK/deployment target, `CARGO_BUILD_JOBS=3` and `CARGO_INCREMENTAL=0`. It was not modified. No cache/source/evidence deletion occurred.

| Check | Exact command | Result | Log |
| --- | --- | --- | --- |
| Regression red, before conditional guard | `python3 /Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py backend cargo test --bin pennel commands::settings_inference_tests -- --test-threads=1` | exit 101; 2 passed, 4 expected failures; 401 filtered | `final-fix-red.log` |
| Final-source focused green | `python3 /Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py backend cargo test --bin pennel commands::settings_inference_tests -- --test-threads=1` | exit 0; 6 passed, 0 failed; 401 filtered | `final-fix-green.log` |
| Final-source full backend | `python3 /Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py backend cargo test` | exit 0; **407 passed, 0 failed, 0 ignored** | `final-fix-backend.log` |
| Final-source strict production bin/tests analysis | `python3 /Users/lander.parren/Developer/pennel-app/pennel/.omx/plans/full-redesign/with-cache-lock.py backend cargo clippy --bin pennel --tests -- -D warnings` | exit 0; no warnings | `final-fix-clippy.log` |
| Final-source formatting | `cargo fmt -- --check` | exit 0 | `final-fix-fmt.log` |
| Source whitespace | `git diff --check` from worktree root; also `git diff --cached --check` before commit | exit 0 | `final-fix-diff-check.log`; cached command tool receipt |

Intermediate diagnostics are retained rather than omitted:

- `final-fix-fixture-check.log`: the first green attempt had 4 pass/1 failure because a test's timeout assignment matched the real default, making the intended mixed edit policy-only. The fixture was strengthened to increment timeout and use a real configured model. The six-case red proof above was then rerun against the old unconditional guard before restoring the fix.
- `final-fix-initial-clippy.log`: one `collapsible_if` warning in the new test HTTP header parser. It was corrected with the idiomatic let-chain and an explicit incomplete-header failure. Final full/focused tests and strict analysis all use the corrected source.

The commit hook also reported no staged secrets. No production source change occurred after the successful final-source verification; commit added only these two verified files.

## Limits and final handoff state

No full Core retest was repeated: Core is unchanged and its final 1295/7-ignored plus keyboard/release/clippy evidence is parent-owned. No frontend/Swift/mobile or non-Mac build was repeated; there was no affected source in those lanes. No CI waiting, push, release, branch switch/merge, tracker operation, assets publication or subagent work occurred.

The harness establishes the real backend save/token/policy boundary and synthetic provider completion. It does not establish native AppHandle UI effects, actual Apple model/device/provider behavior or admitted production Suggestions under the empty registry. Existing physical-input, microphone/paste/focus, accessibility, signed-device/Cloud/store/release acceptance limits remain with the parent and existing acceptance work.

After commit, `git status --porcelain=v1` is empty. Sequencer, MERGE_HEAD, CHERRY_PICK_HEAD, REVERT_HEAD, rebase-merge and rebase-apply are all absent. Exclusive Git/source mutation hold is released with this report. Parent can perform the one scoped re-review of `5757cc2eba55b544f462574510fcbcb3f084bd97..b10c122eba41e1b083304a43c85f48b172910ecd` using `final-fix-review.diff`, then integrate the final handoff.
