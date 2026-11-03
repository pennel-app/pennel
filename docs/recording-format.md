# Meeting recordings and provenance

Historical commands that mention `make app`, the old iOS targets or retired SwiftUI shells refer to the dated candidates recorded below. Those targets were removed under [cleanup #75](https://github.com/pennel-app/pennel/issues/75); current builds use the [desktop](desktop.md#build-and-test) and [mobile](mobile.md#build-and-verification) workflows.

New desktop recordings keep `meeting.md`, `transcript.md` and `notes.md` at the top of the existing `YYYY-MM-DD HHmm Title` folder. Microphone and computer audio are `audio/microphone.m4a` and `audio/system.m4a`. An iPhone room recording uses `audio/room.m4a`; its physical source is `room_microphone`, even though the transcript uses the Others speaker role. Audio remains AAC at 16 kHz mono.

Existing meetings are not migrated. The shared core resolves legacy `ik.m4a` and `anderen.m4a` names as well as the new layout. Adapters use that resolver for availability, playback, speaker recognition and retranscription. Retention covers both layouts. The stable `pennel_id` survives folder/title changes. A copied folder without hidden metadata can still resolve its fixed audio filenames; ambiguous mixtures and paths escaping the meeting are refused.

## Local metadata

`.pennel/recording.json` has `schema_version: 1` and records:

- Meeting UUID, start/end, precise duration and capture completeness.
- App version/build, platform and OS version.
- Original frozen speech provider kind, model, requested language and the SHA-256 of a downloaded local model when present. Requested language is configuration provenance, not proof that the model can force that language.
- Physical track sources, relative paths, archive formats, input-format epochs and transport categories. On Mac, format epochs include reported tap and effective aggregate rates.
- Available frame/sample counts, inserted silence, gaps, rate mismatches, capture errors and dropped diagnostic events.
- Initial and subsequent transcription attempts with their own configuration, outcome and transcript fingerprint. Retranscribing does not replace the original capture configuration.

`.pennel/capture-events.jsonl` stores metadata-only transitions, interruptions and short error codes. Metrics update the manifest without adding a log line every second. Native diagnostic buffering is bounded; dropped-event counts are retained. Unknown hardware information remains unknown rather than being inferred from the archive format.

Diagnostics contain no transcript/audio content, credentials, account identifiers, device serial numbers or absolute paths. Both `.pennel/` and audio remain local and are excluded from text sync. Safe provenance is also preserved in `meeting.md` frontmatter. Recording metadata participates in retranscription backup and rollback. Metadata finalization failures remain retryable after the transcript has finished saving.

## Mac Bluetooth capture

The tap is attached to an aggregate before resolving its effective stream format. The recorder selects validated tap inputs, disables unused hardware input/output streams, and uses acquisition timestamps. Format/rate/routing notifications are coalesced for 1.5 seconds; stale callbacks cannot enter replacement sessions. A recording-lifetime route watcher permits recovery even when tap startup failed.

Rate disagreement is checked against acquisition timing. Genuine interruptions remain silence on the common timeline; persistent timing failures stop affected delivery and request recovery instead of indefinitely adding periodic padding. The recording retains a warning and incomplete status when capture was damaged, even if audio subsequently resumes.

## Verification and acceptance

Implementation is tracked in [#47](https://github.com/pennel-app/pennel/issues/47), with actual Mac acceptance under [#5](https://github.com/pennel-app/pennel/issues/5). Synthetic checks cover stale 48 kHz metadata with 24 kHz input, pitch/duration, supported layouts/rates, callback timing, notification bursts and stale generations. Storage checks cover old/new/room layouts, privacy, retention, identity and interrupted replacement.

A real Bluetooth call on the affected Mac is still required. Record the candidate/build, OS/headset/input/output configuration, playback quality and Dutch transcription, including profile and route changes. Automated checks and private recovery experiments do not establish hardware acceptance.

The supplied damaged recording was processed separately outside the repository. Its recovered transcript is approximate, uses original-clock timestamp mapping and keeps source files unchanged. Recovered playback has a different timeline; no recovered audio or transcript is installed into the production library or uploaded.


Local verification on 28 October 2026: `make core`, `make test` and `make app` passed (605 core tests, 5 ignored; retained Mac Swift suites also passed). The shared desktop frontend built and passed 185 tests; `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` passed 205 tests. Core/desktop formatting and Clippy with warnings denied, 39 packaging checks and 93 Mac platform Swift tests passed. `scripts/build-desktop-preview.sh` produced the final preview app and DMG; signature/integrity checks are separate from notarization or device acceptance. The [mobile validation record](mobile.md#recording-provenance-validation--28-october-2026) covers clean/repeated generation, simulator tests and unsigned Release builds. No Windows/Linux CI run or physical Bluetooth acceptance is claimed.

## Canonical speaker metadata

The speaker-turn candidate keeps `transcript.md` readable and adds root-level `transcript.meta.json` with immutable hash-named metadata parts of at most one MiB. These text files travel through existing encrypted text sync. The manifest binds the meeting UID, exact Markdown digest, revision and every part hash; readers apply richer data only when the complete generation agrees.

Canonical utterances retain IDs, source IDs, actual start/end times, exact authoritative text, timed text spans and attribution/correction provenance. Physical sources are `microphone`, `system_audio` and `room_microphone`; the older immutable `system` alias remains readable. Ambiguous legacy provenance stays unknown. Inferred person labels never select an audio track.

The legacy Segment/Markdown projection loses richer attribution meaning. Its Me/Others labels do not prove who spoke or which archive a relabelled row came from. Current UI, cited questions, summaries, exports and playback use the verified canonical snapshot. Unparsed legacy text and unknown/corrupt metadata are retained; uncertain remapping requires review.

Snapshot and speaker/retranscription journals recover text, names and metadata together. Private ownership metadata permits pruning only proven unreferenced generated parts after successful commit, with complete backup and sync-conflict references protected. Embeddings, voiceprint internals, diagnostic context and raw capture data are not included in portable attribution metadata. See [the candidate's evidence and remaining gates](evidence/speaker-turns-22.md).

### Provisional live attribution

Automatic attribution carries a `provisional` flag. Manifest versions retain both attribution state and original capture/completion provenance:

| Schema | Attribution | Recovery provenance |
| --- | --- | --- |
| 1 | Settled | Absent |
| 2 | Provisional | Absent |
| 3 | Settled | Present |
| 4 | Provisional | Present |

The final reader supports schemas 1–4. The #128 backport supports 1/3 and preserves 2/4 read-only. Earlier readers preserve unsupported generations rather than treating provisional estimates as settled or overwriting completion proof. Unsupported, malformed and conflicting generations retain the existing preservation behavior.

Ordinary saves, rename, reopen and encrypted text sync retain provisional state. Actual final acoustic reconciliation can clear it and write schema 1 without recovery provenance or schema 3 with it. Refinement retains original capture/completion proof, including hidden empty dispositions; it does not downgrade a proven completion to an unproven row. Valid manual corrections remain authoritative; ambiguous automatic remapping remains unknown or requires review. The private ownership ledger and recovery journals keep their existing version: text, parts and manifest still recover as one checked generation at every journal boundary.

Portable metadata contains attribution and original capture/completion proof. Session centroids and owned PCM never enter the sidecar or sync. Provisional person labels do not change utterance IDs, physical source identity or frozen question evidence; interim caption text remains excluded. Recovery suppression and cleanup require the original lane, basename, range and verified WAV hash, independently of person labels and acoustic splits. Failed or unverified recovery audio is retained. See [the audio preservation review](evidence/audio-recovery-cleanup-128-129.md) and [the live-speaker evidence](evidence/live-speakers-99.md).

## Reviewed meeting insights

`insights.json` is the sole portable review store. Schema 1 binds the document to the meeting UID and has a 3,000,000-byte limit, within the existing encrypted text-sync envelope. It contains source-linked proposals, their processing identity and qualification, the finalized evidence cutoff, separate user review/edit state, and note-insertion receipts. It does not change `transcript.md`, personal notes or summary checkboxes merely by accepting an item.

Each source retains the canonical utterance ID, evidence fingerprint, exact UTF-8 byte range and quote, and authoritative start/end time. Owners and original deadline phrases require exact supporting source spans; unsupported fields remain unknown. Exact extraction and whole-passage proposals are distinguished. Typed prior evidence preserves an explicit retraction or changed commitment even when its target cannot be resolved uniquely. Only proven one-to-one lineage can reuse a reviewed identity after transcript replacement; ambiguous sources remain available for review without an invented replacement anchor.

Regeneration uses revision-checked atomic writes. It can replace unreviewed proposals only within explicitly assessed source/kind scopes; omitted or abstained scopes are retained. Edited, claimed, accepted and dismissed items survive regeneration. A changed confirmed commitment becomes a linked proposal. Source replacement marks affected anchors stale and retains the original quote and reassociation history. Unknown versions/fields, malformed documents, oversized documents and sync conflicts are preserved rather than overwritten.

An explicit Add to notes operation first persists a pending receipt with before/intended note hashes and the accepted item revision. The ordinary guarded notes writer acknowledges it only after durable saving. Recovery distinguishes the exact intended notes, unchanged notes and ambiguous intervening edits. Repeated clicks cannot duplicate a confirmed insertion; ambiguous recovery requires an explicit review bound to the currently saved notes hash. Insertion preserves the user's exact edited Markdown. Accepted-item export keeps that body intact and adds a separate original-evidence section with transcript-file references, durable identities, timing, cutoff, destination and current/stale status. Relative transcript-file references require the meeting folder; they are not an application deep link or a claimed Markdown row anchor.

The JSON review APIs, opaque single-use analysis admission and metadata-only invalidation events are shared by Tauri and Expo/iPhone. Admission freezes and qualifies the source/configuration before native credential reads. Logical cancellation keeps physical worker accounting and the library lease until underlying work exits. See [the #57 verification receipt](evidence/reviewed-insights-57.md). Production qualification is empty; persisted fixture review does not establish model quality or enable live/background suggestions.
