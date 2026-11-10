# Speaker quality and automatic Teams names — #22 / #101

PR #190 builds on merged PR #189 at `6dd72cc`. The completed 9 November phase, retained at `a511725`, validates two saved Flemish Teams recordings and the component/replay behavior below. A 10 November follow-up tests real capture-to-name behavior. The PR is draft again until that evidence and final review/checks pass; earlier component results do not qualify the new live path. #22 and #101 stay open, with #100 owning broader manual-correction work.

## 10 November: real recording-to-name acceptance

**Current checkpoint: v7 positive and cold reopen qualify; current-head live acceptance is blocked and remains open.** The fifth positive attempt passes both the canonical evaluator and the independent PCM supplement. It produces two expected automatic names with 29 valid SystemAudio proofs. Its same-binary/profile cold reopen also passes. A subsequent failed negative startup reveals a wide clock probe; the bounded reprobe fix is built and signed as v8. The final production head is now `0af18c3`, with additional reviewed fixes. Its v9 build/sign and local source checks are complete, but it has not been launched and still requires a fresh positive, actual scope-loss negative and cold reopen before this follow-up is complete. Started/completed failed or diagnostic attempts are not counted as qualified sessions. The first actual detector-prompt start reaches the production answer handler but fails: the echo path reports OSStatus `-10875`, and raw fallback reports no usable microphone/input format. Microphone permission and an available capability did not prove successful recording startup. The failed attempt is retained and is not counted as a positive session. After the microphone fix checkpoint (192 Swift tests), a further actual run starts microphone capture but has no valid system tap from the start: `system_audio=false` and the source guard is invalid. It is manually stopped before guests/public speech are admitted. This is another failed source-gate attempt, not a positive or scope-loss-negative test: no valid app-scoped→global transition occurred. No permission cause is inferred from the absence of a permission sheet. Those failed attempts remain separate from the later qualified controlled positive; their failures are not relabeled as successful positive or negative evidence.

A later actual capture also remains failed: application observation breaks during the call, and the observed natural-leave stop takes 31.9 seconds, above the ten-second gate. These failures are retained separately from the earlier missing-system-tap attempt. Follow-up native checks reach 198 passing Swift tests. The signed candidate then completes the controlled positive below; v8 checks passed at that checkpoint. The later current-head checks and static reviews below are complete, but fresh final-source live runs and selected PR checks remain outstanding.

The current live test is an English Microsoft Teams 2 desktop host with public personal-Teams guest workflow on macOS 27.0.1 and the current built-in microphone; AirPods are not present. The original two AirPods recordings remain separate acoustic/replay evidence. No original work-call accessibility context, enterprise layout, other Teams UI locale or fresh AirPods capture is inferred from this QA setup.

The live profile starts empty. Historical recordings, reference aliases, voice enrollment and authored/model-derived speaking timelines are not imported. The product must create the recording UID, audio, local ASR rows, canonical turns, real Teams observations and automatic proofs itself. Read-only monitors copy actual output rather than supply any of those artifacts.

| Acceptance stage | Required evidence | Current checkpoint |
| --- | --- | --- |
| Positive start and normal end | Real Teams host-alone detection and prompt, actual start/session/UID transition, guests joining afterward, native app-scoped audio, final ASR/diarization/names, then natural Teams hangup that preserves earlier valid evidence | v7 positive qualified, including 0.940 s natural stop; fresh final-source run required |
| Clock and source integrity | Native PCM origin aligned through a measured RPC bracket; actual `audio_clock_verified` and uncertainty at most 100 ms; native capture scope/process recorded separately from the physical SystemAudio track | v7 positive verifies 55.952 ms; a later negative-start probe at 123.087 ms correctly fails the 100 ms bound |
| Independent accuracy | First solo phase per voice calibrates one fixed mapping; later solo phases are held out, with three-second transition guards. Ground truth comes from controlled mute/TTS actions and fixed public voice inputs, never from observed highlights or inferred names | Positive scored from actual control UTC/RPC bounds, calibrated once and held out |
| Automatic-name output | Zero wrong automatic names, canonical coverage reported with its limits, plus independent PCM-energy coverage described below; valid recording/process/window/source proofs, no microphone-derived automatic proof, no false naming of Mixed/Overlap/unknown text | Positive canonical and independent PCM gates passed; negative remains separate |
| Actual scope-loss negative | A fresh recording's native source metadata demonstrates an app-scoped→global transition; post-transition SystemPCM energy and successful SystemAudio ASR demonstrate continued capture, while names/proofs remain absent and the guard remains invalid | Not qualified; a stored unavailable reason alone will not count |
| Finalization and cold reopen | Speaker postprocessing completes after capture stops; proofs, names and policy remain valid after a cold product reopen without enrollment or source/evidence repair | v7 same-binary/profile reopen passes; final-source reopen remains required |
| Isolation and final checks | Owned capture hotkey remains off across detector polls and recording; ordinary apps/profiles remain untouched; final-source tests, two review axes and selected PR checks pass | Current-head local checks/static reviews pass; final-source runtime and linked PR checks remain open |

