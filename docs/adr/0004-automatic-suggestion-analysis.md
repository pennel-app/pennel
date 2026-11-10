# Run suggestion analysis after a meeting under its own consent

Status: proposed on 10 November 2026 for the owner's sign-off. Nothing in this note is built. It belongs to task T6b of the desktop redesign.

## Context

The redesigned finished meeting has a Review tab beside the document. It shows suggestion cards and a count of the open ones, and the design expects them shortly after Stop. Today suggestion analysis starts only from Find suggestions in the Suggestions view (`apps/desktop/src/reviewed-insights.ts`). The owner first picks a content language, and the core admits the run only when the optional decision connection in `settings.decision` matches an adopted qualification record for that task, language, model and route (`core/src/meeting_qualification.rs`, `core/src/meeting_insight_engine.rs`). On 10 November 2026 the owner decided that analysis runs automatically after Stop, under its own consent, and is built after the Review layout (T6a).

Summaries already have an Unanswered, Manual and Automatic policy with a durable attempt marker and policy re-checks (`core/src/settings.rs:47`, `core/src/meeting_summary.rs:576`), and the desktop runtime calls it once when a meeting finishes (`apps/desktop/src-tauri/src/meeting_runtime.rs:1024`). The automatic summary holds the meeting's edit guard for its whole run and suggestion admission needs the same guard, so the two cannot overlap on one meeting. The bundled registry `assets/qualifications/meeting-assistance.json` has no adopted record, so no setup can run suggestion analysis today, by hand or automatically.

## Decision

### Policy and setting

`Settings` gains two fields, both with `#[serde(default)]`:

```json
{
  "summary_automation": "manual",
  "suggestion_automation": "automatic",
  "suggestion_language": "nl"
}
```

`suggestion_automation` is a new enum, `SuggestionAutomation`, with the values `unanswered` (the default), `manual` and `automatic`. It is a separate type from `SummaryAutomation` so that one consent cannot be read as the other. `suggestion_language` is `null`, `"en"`, `"nl"` or `"mixed"`. It is stored as an optional string and validated when used, like `Settings.language`. New and upgraded profiles are Unanswered, and `summary_automation: "automatic"` grants nothing here.

### What the run reads

The automatic run sends what Find suggestions sends: each finalized, non-failed utterance with the one before and the one after it as context (`windows()` in `meeting_insight_engine.rs:49`). It sends no typed notes, summary, live captions, speaker names or meeting title. The prompt marks the utterances as untrusted data. The answer is a closed set of choices plus verbatim spans that are checked against the source. The result is a list of proposals that only the owner can accept, and nothing is written to the notes or the summary.

### When it runs, and in what order

The desktop runtime calls a new core entry point, `admit_automatic_meeting_insight_analysis(id)`, in `postprocess` directly after `summarize_meeting_automatically` returns, whatever that call returned. The order after Stop is speaker recognition, automatic summary, automatic suggestions. Capture and transcript finalization are complete before any of them start.

The suggestion run cannot delay the summary. Because the two run one after the other, they never send automatic requests to one provider at the same time, even when `llm` and `decision` use the same account. A failed or skipped summary does not stop the suggestion run, since each consent stands alone.

The run uses the existing admission, job and commit path at the existing explicit-analysis priority. That keeps the current bounds: one job per meeting and two in total, one request at a time, one deadline for the whole run equal to the connection's timeout (90 seconds at most), a limit of 512 utterances, and nothing saved unless every window finished. Speech work has priority: a dictation or a meeting transcription cancels a request in flight, which ends the run. A question has priority too, and the run's next request waits while one is in flight. If speech or a question is in progress when the run would start, the run is refused and is not queued. Credentials are read only after admission, as `begin_insights_analysis` does today.

### Language

Automatic analysis has no moment to ask, so the core resolves the language policy:

1. The meeting's recorded speech language (`pennel_stt_language`, frozen when recording starts): `en` gives `en` and `nl` gives `nl`.
2. If that is `auto` or missing: `suggestion_language`, when it is `en`, `nl` or `mixed`.
3. Otherwise the language is unknown.

