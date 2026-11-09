# Speaker quality and automatic Teams names — #22 / #101

PR #190 builds on merged PR #189 at `6dd72cc`. The continuation replaces the earlier fragment-reconciliation candidate and validates two consented Flemish Teams recordings. Review, release and broader real-call acceptance remain separate gates; #22 and #101 stay open, with #100 owning the broader manual-correction workflow.

## Confirmed failure and current repair

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

## Live Teams evidence

The actual production observer is exercised through the public C ABI in a personally signed native probe on macOS 27.0.1 and Teams 26289.204.5281.6363, using a controlled call with public synthetic guest voices. No recording content is sent to Teams. The measured source fixes restore readable participants and stable process/window association; the speaking predicate itself remains unchanged.

| Live control | Available samples | Highlighted / not highlighted | False muted-guest highlights |
| --- | ---: | ---: | ---: |
| First guest only | 100 / 100 | 65 / 35 | 0 |
| Second guest only | 100 / 100 | 66 / 34 | 0 |
| Both guests muted | 100 / 100 | 0 / 100 | 0 |

A fourth 100-sample phase with both guests unmuted observes 28 unhighlighted samples, 26 first-guest-only, seven second-guest-only and 39 simultaneous highlights. All 400 samples are available; every phase has stable identity, zero duplicate names and no unexpected participants. Unhighlighted samples are not independently annotated acoustic silence, so this does not measure per-sample voice-activity recall. The both-muted control produces no highlights. A further 240-sample lifecycle trace has 89 readable samples and 151 unavailable samples while the call is minimized or no longer associated. Every unavailable sample returns zero names; restoring the call establishes a fresh association. Four additional post-quit samples report `app_unavailable` with no names. These are availability/association controls, not automatic-name writeback. Early silent-input and navigation harness attempts are retained as failed harness attempts, not passed product checks. These observer controls are separate from the saved-recording automatic-writeback tests below; they do not establish live capture-to-name writeback.

## Verification and retained products

| Verification | Current continuation result |
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

The new personally signed Capture Preview binary has SHA-256 `86b4635b0dc9f64b05c6f31db74a56d9e4dfc1bed26652999ee03cfdcb42b3d1` and verified personal team `B538Q2K76C`. It is built with Tauri `--no-bundle` and assembled in a new private bundle from a verified preserved capture skeleton, then signed and deep/strict verified. It is not a fresh bundler/DMG, notarized release, installation or signed upgrade. The actual packaged `diarize_meeting` handler replays both recordings through the native AVAudioFile/AVAudioConverter decoder, without a substitute ffmpeg path or new audio capture. It completes in 94.98 and 65.76 seconds, retaining four remote IDs, 161/24 owner rows, zero capacity-unassigned identities, chronological output and ordered words, notes and audio. Source revisions match the Core-driver results. Backend cold reopen is verified. Full canonical metadata and visible projections match the Core driver exactly. Native voiceprint caches differ numerically, so a separate replay validates their actual naming gates and reproduces the same named/unknown intervals and coverage. All 439 product source inputs match reviewed head `0384523` byte-for-byte; the older embedded dirty revision is not a release-source claim.

A further private clone runs the actual packaged diarization hooks with **model-derived, nonhistorical** observation timelines and the automatic policy. Both runs store four expected names and reach `Applied`, with 114/161 valid automatic-provenance rows (275 total), valid evidence revisions, unchanged canonical words, times, physical source, manual state, owner rows, notes and audio, and no enrollment. They complete in 89.82/65.42 seconds; repeated reads remain stable. The real app exposes automatic Teams-estimate labels and Me/Ik in four English/Dutch light/dark AX cases, without falsely naming unknown, Mixed or Overlap words. Three cases also have observed own-window images; the English/light case has AX verification without a new native image. The native Settings images are not published. A cold reopen retains all eight names, all 275 proof rows and `Applied` status, with unchanged canonical/provenance files, zero remembered voices and no self-voice profile. The UI exercise changes only the private interface-language setting from English to Dutch; speech language and STT choices remain unchanged. Separate repair previews now contain four meeting-local manual aliases per recording, applied from the owner-confirmed reference samples. Their 114/161 manually named rows and 161/24 owner rows were checked in both real meeting views. Names have no Teams-estimate suffix; unknown, Mixed and Overlap passages remain uncertain. These previews contain no automatic Teams evidence/proofs or voice enrollment: their historical names are reference-based manual corrections, not recovered Teams observations.

Private source snapshots, all prior/current binaries, models, reference/attempt diagnostics, recovery backups and iOS products are retained. The owned simulator is shut down; no physical phone or Metro changes were made. Both owned review-preview folders are refreshed at their existing paths after ownership guards verify all 35/34 previous data files against the earlier baseline. Complete old-folder backups retain 37/36 hash-verified files. The new 70/69 data files match the validated reference-repair profile; their readme/reference mapping identifies the manual-name provenance. Both targets remain outside all three ordinary Pennel/Preview libraries. All 39 frozen source files across the original folders remain unchanged.

All owned QA apps, guest browsers and Teams processes are stopped, the owned simulator is shut down, and QA artifacts are frozen. Ordinary profiles, provider data and credentials are unchanged. The patch is not installed in the ordinary app.

Live capture-to-automatic-name writeback remains separate from the actual saved-audio writeback and controlled native-observer tests. Open acceptance also includes dense independent whole-call annotations, additional consented Flemish calls, broader Teams layouts/locales and shared devices, installed-app/signed-upgrade behavior, accessibility and physical iPhone acceptance. Automatic naming remains opt-in and historical model-derived estimates are not presented as recovered Teams evidence. No production release is claimed.