A supplementary PCM protocol is fixed before any qualifying public-speech run: **16 kHz mono Float32, 20 ms RMS frames at both −45 and −55 dBFS, three-second phase-edge guards, and at least ten seconds of energy-active audio per solo phase at each threshold**. The held-out denominator includes every such energy-active frame, including time with missing ASR or diarization. Correct-name credit requires a valid automatically named SystemAudio transcript interval plus matching clean single-voice source-turn support. Both thresholds must reach at least 80% correct-name coverage per actor and overall, with zero wrong automatic labels across all controlled solo phases. Row-label-only coverage is reported separately. Names are compared directly with the declared actor; an A1/B1-only anonymous mapping is diagnostic and never refitted on held-out data. Seventeen semantic negative-control tests pass for this private supplement. Its first scored qualifying real run passes both fixed-threshold gates. RMS is an energy proxy, not human speech annotation, whole-wave DER, VAD recall or WER.

A control-timing correction is recorded **during capture but before name outcomes/postprocessing**: actual CUA switch-to-mark IPC gaps reach 17.53 seconds, so delayed marks cannot define the preceding phase end. Both evaluators now share one parser using actual control-before/control-after UTC brackets and conservative RPC wall/runtime epoch bounds. A1/B1 use fixed first-30-second interiors; their unrecorded actual end timestamps are explicitly unknown. A2 keeps its conservative marked start and uses the next actual switch for its end; later held-out boundaries use the new control receipts. Raw ledgers and the original protocol are preserved. RMS thresholds, frame size, coverage/error gates and three-second guards are unchanged; no timing or thresholds are fitted to naming outcomes. The qualified fresh positive records actual control brackets from its first phase and validates all six phase bounds before name outcomes; it does not rely on invented old transition times.

### Qualified controlled positive

The actual signed app creates its own recording UID, both physical audio files, local ASR, canonical turns, real Teams samples and final automatic-name provenance. No historical audio, model-derived observations, reference alias, manual correction or voice enrollment is imported. The tested binary is `f576cc95…`, signed with personal team `B538Q2K76C`; its 439 source inputs are pinned separately from the preceding replay candidate.

| Positive result | Observed value |
| --- | ---: |
| Saved SystemAudio PCM duration | 639.331 s |
| Successful nonempty ASR rows / failures | 36 / 0 |
| Automatic names / valid Single-SystemAudio proof rows | 2 / 29 |
| Actual Teams evidence samples / available | 1,259 / 1,240 |
| Maximum native-clock uncertainty | 55.952 ms |
| Natural leave→capture-off upper latency | 939.517 ms |
| Named eligible canonical speech / all canonical detected speech | 100% / 96.84% |
| Strict held-out PCM-energy coverage at −45 dBFS | 95.58% overall; first actor 91.46%, second 99.60% |
| Strict held-out PCM-energy coverage at −55 dBFS | 95.19% overall; first actor 90.78%, second 99.59% |
| Wrong automatic-name time across all controlled solo energy frames | 0 ms |
| Automatically named Single-text words | 799 / 912 (87.61%) |

Both actors exceed 80% independently at both fixed thresholds. Missing ASR/clean-turn support remains in the PCM denominator and is unnamed. Canonical held-out confusion is zero on its separately stated detected-turn denominator. Two Mixed, four Overlap and one Unknown transcript rows receive no automatic proof, and no microphone row receives one. There are no manual name corrections, remembered voices or trained self profile. Samples being unavailable/unhighlighted are not independently annotated acoustic silence. These are controlled personal-Teams/built-in-input energy-proxy results, not whole-wave human DER, WER, enterprise-layout or live AirPods qualification.