Unknown means no automatic run and no marker. The Review tab then shows the language choice and Find suggestions. A meeting recorded in another explicit language, such as French, is always unknown, because the fallback must not relabel it. Mixed can only come from the owner, through the fallback or a manual choice. The code stores no detected language, so there is nothing else to resolve from. The language selects the qualification record and does not change the prompt.

### No qualified model

The automatic run starts only when the registry admits the exact setup for the resolved language and the matching record has `foreground_impact_accepted`, because nobody is waiting for this run. Otherwise it returns before the marker, before credentials and before any request. The Review tab gives the reason with the existing texts (`insights.reason_unconfigured`, `insights.reason_unqualified` and the others) and offers the existing Connection settings action. It never selects another provider, relaxes qualification or sends a transcript in order to look ready.

### Attempt marker

On admission, before the first request, the core creates `pennel.suggestions.automatic.<transcript_digest>.json` in the meeting folder with `create_new`, beside the summary's `pennel.summary.automatic.*` marker. It holds `schema_version`, `meeting_uid`, `transcript_digest`, `language_policy` and `admitted_at`. When the job ends, the core rewrites it atomically with `outcome` (`completed`, `failed` or `cancelled`), `error_code`, `items_found` and `finished_at`. A marker without an outcome means the app stopped during the run. The marker holds identifiers, codes, counts and times, and no meeting text. It cannot live in `insights.json`, because that file rejects unknown fields and older builds would open it read-only.

An existing marker for the current digest ends the call. A second finish event, a restart or another host cannot repeat the work. A failed attempt is retried only when the owner uses Find suggestions.

### Stale and duplicate work

The existing guards do not change. The job freezes the meeting UID, the transcript snapshot, the configuration, the credential epoch and the account. It checks them again before every request and under the meeting's edit guard at commit, finds the meeting by UID after a rename, and merges with a store revision check. The automatic job adds the policy to those checks: if the owner leaves Automatic during the run, it stops and saves nothing. The check is sticky, so switching back within one request does not revive the run.

Retranscription holds the meeting's edit guard, so the run is refused while it works. A run commits only if every utterance it read is unchanged, so a transcript replaced during the run ends it as `source_changed`. Passages that failed-transcription recovery adds are not analysed by a run that started before them. In each case the transcript ends with a digest that differs from the marker's. Existing cards keep their stale-source handling, the Review tab says that the transcript changed since the last analysis, and the owner starts a new pass. The only automatic trigger is `postprocess` after a successful Stop. A save that failed and was retried does not reach it today, for the summary either.

Today a change to `summary_automation` cancels every in-flight inference request (`core/src/engine.rs:2497`). With two cards on screen, answering the summary card would cancel a running suggestion job. The policy PR gives each policy its own revocation counter, which only that policy's automatic jobs compare, and takes `summary_automation` out of the global list.

### Consent card

When the policy is Unanswered, a qualified setup exists and the meeting has a finalized transcript, the Review tab's empty state shows a card modelled on the summary card:

- Title: "Find suggestions in this meeting?"
- Text: "Pennel sends the transcript to {destination} to look for actions, decisions and open questions. Your notes are not sent. Suggestions are saved on this computer."
- A language choice (English, Dutch, English and Dutch mixed), only when this meeting's language resolves to unknown.
- Checkbox: "Do this after every meeting".
- Buttons: "Find suggestions" and "Not now".

The destination is the real endpoint and model from the availability check, formatted as the Suggestions view formats it today.

| Control | Effect |
| --- | --- |
| Checkbox on, Find suggestions | Saves `automatic`, and the chosen language as `suggestion_language` when the card asked for it. Then it starts this meeting's run through the manual path, because the run at Stop already returned at the policy check. |
| Checkbox off, Find suggestions | Saves `manual` and runs once. |
| Not now | Saves nothing. The card becomes a quiet line with Find suggestions. Whether the dismissal is remembered follows the answer given for the summary card in T5b. |

