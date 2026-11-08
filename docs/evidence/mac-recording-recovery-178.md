# Mac recording and recovery baseline

[Establish the Mac recording and recovery baseline](https://github.com/pennel-app/pennel/issues/178), 8 November 2026. The [machine receipt](mac-recording-recovery-178.json) records the exact candidate, assertions, request ledger, artifact hashes and outstanding checks.

**The native recording/storage/retry path preserved the test work. The reopened desktop UI did not expose failed-chunk recovery.** This is bounded baseline evidence, not release, speech-quality or human-usability acceptance.

## Candidate and isolation

Used the retained **Pennel Capture Preview**, source `2bf192afc661acfd63618c1487ae271ab1c514c8`, signed with Lander Parren's personal Developer ID team `B538Q2K76C`. Strict/deep signature verification passed. Binary SHA-256: `515758fbcc6f50d57db1a8f1d69e98364f2a5c6fb8115071eb23753dc90e383a`. The retained ZIP/package hash and complete bundle-tree hash are in the receipt.

Against main `a005b8e0d39161aea69d2a30b5638078eef2a46d`, desktop frontend/backend and macOS platform trees are identical; the two changed core capture files differ only inside tests. Other production differences concern shareable-export label localization. Recording/recovery equivalence is supported; full-app/export equivalence is not claimed. No native rebuild or re-sign was needed.

The dedicated capture identity, `PENNEL_WINDOW_CAPTURE=1` and a new absolute `PENNEL_CAPTURE_DATA_DIR` isolated config, library, history and logs. Ordinary Preview and production were not launched. The capture Keychain service is separate; no keys were written. Its WebKit/TCC/Keychain identity remains fixed across capture runs, rather than unique per data directory. Existing global audio defaults were retained.

The owner confirmed a quiet window with other audio paused. Offline system TTS supplied invented meeting material. Real microphone/system taps and AAC archives ran through the native app. A keyless loopback speech fixture supplied synthetic recognition responses and controlled HTTP 503 failures; no browser platform or capture IPC was mocked. Automatic detection, shortcuts, sync, calendar, automatic summaries and consent-copy were disabled in the new profile.

## Observations

| Scenario | Observed result |
| --- | --- |
| Normal capture and Stop | About 36 seconds captured; both tracks retained, one completed transcript row and the note preserved. Stop RPC returned in 0.194 seconds. |
| Controlled speech failure | About 52 seconds captured; two failed system-audio rows and both tracks retained; note preserved. Stop RPC returned in 0.240 seconds. |
| Relaunch before retry | Every meeting-file hash identical. Request count stayed 5 → 5; no automatic resubmission. |
| Partial explicit retry | First chunk committed; second returned a truthful provider error. One completed/one failed row and the note survived another relaunch. |
| Final explicit retry | Exactly one remaining chunk requested. Both rows completed; the successful prefix was not resubmitted. Notes and archive hashes were unchanged; committed recovery WAVs were removed. |
| Final relaunch and playback | Recovery persisted; request count stayed 8 → 8. Native saved-system-track playback and Stop succeeded from the 30-second position; listening quality was not judged. |
| Native UI inspection | Recording notes and recovered rows were visible. After relaunch, Notes showed “Saved meeting” without failure guidance; Transcript showed failed rows without targeted retry. “Files and recovery” had no transcription retry. |

RPC times include the local HTTP/Tauri control roundtrip and are observations from an immediate loopback provider, not an acoustic Stop or performance qualification. The initial failed capture made two physical HTTP attempts per chunk; all four attempts remain in the ledger. The two recovery actions used the real `retranscribe_meeting` command because the UI did not expose that action.

## Reproduced defect and next decision

[Expose failed-chunk transcription recovery after reopening a saved meeting](https://github.com/pennel-app/pennel/issues/183) records the minimal reproduction, owning source and regression expectations. The existing failed-only API has no desktop UI caller; full-recording retranscription is a separate operation. No product fix was made here.

The next Wayfinder decision is [Which bounded changes should make the first Mac workflow ready?](https://github.com/pennel-app/pennel/issues/179). Consider this demonstrated recovery gap alongside the [completed route research](https://github.com/pennel-app/pennel/issues/177#issuecomment-6721546770). Existing capture, review, performance, model and human-acceptance issues retain their criteria.

## Evidence retained and limits

Owned candidate ZIP/app, generated material, isolated profile/audio, request hashes, per-stage reports and the local control scripts remain under ignored `build/mac-recording-recovery-178/20261108T124533Z` and its parent. Public evidence contains metadata and sanitized conclusions only. Native commands used the existing capture driver/evaluation surface; UI observations used the app-scoped computer-use interface. The isolated app and owned loopback service were stopped afterward.

Actual ASR quality, Dutch/mixed speech, two-party calls, speaker/echo quality, permission/device changes, long recordings, delayed-provider stress, disk-failure recovery, VoiceOver/enlargement, human tasks and model qualification were not established. The default Bluetooth route reported echo cancellation inactive; the warning was visible. No release, deployment or other-platform acceptance is implied.