The qualified v7 recording survives a cold reopen with the same binary and profile: all 18 meeting files and the complete API result stay unchanged, including two applied names and 29 Single-SystemAudio proofs, with zero manual corrections, remembered voices or self profile. No second capture or inference is requested.

### Final-source access blocker

The previous signed v8 candidate (`bea7ab87…`, source manifest `8a2b1937…`) remains idle/OFF and untouched. Current production head `0af18c32dd7123ee3d45b168564af6874b51cf4e` is committed and its v9 build/sign-only verification is complete. The signed binary SHA-256 is `c73e5cf1824d9081a92977a4e9619e5a32de6717eb1cc2996d7f2cd9471670ed`, source manifest `ca733639ec373ef8a34b9b8410caa4c9b44bb1b00eda2acc21ea590971027d02` over 439 product source inputs, with personal team `B538Q2K76C` and deep/strict signature verification. It uses a guarded Tauri `--no-bundle` release build and a preserved verified private capture-bundle skeleton; no new DMG, notarization or ordinary-app installation is claimed. **V9 is not launched**, the active pointer is unchanged, and the previous v8 remains idle/OFF. There is **no qualifying current-source live run**. Computer use is blocked by the locked Mac after the quota-dialog dismissal failed; manual unlock is pending. No new QA room was created, the second negative test never received GO, and no capture was started in this blocked stage. The older QA room reached the service's 60-minute limit; no paid extension was used.

The successful v7 positive and cold reopen remain valid evidence for that immutable binary. They are not transferred to the current production head. Fresh final-source positive/negative/reopen testing and selected PR checks are required before any merge; static source reviews are complete, but the PR remains draft. Code review can proceed while access is unavailable. This is a live-test blocker, not a completed acceptance or a negative pass.

### Current reviewed source and completed checks

Production head `0af18c32dd7123ee3d45b168564af6874b51cf4e` includes the following reviewed corrections. Static standards/spec reviews have zero remaining code findings; this is not runtime live acceptance.

- Shared outputs preserve automatic names as estimates, while explicit manual names take priority. A derived-cache stamp prevents old unqualified labels surviving in cached outputs; prompt rules and mobile presentation retain the distinction.
- Two microphone-owner refusals are localized in English and Dutch, with a red-to-green localization regression and eight passing i18n tests.
- Bounded speaker observations use the actual midpoint only for reads at most 200 ms. Wider reads make the whole observation frame unknown; the sampler also quarantines the interval affected by Core’s highlight latency. Duplicate/conflicting evidence retains its veto, and cancelled evidence commits fail closed. The read-only latency accessor does not change the policy threshold. Targeted timing checks cover 38 cases and 16 UI-state cases.
- Clean-window reconciliation is described as average linkage; the documentation does not imply complete-link clustering.

| Current-source verification | Verified checkpoint |
| --- | --- |
| Core default / lean / keyboard | 1,189 / 1,171 / 17 passed; seven/five existing ignored tests |
| Core formatting / strict Clippy / Release | Passed; Release build 35.2 s |
| Desktop frontend | npm ci, production build and 439 tests passed |
| Desktop backend | 333 tests, formatting and strict all-target Clippy passed |
| Mac services | 198 tests passed on unchanged native source |
| Mobile fast/native compatibility | Typecheck; 160 UI, 17 plugin, seven widget, 116 MobileKit, 144 runtime/service and 36 keyboard tests passed |
| Mobile generation and products | Clean/repeated Prebuild, six pod-ownership checks, both iOS Rust targets and sequential unsigned ARM64 simulator/device Release builds passed |
| Mobile binding/source identity | Swift/header/module-map bytes unchanged; binding SHA `a3dd7102…`; static libraries bind current Core source `5833097d…` |

The owned simulator is shut down. These mobile results do not establish signed physical-device, keyboard cold-start, Cloud/store or real Teams acceptance. The current Mac build/sign and final desktop verification receipt are complete; v9 is not launched and current-source live acceptance remains blocked. Existing ASR language comparison remains exploratory without annotated WER; no new ASR-quality numerical gate or accuracy claim is invented.

The first negative attempt is **not qualified**. It records an actual app-to-global transition and successful JSONL fallback diagnostic at 26.064 seconds, but initial clock uncertainty is 123.087 ms, above the unchanged 100 ms limit. The guard stays closed, evidence has zero samples, no guests/public speech are admitted, and there is no qualifying post-transition SystemAudio ASR. The v8 candidate implements a bounded maximum of three fresh startup clock probes: the first valid probe returns immediately; all-wide probes still fail closed. Five new clock regressions and 20 meeting tests pass; the signed v8 build also passes 323 desktop-backend tests, formatting and strict Clippy, with 198 native and 28 sampler tests reported by their owners. This does not promote the failed attempt to a negative pass; fresh final-source positive/negative/reopen evidence remains required.