Showing the card reads configuration and qualification only. It does not save a policy, read a credential or contact a provider.

### Settings control

The Meetings page in Settings shows the two policies under one heading, "After a meeting": the existing summary control, then a suggestions control built the same way. The suggestions control has one line of explanation with the real destination, the buttons "Automatically" and "I'll find suggestions", the reason and the Connection settings link when no qualified setup exists, and a select "When speech language is automatic" (Ask each time, English, Dutch, English and Dutch mixed) bound to `suggestion_language`. The owner can choose a policy before a setup is qualified, and it has no effect until one is.

### What the Review tab shows

| State | Shown |
| --- | --- |
| Unanswered | The consent card. Without a qualified setup: the reason and Connection settings, and no card. The tab shows no count. |
| Manual | One line with the destination and Find suggestions. The language select starts at the resolved language. |
| Automatic, running | "Looking for suggestions…" from Stop until the run ends, with the existing progress line and Cancel once the job exists. |
| Done with items | The cards and the count of open ones. |
| Done with none | "No suggestions found in this meeting.", the existing incomplete-coverage notice, and Find suggestions as a secondary action. |
| Failed | The reason from the marker's code, "Nothing was changed.", and Find suggestions. There is no automatic retry. |
| Not run | Automatic, processing finished and no marker: the availability reason (no qualified setup, unknown language, no transcript, meeting too large, busy) and its action. |

