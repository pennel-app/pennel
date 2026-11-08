# Writing profiles

Tracking: [#184](https://github.com/pennel-app/pennel/issues/184). Writing profiles are available in the regular desktop app and Pennel Preview, with learning and sharing off by default. Automated checks, real connector acceptance and owner style evaluation are separate gates; availability alone does not establish that generated drafts sound like the owner.

## Learn and review

In **Settings → Meetings → Writing profile**, enable learning after reading the transcript-processing disclosure. Pennel analyzes eligible original speech transcripts with the configured text model. A remote text provider receives those eligible transcripts; a local provider keeps that analysis local. CLI-agent providers are unavailable for profile extraction because extraction must run without tools. No provider or credentials are substituted automatically.

New successful ordinary dictations can contribute after opt-in. Command/rewrite requests, AI-cleaned output and ambiguous old dictation history cannot contribute. Eligible finalized meeting speech can be backfilled, but only stable owner attribution qualifies: microphone capture alone, a speaker name such as “Me”, provisional attribution and overlapping voices do not establish ownership. Owner enrollment must remain available. Changing the enrolled owner invalidates earlier ownership bindings; affected legacy meetings need fresh attribution before contributing again.

Evidence is separated by spoken language, independently of interface and speech-recognition preferences. Unknown or mixed-language samples do not contribute. Regional variants share their base language. Initial readiness requires 1,000 eligible words across at least three capture sessions and repeated support for a trait. Several passages from one meeting count as one independent session.

Pennel learns six constrained traits: directness, detail, register, sentence rhythm, clause structure and hedging. It does not infer email sign-offs, punctuation or document layout from speech. Review the source list, exclude or reinclude a whole capture session, and correct traits using the offered values. Exclusion covers every present or future analysis chunk from that dictation or meeting. Corrections persist through later learning. An Automatic selection removes that explicit correction.

Background analysis runs in bounded batches while the desktop is idle; Refresh requests another batch. Pause stops new analysis and keeps the learned profile. Source deletion, retention expiry and changed attribution still remove outdated influence while paused. Delete profile clears evidence and corrections and stops learning and sharing. Re-enabling learning starts a fresh opt-in period.

The local `writing-profile.json` stores source references, fingerprints, constrained traits, exclusions and overrides, not another copy of transcript text. It is outside meeting sync. Preview data, meeting libraries and credentials remain isolated from production.

## Connect an assistant

1. Sign into Pennel Cloud from the desktop app.
2. In the Writing profile section, enable assistant sharing. This explicitly designates this computer as the profile source.
3. Add the displayed **writing-profile connector URL** to ChatGPT or Claude and authorize profile reads. The regular app uses `https://api.pennel.app/mcp/style`; Preview uses `https://api-staging.pennel.app/mcp/style`. Authorize the matching account and environment. Existing Preview connections continue to read Preview; installing the regular app does not move their authorization or data.
4. Enable Pennel in the chat and ask it to use your profile for the requested output language before drafting.

The designated computer must be running. Selecting another computer changes the source explicitly; there is no fallback to whichever computer happens to be online. Learning and sharing are separate controls. Disable sharing to stop new profile responses; pause learning to retain and share the current guide without processing new speech.

The connector exposes only `get_writing_profile({"language":"en"})` at `/mcp/style`, with the independent `writing_style:read` permission. Its bounded response includes a format version, revision, requested base language, readiness, evidence counts, available languages and fixed-rendered guidance. Insufficient evidence produces no usable guidance. Retrieval does not call a text model. It cannot search meetings, read notes, retrieve source quotations, change the profile or send a message. Existing meeting grants remain separate.

Cloud persists designation and authorization metadata; profile responses pass through the relay in memory. Source changes, account changes, disabled sharing and source invalidation are checked before returning a response. Already delivered text can remain in an assistant's conversation; stopping sharing prevents further retrieval.

## Acceptance

First verify actual profile-tool calls in ChatGPT web and Claude web. Record the app commit/artifact, environment, OS, assistant and profile revision without retaining private source content in public evidence.

For each assistant, compare ten personalized drafts against ten drafts from the same assistant without a profile. Use fresh conversations, identical task briefs, no source-recording topics, and randomized A/B ordering. Cover emails, messages and longer text. The owner records which sounds more like them before revealing A/B identity. At least eight personalized drafts must be preferred per assistant, and every draft must follow the brief without importing facts from recordings. The owner is the judge; automated checks cannot complete this criterion.

Use [the blank evaluation sheet](evidence/writing-profile-owner-evaluation.csv) locally. Keep completed sheets and drafts private. Only aggregate outcomes belong in the issue. These are owner acceptance criteria, not evidence of quality for other people or untested languages.