The separate negative-only PCM check is also sealed before a negative run (six unit tests). It reads only archived SystemAudio after the verified app-to-global transition upper bound plus three seconds, and requires a complete energetic 20 ms frame at both unchanged RMS thresholds alongside successful post-transition SystemAudio ASR. Missing actual scope/fallback/clock prerequisites suppress decoding. No negative live run has qualified yet.

`SystemAudio` proves the physical track, not app isolation. The positive case needs actual native scope and process identity. UTC meeting start metadata is not the PCM epoch. A missing/unverifiable clock or lost capture scope must fail naming closed. The fixed phase guard is a controlled-test allowance, not a universal network-delay guarantee.

The changed capture-scope/clock reasons and warning copy have **24 prepared synthetic images (12 before/after pairs)**, covering English/Dutch and light/dark from identical fixtures. Their after-source snapshot is `5b21804e…`. All 24 are published and read back on `pr-assets` at `7266a8b`. Eight further synthetic echo-warning-dedup images are published and read back at `7493c58`, using the same native capture input with only the mocked backend warning projection changed. A further 24 P2-review images are published and read back at `0a3f85d`: eight explicitly labeled mobile-component mocks and 16 English/Dutch microphone-refusal renders. Together with the earlier 32, the gallery retains 88 PR #190 PNGs. Each set remains tied to its recorded source snapshot; older snapshot hashes are not asserted as current catalog hashes after the new fixes. They show UI copy only, not native recording, real highlights or inference acceptance. The earlier 32-image set remains evidence for the preceding phase.

All sections below describe the **completed 9 November baseline** unless explicitly superseded by a final live-run receipt. Its test counts, source hashes, signed binary, stopped-QA state and repaired historical copies are not claims about the in-progress candidate.

## Completed saved-recording repair

Both recordings have one personal-headset microphone and four remote people. The first saved result contained twelve remote IDs and 347 generic labels among 412 stored rows. Remote allocation exhausted the portable ID space before the microphone was processed. Eight empty completion markers also moved out of chronological order.

The earlier PR candidate retained eight and eleven remote identities. The reviewed clean-window implementation now produces **four qualified remote identities in each recording**, without a configured speaker count. Explicit personal-microphone correction restores 161 and 24 owner passages. Full Core repair reports zero capacity-unassigned identities and preserves recognized words, typed notes and audio. Core and signed Mac replays each retain 440/397 canonical stored rows and project 432/396 nonempty chronological rows after timing splits; these differ from the earlier candidate's row counts. The eight/one internal completion markers remain hidden.

Original recordings, participant names, clips, embeddings, timelines and detailed receipts stay private in the ignored evaluation directory. Original folders and ordinary Pennel profiles remain unchanged. Older summaries remain stale rather than being regenerated through a provider. Matching the expected number of groups is supporting evidence, not whole-call accuracy proof.

## Implemented behavior

- Infer both physical sources before meeting-wide ID allocation. Preserve manual and established identities, reserve retained IDs and report incomplete allocation explicitly. A fully confirmed personal microphone needs no numbered identity slot.
- Build independent clean 1.5–5 second embedding windows and bound window inference, using the existing local models. Reconcile supported windows by average linkage while retaining native clustering distance `.6` and segmentation shift `.25`. No universal four- or five-speaker limit is added.
- Preserve hard constraints through weak reassignment, including mutually conflicting weak proposals, real overlap and distinct supported endpoints behind a dominant intermediate cluster. Adjacent source turns retain clean evidence; duplicated audio cannot inflate identity support. Uncertain windows remain unknown and all source time and overlap multiplicity are preserved.
- Keep owner confirmation, automatic Teams estimates and remembered biometric voices distinct. No voice enrollment is implicit. Guard commits against concurrent transcript, evidence, policy and manual-name changes.
- Learn names from finalized canonical **system-audio acoustic turns**, including clean intervals inside Mixed/Overlap transcript rows. Remove competing/unknown intervals and retain the existing agreement, duration, independent-turn/lineage and coverage gates. A candidate also needs a writable Single-row provenance target. Mixed words remain mixed; acoustic naming does not create word alignment.
- Bind optional Teams evidence to the detected recorded call, process lifetime, window and recording timeline. Stop observation on consent loss and finalize evidence after audio stops. Enabling names never changes the source selected for a manual recording. Automatic policy remains opt-in.
- Support the measured current English personal/free Teams navigation and camera-off tile shape. Exclude only the exact observed native macOS sharing-button utility window; generic unreadable dialogs and multiple calls still fail closed. Ordinary chat text values are not collected. Changed or localized layouts can report unavailable rather than guess.
- Preserve retryable partial recognition, completion-marker chronology, tentative summary meaning and capability-schema-5 compatibility. Speaker-only repair requires no retranscription or provider call.

