# PR #185 review and corrections

8 November 2026. Independent standards and specification reviews compared head
`c4a83a818e6827fdd4d30766010605216e716d8a` with the pinned main base
`3272f0abdca5ed8802c50f85850032b43233d2e7`. Both reviewers re-reviewed the
corrections and reported no remaining findings or new regressions.

## Standards

- **P2, owned failure localization:** recovery integrity, unverified-provenance and
  metadata-restoration diagnostics could appear in English in a Dutch interface.
  This violated the [owned-message language policy](../language-policy.md).
  Four exact source/English/Dutch backend mappings now cover these diagnostics.
  Unit and document interaction checks preserve arbitrary provider responses
  literally, including text resembling an owned diagnostic.
- **P3, duplicated code heuristic:** fixture export construction and exact
  regeneration checks were repeated in the evaluation runner. Shared
  `_export_fixture` and `_verify_export` functions now keep those paths aligned,
  preserving deadlines, injected clocks, failure codes and attempt retention.

Both standards findings are resolved. The duplication finding was a judgement
call, not a documented hard violation.

## Spec

- **P2, partial recovery guidance:** an authoritative plan with one retryable
  passage out of two failures could say that no matching audio remained. This
  contradicted the [brief's requirement](../finished-meeting-first-work.md) to
  explain missing audio truthfully. Subset-aware English/Dutch guidance is now
  selected only for `0 < retryable_count < failed_count`. Zero-eligible copy,
  retry gating, counts and the separate full-recording action are unchanged.

The spec finding is resolved. The review found no missing bounded requirement or
unrequested product behavior. Deferred model, ASR, human and device qualification
was not counted as missing implementation.

## Verification

Corrections are in `4c0fefb339f512c3db05acd05240a4c30140ff12` (evaluation) and
`4b2f567416237fe82e7a048551549bd6914d5f78` (recovery copy and regressions).

- Before-fix document assertions reproduced the one-of-two audio contradiction
  and English integrity failure in a Dutch interface. Their failed manifests
  remain retained locally.
- After corrections: **423 frontend tests**, TypeScript/Vite build, **300 Python
  evaluation tests**, **11 Rust localization tests** and two language-policy
  checks pass.
- **24** checked EN/NL light/dark document renders pass, including minimum-size
  partial/error states, fully unavailable copy and the original recovery flow.
- Separate matched before/after mock captures use identical fixture/action input
  at `c4a83a8` and `4b2f567`. Only non-mutating assertions are disabled in both
  screenshot snapshots so the known broken state can be captured; the full
  after interaction assertions run independently. All four original primary
  after images are byte-identical at the corrected source. Images and hashes
  live on [pr-assets](https://github.com/pennel-app/pennel/blob/pr-assets/pr-185/review/manifest.json).
- A fresh evaluation-bridge build reproduces the retained bridge's SHA-256
  `e8e2022ae3d11e65e26e27b83f2c4152964d2aa801a9214446326615780d334c`.
  Production exports remain identical. Offline replay of the retained local
  HTTP 200 response still returns **identity_unavailable**, with no admission.
  No model inference or network request was made by this check.
- The original pilot freeze remains unchanged. Its runner fingerprint stays
  bound to the original source; the refactored runner correctly rejects that
  old fingerprint. The newly generated offline freeze is verified separately.

## Acceptance boundaries

Final-head applicable CI must pass before merge. The earlier personally signed
Mac recovery run remains evidence for its pinned candidate and synthetic speech
fixture; these copy corrections add automated and mock-render evidence only.

Suggestions remain gated and the qualification registry remains unchanged with
zero adopted records. [#96](https://github.com/pennel-app/pennel/issues/96) stays
open for the Decider serving revision/provider identity blocker and remaining
language, quality, calibration, resources, natural-ASR and human/device work.
Closing [#183](https://github.com/pennel-app/pennel/issues/183) after review, CI and
merge completes its bounded recovery fix, not those separate acceptance gates.

## Integration with writing profiles

Main advanced during final CI to
`a60ade30e2e5c837c9069b10385c9e22a2fa282d`. Integration commit
`9b3b22ed62ed60fd893b1b2e525d2f11c3538047` preserves that writing-profile feature
and this meeting work. The two manual merge resolutions retain both glossary
term sets and main's canonical `settings-dictionary-legacy-window` fixture name.

The new optional self-voice metadata defaults to absent. Recovery continues to
preserve complete authoritative utterance metadata. The earlier bridge hash,
freeze and image observations above remain bound to their stated source commits;
they are not relabelled as new inference or acceptance evidence.

The metadata source changed, so complete production export fingerprints must
change and historical exports must be rejected by the new context. Refreshed
offline preparation/replay and combined core, frontend/backend, rendering and
hosted native checks are recorded in the PR before merge. No model trial or
qualification is repeated or granted by this integration.