The snapshot that the tab already reads gains `automatic_attempt` (the marker's content) and `resolved_language`. To tell waiting from not run, the tab uses the in-progress signal that T5b adds for the summary if it spans all of `postprocess`. Otherwise the runtime keeps a per-meeting processing flag.

### Scope

This note covers analysis after Stop. Suggestions during a meeting (#97) keep their own control, their own qualification use case and the background scheduler, and `suggestion_automation` does not enable them. The setting and the core entry point are shared with mobile. This note wires the desktop runtime only, and the iPhone app keeps Find suggestions.

## Consequences

- Until a qualification record is adopted, nobody sees the consent card or an automatic run. T6b delivers the policy, the gate and the explanations. A record matches one exact app version, platform, model and language, so every release needs its own record.
- A long meeting can end as failed (the 90-second deadline) or too large (512 utterances). Both limits are part of the qualified context, so changing them needs new qualification evidence and a decision of its own.
- During back-to-back meetings the next recording's transcription cancels the previous meeting's run. The owner sees the reason and can run the analysis later.
- One automatic run sends up to 512 requests to the connection the owner chose.
- `DESIGN.md` changes in the same PRs. The automatic-summary paragraph gains the suggestion policy, and the sentence that experimental connections serve setup and testing only gains this exception. The text "Analysis runs only when requested" (`insights.qualified`) changes.
- `GLOSSARY.md` gains "Suggestion analysis": a pass over a meeting's finalized transcript that proposes suggestions. It reads no typed notes. It runs when the owner starts it or, under separate consent, once after a meeting ends.
- Every new string exists in English and Dutch, and the UI is the same on every platform.

## Alternatives considered

None of these is proposed.

- One combined consent for summaries and suggestions. The two read different things (typed notes and transcript against transcript only), can go to different destinations and become available at different times. Owners who already chose automatic summaries would also get suggestions switched on by an upgrade, which the policy's own rule forbids.
- A new value on `summary_automation`. Startup parses settings strictly (`load_strict`), so an older build would fail to start on a value it does not know.
- A new qualification use case for automatic runs. The request, prompt, schema and metrics are identical, so a second record would duplicate the evidence. Automatic runs differ only in foreground impact, and the record already has a field for that.
- Detecting the language from the transcript text. A wrong guess runs a model on content it was not qualified for.
- Running suggestions before or beside the summary. Before delays the summary, which #98 rules out. Beside needs the summary to give up the edit guard that protects its save.
- The background priority and the live coordinator. They are built for rate-limited batches bound to a visible live workspace.
- Running again by itself after a failure or a transcript change. #98 allows no automatic retries, and the summary treats an attempt as consumed.

## Migration and downgrade

No data is migrated. `insights.json` and its schema version do not change. A profile without the new fields loads as Unanswered with no fallback language. `Settings` ignores unknown fields, so an older build starts normally on a newer file. It drops both fields the next time it saves settings, and after a later upgrade the owner is asked again. Losing the choice only stops automatic work, so that direction is safe. The enum never gains a fourth value, since a newer value would stop older builds at startup. A later need becomes a new field. Marker files are unknown files to an older build. It syncs them like any other meeting file and never reads them.

## Test plan

Rust, in `core/`:

- Settings: the default and a missing field are Unanswered, the wire names are stable, an old file passes `load_strict`, an unknown `suggestion_language` is tolerated, and automatic summaries do not imply suggestion consent.
- Language resolution as a table: `en`, `nl`, `auto` with and without a fallback, `fr` with a fallback, a meeting without recording metadata, and an invalid fallback.
- Admission: Unanswered, Manual, an unconfigured or unqualified setup and a record without `foreground_impact_accepted` each make no marker, read no credential and send no request.
- Marker: it exists before the first request, a second call and a new engine on the same folder send nothing, it stays consumed after a failure, it records each outcome, and it holds no transcript text.
- Policy: revoking during a run cancels it and saves nothing, even when the policy is switched back within one request. Changing the summary policy does not cancel a suggestion job, and the reverse.
- Source: replacing the transcript during a run commits nothing, and admission is refused while retranscription holds the meeting.
- Content: sentinels in the notes, the title and a speaker name never appear in a request.

Rust, in `apps/desktop/src-tauri/`:

- `postprocess` calls the summary, then the suggestions, and neither while quitting.
- A fresh profile finishes a meeting without analysis.
- Credentials are read after admission.
- Events carry metadata only.

TypeScript, in `apps/desktop/`:

- A pure function maps policy, availability, attempt, job and snapshot to the states in the table above, with a table-driven test.
- The card mapping: checkbox and Find save `automatic` and then start, Find alone saves `manual`, Not now saves nothing, and rendering the card calls neither save nor start.
- Without a qualified setup there is no card and the setup action is visible.
- English and Dutch parity, interaction checks for each state, and renders in light and dark at 1280×860 and 760×520.
- The mobile type mirror with its type and unit checks.

## Pull requests

1. Core policy: `SuggestionAutomation`, `suggestion_language`, language resolution, the per-policy revocation counters and the mobile type mirror. Nothing visible changes.
2. Core run: the automatic entry point, the marker with its outcome, the policy re-checks and the two snapshot fields.
3. Desktop runtime: the call after the summary in `postprocess` and the processing signal.
4. Desktop UI: the consent card, the Review tab states, the Settings control, English and Dutch copy, `DESIGN.md`, `GLOSSARY.md` and the before and after renders.

## Questions for the owner

1. No setup is qualified today, so T6b stays invisible until a record is adopted. Build it now? Recommendation: yes, and keep the gate as it is. The work is small once T6a exists, and without it the Review tab shows an empty list with no explanation.
2. One consent or two? Recommendation: two, shown in the same form and placed together in Settings.
3. Accept the second setting, `suggestion_language`, asked on the card when the speech language is automatic? Recommendation: yes. Automatic is the default speech language, and without the fallback most owners would never get an automatic run.
4. Require `foreground_impact_accepted` on the record for automatic runs? Recommendation: yes, because the registry asks for it everywhere except where the owner starts the run and waits.
5. After Retry failed transcription or Retranscribe recording, should analysis run again by itself? Recommendation: no. It matches the summary, and the tab offers Find suggestions.
6. Accept the 90-second deadline and the 512-utterance limit for the first version? Recommendation: yes. Measure how often long meetings fail before reopening the qualified context.
7. Desktop only for now? Recommendation: yes. The iPhone app keeps manual analysis until its own acceptance.
