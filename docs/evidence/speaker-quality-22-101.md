# Speaker quality and automatic Teams names — #22 / #101

PR #190 builds on merged PR #189 at `6dd72cc`. Both saved Flemish Teams recordings have been repaired and checked as described below. **The final V11 live recording-to-name positive, source-loss control and cold reopen now qualify on the same personally signed binary.** Product source is `1a3ca4772dd5b8a1b470226341705e099bad5a6f`; this final update changes evidence only. Final review and selected checks on the evidence commit still precede merge. #22 and #101 remain open for their broader acceptance criteria; #100 owns broader manual-correction work.

## 10 November: final live recording-to-name acceptance

The actual production detector and prompt answer handler start a fresh recording in an isolated profile. Pennel creates its own UID, both physical audio files, local ASR, canonical turns, actual Teams observations and automatic proofs. No historical audio, reference aliases, voice enrollment or authored/model-derived observations are imported. Read-only monitors archive and score actual output.

The measured setup is English personal Teams desktop on macOS 27.0.1 with the built-in microphone, public Dutch/English synthetic guest speech and a visible call window. It does not qualify the original work-call accessibility layout, other Teams locales, enterprise layouts, shared devices or a fresh AirPods call. The two saved AirPods recordings remain separate acoustic evidence.

| Acceptance stage | Final V11 result |
| --- | --- |
| Production start and source binding | Actual detector/prompt/answer handler; app-scoped SystemAudio, native process identity, live producer and capture guard verified |
| Native clock | 1.0951045 ms uncertainty using the second fresh startup probe; unchanged 100 ms maximum |
| Automatic output | Two expected names, 20 valid Single-SystemAudio provenance rows; no automatic proof for microphone, Mixed, Overlap or Unknown text |
| ASR and storage | 41 successful nonempty rows, zero failed ASR rows; postprocessing complete, zero unassigned voices, no pending recovery |
| Independent naming correctness | Zero wrong automatic names across all controlled solo energy frames; held-out canonical confusion zero on its separately stated detected-turn denominator |
| Canonical named coverage | 100% of eligible clean single-voice speech; 91.92% of all detected canonical SystemAudio speech |
| Independent held-out PCM coverage, −45 dBFS | 89.74% overall; first actor 80.57%, second 100.00% rounded |
| Independent held-out PCM coverage, −55 dBFS | 89.40% overall; first actor 80.22%, second 99.99% |
| Normal end | Native Teams Leave, without pressing Pennel Stop; capture off within a conservative 983.881064 ms upper bound |
| Cold reopen | Same binary/profile; both names, all proofs, complete actual API and all 18 meeting files unchanged; no inference, capture, manual correction or enrollment |
| Source-loss negative | Same V11 binary: actual app→global fallback, continued SystemAudio PCM and successful ASR, zero names/proofs, evidence cleared and source guard invalid |
| Isolation and cleanup | Owned detector/hotkey off and recording idle before exit; owned QA app and guests stopped, native Teams speaker mute restored at unchanged volume; ordinary apps/data preserved |

The positive records 918 actual evidence samples, 913 available, and 468.028 seconds of SystemAudio PCM. Unavailable observations are not assumed to be acoustic silence. Speaker processing honestly reports `limited_evidence`; that is compatible with completion and conservative unknown attribution. Public guest inputs are not human whole-call ground truth.

### Fixed scoring and declared start scenario

The independent PCM protocol was sealed before qualifying public-speech tests: 16 kHz mono Float32, 20 ms RMS frames at both −45 and −55 dBFS, three-second phase-edge guards and at least ten seconds of active audio per solo phase at each threshold. Each actor and the aggregate must reach 80% strict correct-name coverage across held-out solo phases; all controlled solo phases must have zero wrong automatic labels. Missing ASR, diarization or clean source-turn support counts as unnamed. The first solo phase per actor establishes one diagnostic mapping; held-out data never refits it. Ground truth uses actual CUA mute-action UTC brackets and conservative RPC wall/runtime bounds, independently of observed Teams highlights. Both scorers use the same phase parser. The unchanged protocol and evaluator receipts are hashed in the sanitized JSON.

The first actor's aggregate margin is narrow: 80.22% at −55 dBFS. One individual held-out phase is below 80%; the preregistered gate is per actor across held-out phases, not per individual phase. These are energy-proxy results, not human DER, WER, VAD recall or a claim that every spoken word is named. Missing speech remains in the denominator.

The final positive starts with public guest speech already connected. This scenario amendment was recorded at **10:03:41.992650 UTC, before the final run and its name outcomes**. Production prompt/handler, source, clock, binding, naming-policy thresholds and accuracy gates remain unchanged. The original host-alone-start scorer/report is preserved and fails that extra scenario check; it is not silently discarded or counted as a host-alone pass. The amended scorer accepts connected speech only when the preregistration precedes the run and actual early runtime proves heard app-scoped audio, a verified clock, source guard and live binding. Eight semantic controls reject absent/late amendments and invalid source/clock/binding. A wholly silent app can still widen capture conservatively and lose automatic names; this is disclosed rather than represented as a qualified silent-start positive.

### Same-binary source-loss control

An intended V11 positive loses app scope while the native Teams speaker is off. **It remains a failed positive.** It also supplies a real source-loss control against the already frozen negative gates: actual verified app→global transition, successful native fallback diagnostic and 65.3104165 ms clock uncertainty. Guests were admitted before the first observed global state; no post-transition-admission claim is made.

