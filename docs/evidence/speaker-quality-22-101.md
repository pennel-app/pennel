# Speaker quality and automatic Teams names — #22 / #101

This implementation follows the owner's approved repair plan. It builds on merged PR #189 at `6dd72cc`; review, merge and real-call acoustic/name acceptance are separate gates. #100 retains the broader manual-correction workflow.

## Reference and confirmed failures

A consented 44-minute Flemish Teams recording has one personal-headset microphone and four remote people. The saved result contained twelve remote IDs and 347 generic labels among 412 rows. Remote allocation exhausted the portable ID space before the microphone was processed. Its speaker pass preserved recognized words but left every microphone turn without an allocated identity. Eight retained empty completion rows also moved to the end of saved Markdown.

The original recording remains unchanged. Source files, participant names, clips, embeddings and detailed diagnostics stay in an ignored private build directory. Only sanitized counts and implementation evidence belong here. The recording manifest identifies transcription configuration, not the historical speaker route; current settings alone cannot prove that route.

## Implemented behavior

- Infer both sources before assigning source-fair portable IDs; preserve manual and established mappings, reserve unchanged rows and expose unassigned capacity explicitly.
- Extract voiceprints from clean non-overlapping speech; reconcile supported repeated fragments at `.70` cosine while retaining native clustering `.6` and window shift `.25`.
- Admit modern identities from valid embeddings with three seconds across two clean spans or a ten-second monologue. Keep weak groups unknown rather than exhausting IDs, mask weak prints before known/self matching and preserve an explicit legacy path. A fully confirmed personal microphone consumes no numeric slots.
- Keep manual microphone-owner assertions, automatic Teams names and remembered biometric voices distinct. Guard final commits against concurrent source, evidence and policy changes.
- Bind Teams observations to the recording and native process/window, reject ambiguous calls and cease observation on consent loss. Finalize optional evidence after stopping audio.
- Preserve retryable partial recognition text and audio without promoting an incomplete result to success. Request supported native timestamps; retain honest mixed attribution when alignment is insufficient.
- Preserve completion-marker chronology and tentative summary meaning. Speaker-only repair does not require new transcription or a provider call.
- Use capability schema 5 only for explicit owner, automatic-name or partial-text semantics; preserve earlier metadata compatibility and protect unsupported readers.

## Private evaluation

The offline evaluator retains input/model/binary hashes, exact options, timings, peak memory, raw/reconciled anonymous turns and complete attempts. Accuracy requires independently annotated regions; unannotated input is explicitly not scored as silence.

Local whole-recording runs used the existing pinned segmentation and embedding models. Native `.25` shift / `.6` distance produced 61 raw system groups on the first recording. An initially stricter fragment policy retained 57; the accepted `.70` policy retains 52. The second recording changes from 49 raw / 46 initially reconciled groups to 42. These raw groups include tiny unreliable fragments; modern admission is a separate step. Native clustering and segmentation defaults remain unchanged.

Increasing native distance to `1.1` happened to yield four groups, but independent user identification proved that it merged two actual people; its held-out confusion was 44.99%. It was rejected. A denser `.1` shift also worsened the labeled checks. Count matching was not used as an acceptance criterion.

The first recording has 35.755 seconds of calibration and 11.975 seconds genuinely held out, with one mapping fitted only on calibration. Accepted fragment consolidation reduces calibration confusion from 5,449 to 4,159 ms; the unchanged 68 ms VAD-boundary miss gives 15.43% → 11.82% calibration DER. The held-out check remains zero. Four independently identified second-recording passages total 30.890 seconds and show zero confusion with the accepted policy; each person appears once and default-selected clear passages favor the baseline. These sparse checks do not qualify whole-call accuracy, general speakers or cross-recording identity recognition.

