# Redesign issue closure audit — 11 November 2026

The original PR bodies used Part of/Tracks/References, not closing keywords or closing Development links. PR221 and247 had zero closing-issue references at audit start. Keeping every issue open for broad acceptance also left completed software-only tasks open unnecessarily. No repository auto-close setting was changed.

## Completed and closed

| Issue | Merged PR | Why its own criteria are complete |
| --- | --- | --- |
| #191 | #221 | The additive theme/icon/helper/capture foundations, contrast checks and owner title comparison are complete. The owner chose the title sizes; no own device criterion remains. |
| #193 | #225 | The approved 26 px page/onboarding titles, 30 px dictation instruction and 20 px setup heading scope are applied and verified, including corrected EN/NL light/dark minimum/enlarged captures. |
| #205 | #222 | The structured-summary ADR is accepted. The owner explicitly signed off all seven recommendations, and the accepted note is merged. This task contains no runtime/device acceptance requirement. |
| #206 | #235 | Core parsing/validation, revision sidecars, candidate/accept/history/restore/fallback and sync scanning are implemented, tested, reviewed and merged. Its own required verification is complete. |
| #207 | #235 | Generation provenance and mixed/overlap-safe speaker timeline helpers are implemented with round-trip/overlap tests, reviewed and merged. Its own required verification is complete. |
| #209 | #237 | The accepted shortcut ADR, lenient model/validator, legacy derivation/compatibility and mobile type mirror are implemented and verified. Actual native key/permission/withholding acceptance is owned separately by #212–#215. |
| #210 | #237 | Binding-aware keys/tracking, own-key handling, hold/hands-free Cancel and preset compatibility are implemented and tested. This issue explicitly assigns physical-keyboard evidence to #212 and #213. |

All seven issue states were read back CLOSED/completed and roadmap cards Done. The five relevant merged PRs now carry correct Closes references for those seven completed issues. No open native acceptance issue was assigned a closing reference.

## Remaining own acceptance

All software, source reviews, selected CI, merges and Macv0.1.45 delivery are complete. The following issues remain OPEN/In review for their explicit acceptance evidence:

| Issue | Merged PR | Remaining acceptance |
| --- | --- | --- |
| #192 | #227 | Actual Mac VoiceOver and the operating system’s Reduce Motion setting. |
| #194 | #229 | Real dictation into Home practice, actual focus loss, and use while a real meeting is recording. |
| #195 | #228 | Real recording level bars, an actually denied microphone, and the capsule while visiting other views. |
| #196 | #231 | VoiceOver reading order of the continuous meeting document. |
| #197 | #236 | Native split dragging and Follow during a real recording. |
| #198 | #234 | Actual macOS Email and Share service hand-off. |
| #199 | #239 | A real application summary run with a configured model and with no model configured. |
| #200 | #240 | VoiceOver on accepted and undone Review cards. |
| #201 | #246 | Production qualification adoption under #98, a real qualified meeting/provider run, and native VoiceOver/accessibility. The intentionally empty qualification registry is an accepted gate, not missing implementation. |
| #202 | #243 | Physical IME, native WebKit/menu dispatch and VoiceOver, plus execution of native verification and large-note typing/performance measurements. |
| #203 | #230 | The actual Mac permission-repair/System Settings routes. |
| #204 | #233 | A real model download and the first microphone practice. |
| #208 | #238 | VoiceOver interaction with the speaker lanes and meeting map. |
| #211 | #237 | Actual Mac menu accelerators with a non-US keyboard layout. |
| #212 | #237 | Physical fn capture, global shortcuts in other applications, and actual withholding of the bound key from the receiving app. |
| #213 | #241 | Deferred Windows menu masking and X11 modifier/capture/layout runtime evidence; overlapping Mac fn/layout/focus/secure-input/withholding checks remain owned by #212/#214/#215. Existing portable acceptance owners #7/#8/#9 remain separate. |
| #214 | #242 | Real Mac fn/modifier-side/IME/layout recording and receiving-app withholding, signed focus/permissions, secure input and VoiceOver. |
| #215 | #242 | Recording actual keys in the native app across fn/modifier sides/layouts, signed focus/permissions, secure input, receiving-app withholding and VoiceOver. |
| #216 | #226 | Native pill nonactivation/click-through and the real paste key/delivery on the supported installed platforms. |
| #217 | #244 | Real microphone/current-capture model switching, the native picker and hook delivery, signed permissions, receiving-app insertion, focus and secure input on a Mac. |
| #218 | #245 | Actual multi-display/cursor placement, native focus/permissions, VoiceOver and receiving-app/microphone behavior. |
| #219 | #247 | Actual WKWebView/compositor motion and the OS Reduce Motion setting, with the named native VoiceOver/IME/focus and capture/delivery integration evidence. |

Windows/Linux compilation is owner-waived in this continuation; no repeated all-platform build is required. Deferred runtime acceptance is not claimed passed. Approved keyboard-first/click-through pill and X11 keyed fail-closed behavior are completed current scope; future enabling gates are conditional. Generic cross-product caveats do not create missing software work.

#202 notice-copy verification is now complete: source/public and retained final production/dist are byte-identical,14928bytes,SHA256 `981ae28ca158ae6266fcdc9986bc6539e08ac02e80e5758b4ace6c6a5c796a30`, with12fullnotices. Actual IME/WebKit/VoiceOver/performance evidence remains outstanding.

The umbrella #220 keeps seven child boxes checked and stays OPEN/Inreview while22acceptance children remain. The native checks are not inferred from browser fixtures, CI or publishing. Existing software proof: [complete receipt](https://github.com/pennel-app/pennel/blob/a0acf423079be784deecebaad8849cc61e70d4b7/docs/evidence/desktop-redesign-220.md) and [release proof](https://github.com/pennel-app/pennel/blob/pr-assets/pr-247/receipts/mac-release-delivery.md). No application, workflow, release, branch, stash or user-device state changed during this audit.