Independent review found and reproduced weak reassignment bypassing an overlap constraint, adjacent clean windows disappearing and an imbalanced bridge joining distinct endpoints. Regression-first fixes were independently re-reviewed. Confidence thresholds were not lowered to reach a coverage percentage.

## Human references and decoder scope

The first recording has 35.755 seconds of calibration (A–D) and 11.975 seconds held out from the identity-mapping fit (E/F). Its one-to-one mapping is fitted once on calibration and is never refitted on E/F. These fixed references have been reused across candidate comparisons; they are not a new blinded whole-call qualification set. The second recording has 30.890 seconds of separately identified references (G–J); these fit that recording's mapping and are not a held-out identity-recognition set.

The following scores use the **actual saved Core canonical acoustic metadata**, counting every reference millisecond. Final signed Mac native-decoder replays produce exactly equal canonical metadata and visible projections; a separate replay with their actual voiceprint caches also produces exactly equal clean/Single naming intervals and gate results. Unknown, overlapping and noncanonical gaps count as abstention, not wrong names.

| Reference | Correct | Unknown / abstained | Wrong name |
| --- | ---: | ---: | ---: |
| First calibration A–D | 33.855 s | 1.900 s | 0 |
| First mapping-held-out E/F | 11.975 s | 0 | 0 |
| Second G–J | 29.373 s | 1.517 s | 0 |
| All confirmed passages | 75.203 s | 3.417 s | 0 |

This is 95.65% correctly assigned time and 4.35% abstention over 78.620 seconds of sparse reference audio, with zero measured speaker-confusion time. It does not establish dense whole-call error rates, general speakers or cross-recording identity recognition.

The standalone PCM16 diagnostic's first-calibration DER is 3.80%, with zero confusion and 1.358 seconds missed/unidentified. It must not replace the actual canonical Core calibration abstention of 5.314%. Quantization and canonical transcript bounds affect uncertainty. A replay of the exact Core float input matches the saved name/unknown sets across 944.539 seconds of observed row speech, with zero disagreement; the apparent projection loss in a cross-decoder comparison was not a product projection bug.

Increasing native clustering distance to `1.1` previously yielded four groups but merged two real people, with 44.99% held-out confusion, so it was rejected. A denser `.1` shift also worsened the labeled checks. A later diagnostic short-window attachment adds only 3.307 seconds on the first recording and introduces 0.240 seconds of disagreement with an independent model; it is not shipped. Embedding coherence also does not establish general speech/music classification: a synthetic stationary tone can qualify.

## Automatic-name replay and coverage

Neither historical recording contains captured Teams speaking observations. Sparse, truthful reference-only timelines therefore apply **zero automatic names** under the real policy. Missing historical highlights cannot be recreated.

A separate Sortformer model is run in default and offline configurations; their agreeing single-speaker intervals form a **model-derived timeline**, not recorded Teams evidence or dense human ground truth. The configurations share model weights. Sortformer is a diagnostic comparator with a four-speaker capability, not the production default or a universal speaker limit. The default-feature Core replay passes 31 guarded API controls and names four expected remote identities in each recording, with zero wrong names on the confirmed A–J intervals. This is repeated with snapshots of the actual signed Mac native-decoder caches, matching all gate results and named/unknown intervals exactly while leaving source profiles unchanged. Manual corrections, words, notes, physical source, canonical attribution, provenance, reruns and explicit enrollment choices remain intact.

Coverage retains both an all-consensus denominator and an independent transcribed-region denominator. The latter intersects consensus single-speaker audio with the **frozen pre-repair successful, nonempty system-audio transcript bounds**, with valid digest-matched metadata. It excludes no interval because of output identities, uncertainty masks, manual state or provisional attribution. Using newer timing-split rows would inflate the percentages, so they are not the primary denominator.

