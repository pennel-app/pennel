# iPhone keyboard: bilingual typing, voice and feel

Tracking: [keyboard #39](https://github.com/pennel-app/pennel/issues/39), [device acceptance #18](https://github.com/pennel-app/pennel/issues/18), [signing/review #20](https://github.com/pennel-app/pennel/issues/20). This work builds on the device-verified [launch-crash fix #81](https://github.com/pennel-app/pennel/issues/81). Implementation branch: `codex/iphone-keyboard-experience`.

## Product contract and qualification

Typing and voice have equal product priority. Optimize Dutch, English and mixed sentences while retaining QWERTY/AZERTY and Pennel's existing brand. Ordinary input stays local, personal learning is opt-in, semantic writing actions are explicit, and only the chosen provider receives requested content. No account, model download or Full Access is required for ordinary typing.

The implementation is a candidate for testing, not evidence that Pennel is the best keyboard or meets the proposed 20% completion-time/30% harmful-correction improvement targets. Tactile quality, physical latency, energy, microphone interaction, mixed-language speech quality and broad host acceptance need actual measurements. [Keyboard-only cold/expired start](mobile-keyboard-feasibility.md) remains an open release gate; no app-switching alternative has been accepted as fulfilling #39.

## Implemented subsystems

| Subsystem | Behavior and limits |
| --- | --- |
| Local typing policy | A small internal Rust library under `core/keyboard`, consumed by the main core and the extension-only `PennelTyping` facade. No engine, database, networking, credentials or content logging in this library. |
| Bilingual candidates | Native spelling candidates, supplementary lexicon and bounded personal vocabulary feed local ranking. Typing languages default to Dutch and English, independently of interface language, layout and speech language. |
| Correction | Narrow authored known-typo mappings require current word-boundary/context and host-trait guards. Ambiguous candidates remain suggestions. Original spelling and immediate guarded Undo are retained; rejecting a correction prevents immediate reapplication. Scores are heuristics, not calibrated probabilities. |
| Editing | Native gap-aware multi-touch keys, stable accent selection, spacebar cursor movement, word deletion and symbol sliding. Explicit suggestions require an exact cursor-aligned replacement span. Undo/restoration account for truncated host context and fail closed when the affected text cannot be proven. |
| Learning | Off by default. Bounded words and short word pairs only; no sentence log or automatic Cloud synchronization. Reset and per-word Forget propagate deletion markers to prevent old local records being resurrected. |
| Full Access transitions | Read-only shared preferences are evaluated separately from writable extension-local storage. Local choices/learning remain usable without Full Access; versioned preferences and bounded migration preserve current choices on re-enable. App counts cannot enumerate newer extension-local learning before migration. |
| Emoji and snippets | Local Dutch/English emoji search and recents; snippet creation, editing, deletion and optional delimiter expansion. Snippet opt-in is independent of correction and suggestions. Sensitive/unsuitable host contexts disable assistance. |
| Experimental swipe | Geometry-based bounded decoder using actual key centers. Off by default. Its authored common-word baseline is small; it is not a corpus-trained general-purpose swipe engine. Passing synthetic geometry tests is not quality qualification. |
| Writing actions | Explicit selected-text cleanup, shortening and translation produce a preview. Apply/restore validate the live selection/document; original text remains available for explicit recovery. Limited to 4,096 characters and a configured runnable containing-app service. |
| Apple speech | Existing chosen Apple provider receives bounded user vocabulary. An installed-assets-only live preview shows provisional words; the final selected-provider result remains authoritative and inserts once. No shadow provider requests or preview-triggered model downloads. |
| Speech language | Apple Automatic means the device's preferred language; explicit Dutch/English remain selectable. Separate locale support does not prove code-switching. A newer-transcriber capability probe does not change defaults. |

### Writing-action authorization and storage

Only an explicit action writes selected text to the transient App Group request. The extension never receives provider keys. A prepared core job freezes the selected provider, endpoint, key, native model and prompt; checks the expected configuration before dispatch; participates in configuration/account cancellation; and runs once. Native account/key epochs cover changes before registration. Cancel suppresses late output; an already-issued provider request cannot be recalled, and admission stays serialized until it returns.

Shared request/result operations use non-blocking cooperating cross-process locks and independent per-request cancellation markers. A suspended app cannot hold the keyboard waiting for its lock; atomic availability reads and cancellation do not need that lock. Expired/malformed/oversized data cannot be delivered and is purged when a participating process next runs. This is not a promise of wall-clock deletion while iOS suspends both processes. Selected-text previews do not create History entries. Foreground writing needs no microphone session; background requests require an actually live ready session. A finite OS background task covers an explicit accepted action, never idle preparation or an unrelated background mode.

## Haptic design and comparison

Off is the initial typing default. Existing explicit opt-in maps to Gentle; Crisp is optional. The keyboard owns view-associated generators only while visible, prepares before likely commitment, reuses them during bursts and releases them on disappearance/backgrounding. Preparation is not polled. Settings provide immediate samples, an unsaved practice field, anonymous shuffled A–E comparison and an explicit five-minute real-keyboard trial. A trial never enables an off preference and expires without extending when mirrored.

| Event | Feedback rule |
| --- | --- |
| Character/space/punctuation | One compact confirmation at accepted commitment, independent of predictions |
| Cancelled touch/disabled control | None |
| Held delete | Initial pulse only; no repeat buzzing |
| Discrete accent/cursor choice | Restrained selection feedback |
| Suggestion/snippet/undo | One pulse for the operation, not its output characters |
| Automatic correction | Reversible visual indication |
| Recording/result/error | Acknowledged transition only; no speculative success |
| Background/polling | None |

The current soft `0.4` baseline, the same prepared pulse, light, rigid and off are experiments. Equal UIKit intensities are not equal physical strength, and `.soft` does not establish an ideal typing feel. The app preview's bridge preparation is best effort; a baseline sample means no explicit preparation, not guaranteed cold hardware.

Keep recording suppression intact. No audio-session category or haptic-during-recording policy is changed to force feedback. Compare first-after-idle and rapid typing, one/two thumbs, cases/grips, extended writing, cancelled touches, gestures, Full Access, VoiceOver, actual recording, Bluetooth and interruptions. Software timestamps describe requests; physical onset needs external sensing. A simulator/screenshot cannot establish feel. Core Haptics remains a later experiment only if it earns its additional lifecycle/resource cost.

## Benchmark tooling

`docs/evidence/keyboard-benchmark-corpus.json` contains 90 authored **synthetic** tasks, evenly split among Dutch, English and mixed language. It includes numbers, negation, compounds, names, addresses, quotes and spoken corrections. The offline tool collects nothing from ordinary use and reports aggregates without raw answers or participant identities:

```sh
python3 scripts/keyboard-benchmark.py validate
python3 scripts/keyboard-benchmark.py template --with-ratings > /tmp/pennel-keyboard-trials.csv
python3 scripts/keyboard-benchmark.py summarize /path/to/consented-trials.csv --mode all --compare-apple
```

Use equivalent practice and counterbalanced order. Record the same phone/OS, keyboard version and enabled options. Compare Apple and Raycast on English; Dutch/mixed claims must acknowledge Raycast's currently English-only typing. Anonymous Pennel profile comparisons can be blinded; visually different third-party keyboards cannot. Pairing requires matching participant/device/OS/language and task sets; typing and dictation comparisons remain separate. Optional 1–7 ratings capture immediacy, comfort, annoyance and fatigue; missing observations are not imputed. Report absolute error rates and unpaired coverage rather than manufacturing a percentage win from missing or tiny baselines. The corpus is not a substitute for a user study or semantic speech assessment.

## Generation, builds and acceptance

Rust artifacts remain outside generated native projects. `build-mobile-core-ios.sh` prepares both the main core and narrow typing XCFramework; the pod-visible typing artifact is a generated symlink. Ownership checks require the keyboard's narrow typing pod and continue rejecting Expo, React and the full core/runtime. Clean and repeated non-clean Prebuild and both ARM64 Release products are required.

```sh
env PENNEL_VARIANT=preview make mobile-test
env CI=1 PENNEL_VARIANT=preview make mobile-prebuild
env CI=1 PENNEL_VARIANT=preview npm --prefix apps/mobile run build:ios
env CI=1 PENNEL_VARIANT=preview PENNEL_IOS_DESTINATION='generic/platform=iOS' npm --prefix apps/mobile run build:ios
env CI=1 PENNEL_VARIANT=preview make mobile-native-test IOS_DESTINATION='platform=iOS Simulator,id=<verified-device>'
```

Run large native commands through the personal worktree-storage guard. Core, desktop frontend/backend and affected macOS service checks also apply to vocabulary/public-interface changes. Personal Preview signing only; preserve existing app/keyboard/widget identities and data. The final receipt in `docs/mobile.md` distinguishes code, automated checks, installed candidate and acceptance evidence. The personally signed normal candidate `a089c94` is installed on the test iPhone; its physical correction/undo, switching, reappearance and Full Access checks pass. Haptic feel and comparative quality remain unmeasured.

### Isolated intent-routing investigation

The existing harmless `AudioRecordingIntent` probe can now be included in an isolated Preview build:

```sh
env CI=1 PENNEL_VARIANT=preview PENNEL_KEYBOARD_FEASIBILITY=1 npm --prefix apps/mobile run prebuild
```

The flag registers the probe in app and keyboard compilation and shows a clearly labelled QA button only in the exact Preview keyboard bundle. Production rejects the flag. It writes a process/bundle receipt with `recordingStarted: false`, never requests microphone permission, starts capture or forces an app URL. Test cold/resident/suspended/terminated/force-quit states and inspect the actual route. A successful receipt proves only execution placement. Restore a normal flag-off build afterward. Recording eligibility, repeated restart, user-visible app switching and App Review remain separate gates.

On 31 October 2026, the personally signed `3bb2ad3` diagnostic candidate was tested on iPhone 16 Pro / iOS 27.0.1 over USB. Actual public button invocations ran in the keyboard extension both in Pennel's practice field and in Settings after targeted termination of the containing app. The latter left the main app absent and Settings visible; both receipts report no recording. This specific route did not establish containing-app wakeup or microphone eligibility. It is not a user force-quit test or a general impossibility result. See the [feasibility evidence](mobile-keyboard-feasibility.md#signed-process-routing-result--31-october-2026) and [candidate receipt](evidence/keyboard-experience-2026-10-31.json).

## Primary research references

- [Apple custom keyboards](https://developer.apple.com/documentation/uikit/creating-a-custom-keyboard), [open access](https://developer.apple.com/documentation/uikit/configuring-open-access-for-a-custom-keyboard), [App Review 4.4.1](https://developer.apple.com/app-store/review/guidelines/#extensions).
- [Feedback preparation](https://developer.apple.com/documentation/uikit/uifeedbackgenerator/prepare()), [haptic design](https://developer.apple.com/design/human-interface-guidelines/playing-haptics), [recording policy](https://developer.apple.com/documentation/avfaudio/avaudiosession/allowhapticsandsystemsoundsduringrecording).
- [Speech vocabulary](https://developer.apple.com/documentation/speech/analysiscontext/contextualstrings), [SpeechAnalyzer](https://developer.apple.com/videos/play/wwdc2025/277/).
- [Raycast full-keyboard release](https://www.raycast.com/changelog/ios/1-5-0), [keyboard manual](https://manual.raycast.com/ios/keyboard), [dictation limits](https://manual.raycast.com/ai/dictation). Product documentation establishes advertised capability, not measured quality or an Apple-platform exemption.