Only actual SystemAudio after the observed transition upper bound plus three seconds is credited: 11 successful nonempty ASR rows / 352 words and 136.100 / 144.100 seconds of energetic PCM at −45 / −55 dBFS. There are zero automatic names or proofs; evidence is cleared and naming remains unavailable with `audio_capture_not_app_scoped`. Audio capture/transcription therefore survives loss of optional naming authority. The failure classification and separate negative assessment are both retained.

### Failures found and fixed

Earlier attempts remain archived with their original outcomes; a passing older binary never qualifies a newer candidate.

- Microphone startup initially failed despite granted permission. Correct buffer/channel handling fixes the native startup path. A later missing-system-tap attempt was stopped before public input and qualifies as neither positive nor source-loss negative.
- An early positive lost observation and took 31.9 seconds to stop. Actual registry metadata gaps and supported call-end transitions now retain identity conservatively; the final same-binding end reconciliation has a fixed two-second monotonic deadline and never resumes naming votes after uncertainty.
- A negative startup correctly failed 100 ms clock verification. At most three fresh probes are now allowed; all-wide probes still fail closed without widening the bound.
- V9 positive6 failed visibility and exposed a fractional recording-tail save error. Exclusive audio ends now use consistent ceiling semantics; legacy Pending recovery requires actual sample/hash proof and preserves successful metadata and manual edits. A private-copy recovery preserves all 39 nonempty pending rows / 1,017 words and all 41 prior metadata rows; original and backup hashes are unchanged. The original failed naming run is not promoted.
- V10 positive7 failed clean-end detection. Final V11 adds the bounded terminal-only reconciliation above; independent delta review and 202 native tests pass.
- V11 intended positives8/9/10 lost app scope while native Teams playback was muted. They remain failed positives. After the owner's explicit authorization, the native speaker was enabled at its existing volume for the final run and restored to mute afterward. Only positive8 separately meets the frozen source-loss gates.

The earlier V7 positive/cold reopen and V9 negative are retained version-specific history. Their scores are not transferred to V11. All numerical accuracy gates remain unchanged.

## Final source, checks and qualification boundary

Final binary SHA-256 is `df3b05ebcc78e5ea4cd548c129285bf85bc11574b774fa49ac12998cc7a96a30`, signed with personal team `B538Q2K76C`, with deep/strict signature and hardened-runtime verification. Its 439 product inputs match `1a3ca47`; source-manifest SHA-256 is `3fb301df69e1ec7b84f11f5baff332f24e8493398b3f3a9bed5d5d0fc1e69747`. The guarded Tauri `--no-bundle` Release build is assembled into a preserved verified private capture-bundle skeleton. This is not a fresh DMG, notarized release, ordinary-app installation or signed upgrade.

| Verification | Final product source |
| --- | --- |
| Core default / lean / keyboard | 1,198 / 1,180 / 17 passed; seven/five existing ignored tests |
| Core formatting / strict Clippy / Release | Passed |
| Desktop frontend | npm ci, production build, typecheck and 439 tests passed |
| Desktop backend | 333 tests, formatting and strict all-target Clippy passed |
| Mac services | 202 tests passed on final native source |
| Mobile compatibility | Typecheck; 160 UI, 17 plugin, seven widget and 116 MobileKit checks on hash-identical mobile sources; current Core rebuilt into both ARM64 static libraries |
| Mobile generation/native | Clean/repeated Prebuild, six pod-ownership checks, sequential unsigned ARM64 simulator/device Release builds, 144 runtime/service and 36 keyboard tests passed |
| Mobile bindings | Swift/header/module-map unchanged (`a3dd7102…`); native products bind current Core `bbe92703…` |
| Hosted product-source CI | `ci` [42791184903](https://github.com/pennel-app/pennel/actions/runs/42791184903), `desktop` [42791185036](https://github.com/pennel-app/pennel/actions/runs/42791185036), decision evaluation [42791185154](https://github.com/pennel-app/pennel/actions/runs/42791185154), `mobile` [42791184858](https://github.com/pennel-app/pennel/actions/runs/42791184858): all successful at `1a3ca47` |
| Code review | Standards and Spec reviews plus final persistence/native delta reviews have zero remaining material code findings; final evidence review/checks precede merge |

Shared outputs retain automatic names as estimates, manual names win, derived caches refresh, owner refusals are localized in English/Dutch, and slow/stale/duplicate/conflicting evidence remains conservative. Average-link reconciliation is described accurately. No dependency or portable-schema change is added by the final persistence/native fixes.

The [sanitized receipt](speaker-quality-22-101.json) contains the final result and preserves the superseded pre-unlock checkpoint as historical data. Private audio, participant mappings, room URLs, actual UIDs, profiles and detailed receipts stay ignored. The final owned capture app and two guests are stopped; both guests visibly left, the host microphone/camera stayed off, and Teams' speaker mute was restored at volume 6. Native Teams' ordinary main app, ordinary Pennel profiles, physical phone and Metro are preserved. The owned simulator is shut down.

The PR gallery retains 88 matched synthetic EN/NL light/dark PNGs at `pr-assets` commit `0a3f85d3923199779b2562c16ef7cb48a632af41`: 64 earlier renders, 16 owner-refusal renders and eight explicitly marked mobile-component mocks. These show UI copy; native mobile layout/accessibility acceptance remains separate.

Broader #22/#101 criteria remain open: dense independent whole-call annotations, additional consented Flemish/English/hybrid recordings, CPU/memory coverage, broader adapters/layouts/locales/shared devices, fresh AirPods live capture, installed upgrades, accessibility and signed physical iPhone acceptance. Language-detection versus explicit-Dutch comparison remains exploratory without annotated WER. No production release or universal automatic-name reliability is claimed.

The sections below retain the **completed 9 November baseline**. Their hashes, counts and stopped-QA statements describe that phase, independently of the final V11 live qualification above.

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