On isolated copies, explicit personal-microphone correction restores 161 first-recording and 24 second-recording passages. Final local reprocessing retains eight and eleven numbered groups, with zero capacity-unassigned groups and explicit `limited_evidence` guidance. The first signed packaged backend produces the same final source revision as the offline core driver. All 412 first-recording stored rows remain chronological, with 404 visible nonempty rows. Words, typed notes and audio are preserved; both original folders' hashes are unchanged. Whole remote groups are not renamed from isolated clip labels. The older summaries are retained as stale rather than silently regenerated through a provider.

An offline native-component ASR comparison used four fixed clips and eight automatic-language/Dutch attempts with identical audio and the installed model's original hash. All eight attempts completed; language mode changed text in three of four pairs. There is no annotated word reference, so no WER or accuracy improvement is claimed and no global language setting was changed. The installed model advertises segment timestamps only and returned no word/token timings; requesting its finest supported granularity therefore does not establish fine word alignment. The standalone example refuses inference when local models are not compiled in.

## Verification and acceptance

Regression-first checks reproduced source starvation, completion ordering, lost partial-result completeness and overlapping voiceprint contamination before fixes. Independent review additionally exercises concurrent status ownership, retained-ID reservation, stable-ID admission, acoustic evidence duration, multiple call windows and consent revocation.

| Verification | Result |
| --- | --- |
| Core default / lean / keyboard | 1,180 / 1,162 / 17 passed; seven/five existing opt-in tests remain ignored |
| Desktop frontend / backend | 436 / 287 passed; production frontend build passed |
| Mac services / Rust sampler | 168 / nine passed |
| Native speaker library / WAV parser | 23 / one passed; one existing real-recording test ignored |
| Offline scoring / local ASR tool | 15 / six passed |
| Formatting, strict core/backend/native Clippy | Passed |
| Mobile fast checks | 17 plugin, seven widget, 159 UI and 116 MobileKit tests; typecheck passed |
| Generation and linking | Clean/repeated Prebuild and six ownership checks passed; both final Rust targets regenerated |
| Final binding compatibility | Swift bindings remain byte-identical after the internal quality refinement |
| iOS native products / hosted tests | Unsigned ARM64 simulator/device Release builds; 144 runtime/service and 36 keyboard tests passed |
| Visual evidence | Matched synthetic English/Dutch light/dark renders on `pr-assets/pr-190/`; not acoustic or human acceptance |

Product changes are committed as `6b5844b`, `e7fa483`, `b3d55d8` and `d0e7f32`. The last change preserves manual capture selection independently of naming. Native owner repair and preserved Me labels were observed through the personally signed app's real UI; final acoustic admission was then replayed through its actual packaged backend. No microphone recording or real Teams call was staged. Earlier helper-navigation attempts are retained as failed harness attempts, not counted as passed product checks.

The [final artifact receipt](speaker-quality-22-101.json) records the personal team, exact Mach-O hash, final native manifests and command scope. The final signed capture binary at `d0e7f32` has SHA-256 `cafe0dce49ef58917dc7675744f9aa05c1ed7a066f7a2bbbe055adc7c854bae1`, verified with personal team `B538Q2K76C`; it is not notarized. A cold packaged reopen preserves the 161 owner rows, zero capacity-unassigned groups and limited-evidence explanation without voice enrollment. Final iOS Release products were rebuilt after the shared copy change; the internal quality refinement leaves binding bytes unchanged.

Signed baseline/final candidates, native products, private audio/reference/attempts and recovery backups are retained locally. The ordinary profiles, provider choices and credentials are unchanged; test apps and the owned simulator are stopped. This patch has not been released or installed into the ordinary app. Review copies retain the original meeting identity and remain outside the active library.

Open acceptance: independently annotated five-person recovery and at most 10% speaker-confusion time; zero incorrect automatic names and at least 80% named coverage on usable Teams evidence; additional consented Flemish calls; real layouts/shared devices, installed-app behavior, signed upgrades and accessibility. No claim of these results follows from deterministic tests or a matching number of clusters. Automatic naming remains opt-in; the owner's ordinary profile is not switched automatically before qualification.
