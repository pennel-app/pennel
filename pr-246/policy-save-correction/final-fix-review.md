# Scoped review — final independent-policy correction

Reviewed immutable `5757cc2eba55b544f462574510fcbcb3f084bd97` → `b10c122eba41e1b083304a43c85f48b172910ecd`, using `final-fix-review.diff`, `final-fix-report.md`, pinned production source and supplied logs. The scope is the sole P2 from `final-native-review.md` and regressions introduced by this two-file correction. No broader audit, source/index/HEAD edit, subagent, build/test rerun, CI or external publication was performed.

**Original P2: ADDRESSED. New actionable correction-scope findings: 0. Final scoped spec verdict: pass. Final scoped quality verdict: pass. Ready for parent handoff.** This is a correction/source verdict, not a full-goal, hardware, merge or release acceptance claim.

The actual desktop `save_settings → apply_settings → change_settings → apply_settings_locked` route now reaches the shared `persist_settings_and_publish` helper. Its comparison replaces only `summary_automation`, `suggestion_automation` and `suggestion_language` in a copy of the current Settings and requires an actual change before skipping the global inference guard. Consequently, policy-only saves and the Suggestions fallback-language path retain unrelated registered work and its global epoch. Core's existing policy-specific revocation remains authoritative for the automatic job that owns the changed policy.

Any additional changed setting makes the comparison fail, retaining the previous broad mutation path for provider/model/inference configuration changes and mixed updates. Other settings and no-op saves deliberately retain their previous behavior. Dedicated provider/credential writers and the core policy/qualification/admission/marker implementation are unchanged.

The helper extraction preserves the production lifecycle: shortcut-edit admission remains live; preview/library/recording protection and durable settings update occur before publication; speaker-evidence revocation retains its position; shortcut/tray/menu/appearance updates and the settings event run while the broad guard is held; that guard releases before verification refresh and AppInfo. The retained old Settings supplies the same previous-binding calculation. Error returns drop the guards as before. No new global policy owner, public API, dependency or qualification bypass was added.

The regression seam is used by production and replaces only AppHandle-dependent publication/refresh callbacks in tests. Controlled automatic Summary cases exercise the real admitted provider callback, durable save, policy cancellation and consumed marker: unrelated Suggestions policy and fallback edits preserve and commit the original candidate; own-policy revocation cancels without a global epoch change and cannot automatically retry; provider/model/timeout/mixed controls still cancel and advance the global epoch. The reverse test uses a real registered Core decision reaching a local HTTP transport, proving that Summary policy no longer invokes the global invalidation path. It accurately does not claim an admitted Suggestion under the empty production qualification registry. A further real registration test verifies broad admission remains closed throughout native publication and reopens before verification refresh.

Inspected supplied verification, without rerunning it:

- `final-fix-green.log`: **6 passed, 0 failed**, including fallback language, own-policy revocation, provider/model/configuration controls and publication lifetime.
- `final-fix-backend.log`: **407 passed, 0 failed, 0 ignored**.
- `final-fix-clippy.log`: strict `cargo clippy --bin pennel --tests -- -D warnings` completed successfully from the assigned source checkout.
- Reported red proof: **4 expected failures** under the original unconditional guard; fmt and source/cached diff checks exited successfully. The disclosed intermediate fixture and test-lint corrections do not change the final production scope.

Core/frontend/Swift/mobile source is unchanged by this correction, so their earlier final evidence is retained without repeat runs. Native AppHandle effects, actual Apple/provider/device behavior, physical input/focus/withholding, microphone/paste, signed-device/Cloud/store acceptance and production qualified Suggestion execution remain unverified by this source pass, as already disclosed. No additional correction is requested.