| Model-derived replay measure | First recording | Second recording |
| --- | ---: | ---: |
| All consensus single-speaker audio | 959.200 s | 1,346.960 s |
| Named share of all consensus audio | **75.83%** | **82.73%** |
| Frozen pre-existing transcribed-region denominator | 893.800 s | 1,284.780 s |
| Named share in those regions | **81.381%** | **86.730%** |
| Matching-model-name share in those regions | 81.310% | 86.383% |
| Model disagreement where named | 0.640 s | 4.460 s |
| Named Single-row transcript words | 62.89% | 59.97% |

Both transcribed-region naming percentages exceed 80%; **the first recording remains below 80% on all consensus audio**. Model disagreements are not verified human-label errors. All eligible canonical clean acoustic speech receives a name, but that denominator alone would hide abstentions and missing transcript regions. Naming an acoustic voice does not justify assigning every word in a coarse Mixed row.

Independent ASR-region exclusions total 65.400 and 62.180 seconds of consensus speech. Existing successful-empty completion markers account for 32.240 and 4.960 seconds; other missing successful system transcript rows account for 33.160 and 57.220 seconds. The empty markers occupy 141.300 seconds across eight first-recording regions and 16.600 seconds in one second-recording region. Repair does not invent words or transcript rows to cover these gaps.

The earlier local ASR comparison retains eight automatic-language/Dutch attempts on four fixed clips, with unchanged audio/model hashes. Three pairs change text, but there is no annotated word reference or WER claim. The installed model returns segment timestamps without word/token timings. No global language default or blanket foreign-text filter changed.

## Completed native-observer component evidence

The actual production observer is exercised through the public C ABI in a personally signed native probe on macOS 27.0.1 and Teams 26289.204.5281.6363, using a controlled call with public synthetic guest voices. No recording content is sent to Teams. The measured source fixes restore readable participants and stable process/window association; the speaking predicate itself remains unchanged.

| Live control | Available samples | Highlighted / not highlighted | False muted-guest highlights |
| --- | ---: | ---: | ---: |
| First guest only | 100 / 100 | 65 / 35 | 0 |
| Second guest only | 100 / 100 | 66 / 34 | 0 |
| Both guests muted | 100 / 100 | 0 / 100 | 0 |

A fourth 100-sample phase with both guests unmuted observes 28 unhighlighted samples, 26 first-guest-only, seven second-guest-only and 39 simultaneous highlights. All 400 samples are available; every phase has stable identity, zero duplicate names and no unexpected participants. Unhighlighted samples are not independently annotated acoustic silence, so this does not measure per-sample voice-activity recall. The both-muted control produces no highlights. A further 240-sample lifecycle trace has 89 readable samples and 151 unavailable samples while the call is minimized or no longer associated. Every unavailable sample returns zero names; restoring the call establishes a fresh association. Four additional post-quit samples report `app_unavailable` with no names. These are availability/association controls, not automatic-name writeback. Early silent-input and navigation harness attempts are retained as failed harness attempts, not passed product checks. These observer controls are separate from the saved-recording automatic-writeback tests below; they do not establish live capture-to-name writeback.

## Completed replay verification and retained products

| Verification | Completed 9 November result |
| --- | --- |
| Core default / lean / keyboard | 1,185 / 1,167 / 17 passed; seven/five existing opt-in tests ignored |
| Desktop frontend / backend | 436 / 287 passed; production frontend and Core Release builds passed |
| Mac services / Rust sampler | 175 / nine passed |
| Speaker library | 49 model-free / 50 native passed; one existing native recording test ignored; 27 new clean-window regressions |
| Naming API controls / offline scorer | 31 / 15 passed; language-policy helpers two passed |
| Formatting, strict Core/backend/speaker Clippy | Passed |
| Mobile fast checks | 17 plugin, seven widget, 159 UI and 116 MobileKit tests; typecheck passed |
| Generation and linking | Clean/repeated isolated Prebuild and six actual pod ownership checks passed; both final iOS Rust targets regenerated |
| Binding compatibility | Swift bindings byte-identical to the prior API generation |
| iOS products / hosted tests | Unsigned ARM64 simulator/device Release builds; 144 runtime/service and 36 keyboard tests passed |
| Native app UI | Both recordings' automatic-estimate labels and Me/Ik retention verified; four English/Dutch light/dark AX cases and three own-window images; Mixed/Overlap stays uncertain |
| Visual evidence | Existing 32 matched synthetic English/Dutch light/dark images on `pr-assets/pr-190/`; continuation changes no product UI; current own-window Settings images were observed through CUA and are not published |

The initial implementation commits are `6b5844b`, `e7fa483`, `b3d55d8` and `d0e7f32`; clean-window continuation is `b32ce44`, and naming/native continuation is `75af0ec`. The [sanitized artifact receipt](speaker-quality-22-101.json) pins reviewed source content and product hashes. The previous signed candidate (`cafe0dce…`) and its native owner-repair/cold-reopen evidence remain retained as baseline, with eight/eleven remote groups rather than the current four/four.

The 9 November personally signed Capture Preview binary has SHA-256 `86b4635b0dc9f64b05c6f31db74a56d9e4dfc1bed26652999ee03cfdcb42b3d1` and verified personal team `B538Q2K76C`. It is built with Tauri `--no-bundle` and assembled in a new private bundle from a verified preserved capture skeleton, then signed and deep/strict verified. It is not a fresh bundler/DMG, notarized release, installation or signed upgrade. The actual packaged `diarize_meeting` handler replays both recordings through the native AVAudioFile/AVAudioConverter decoder, without a substitute ffmpeg path or new audio capture. It completes in 94.98 and 65.76 seconds, retaining four remote IDs, 161/24 owner rows, zero capacity-unassigned identities, chronological output and ordered words, notes and audio. Source revisions match the Core-driver results. Backend cold reopen is verified. Full canonical metadata and visible projections match the Core driver exactly. Native voiceprint caches differ numerically, so a separate replay validates their actual naming gates and reproduces the same named/unknown intervals and coverage. All 439 product source inputs match reviewed head `0384523` byte-for-byte; the older embedded dirty revision is not a release-source claim.

A further private clone runs the actual packaged diarization hooks with **model-derived, nonhistorical** observation timelines and the automatic policy. Both runs store four expected names and reach `Applied`, with 114/161 valid automatic-provenance rows (275 total), valid evidence revisions, unchanged canonical words, times, physical source, manual state, owner rows, notes and audio, and no enrollment. They complete in 89.82/65.42 seconds; repeated reads remain stable. The real app exposes automatic Teams-estimate labels and Me/Ik in four English/Dutch light/dark AX cases, without falsely naming unknown, Mixed or Overlap words. Three cases also have observed own-window images; the English/light case has AX verification without a new native image. The native Settings images are not published. A cold reopen retains all eight names, all 275 proof rows and `Applied` status, with unchanged canonical/provenance files, zero remembered voices and no self-voice profile. The UI exercise changes only the private interface-language setting from English to Dutch; speech language and STT choices remain unchanged. Separate repair previews now contain four meeting-local manual aliases per recording, applied from the owner-confirmed reference samples. Their 114/161 manually named rows and 161/24 owner rows were checked in both real meeting views. Names have no Teams-estimate suffix; unknown, Mixed and Overlap passages remain uncertain. These previews contain no automatic Teams evidence/proofs or voice enrollment: their historical names are reference-based manual corrections, not recovered Teams observations.

Private source snapshots, all prior/current binaries, models, reference/attempt diagnostics, recovery backups and iOS products are retained. The owned simulator is shut down; no physical phone or Metro changes were made. Both owned review-preview folders are refreshed at their existing paths after ownership guards verify all 35/34 previous data files against the earlier baseline. Complete old-folder backups retain 37/36 hash-verified files. The new 70/69 data files match the validated reference-repair profile; their readme/reference mapping identifies the manual-name provenance. Both targets remain outside all three ordinary Pennel/Preview libraries. All 39 frozen source files across the original folders remain unchanged.

All owned QA apps, guest browsers and Teams processes are stopped, the owned simulator is shut down, and QA artifacts are frozen. Ordinary profiles, provider data and credentials are unchanged. The patch is not installed in the ordinary app.

Live capture-to-automatic-name writeback remains separate from the actual saved-audio writeback and controlled native-observer tests. Open acceptance also includes dense independent whole-call annotations, additional consented Flemish calls, broader Teams layouts/locales and shared devices, installed-app/signed-upgrade behavior, accessibility and physical iPhone acceptance. Automatic naming remains opt-in and historical model-derived estimates are not presented as recovered Teams evidence. No production release is claimed.
