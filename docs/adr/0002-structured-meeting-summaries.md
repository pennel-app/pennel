# Attach a structure to each generated summary and keep the Markdown body canonical

Accepted on 10 November 2026 for redesign tasks T9a–T9d. The owner accepted all seven recommendations below; implementation begins with the core record and generation contracts in T9b. Apple Intelligence remains on the Markdown path pending device evidence.

## Context

The redesigned finished meeting shows the personal summary as a lead paragraph, a meeting map, topics, decisions, action items and open questions, with a Brief / Standard / Detailed control and a "Summary by" chip. Today a summary is free Markdown in `meeting.md`. Export, share, sync, mobile, Claude access, search and questions all read that Markdown.

The summary code has rules a structure must keep. Manual generation returns a candidate and writes nothing. Acceptance, edits, history and restore compare revisions. An edited summary is never replaced automatically. The shareable summary is made from the transcript alone. Text models are called as plain chat and return text. That includes CLI agents and Apple Intelligence with its 6,000-byte input, so no provider guarantees valid JSON.

## Decision

### Shape and storage

The Markdown body stays the stored summary. For every summary it generates, core also writes a generation record beside it, `pennel.summary.structure.<body_revision>.json`. `body_revision` is the SHA-256 of the body, the value `meeting.md` already keeps as `pennel_summary_body_revision`. The name follows the history files, keyed by the body instead of the whole document.

```json
{
  "schema_version": 1,
  "meeting_uid": "0192f3a1-7c1e-7b4e-9d55-2f0c8a6e41aa",
  "body_revision": "5d1f0c2a9b7e4d63f8a1c0e2b4d6f8a0c1e3b5d7f9a2c4e6b8d0f1a3c5e7b9d2",
  "provenance": {
    "provider_id": "groq",
    "provider_kind": "groq",
    "model": "llama-3.3-70b-versatile",
    "generated_at": "2026-11-10T10:19:42Z",
    "template_id": "algemeen"
  },
  "structure": {
    "labels": {
      "summary": "Summary",
      "decisions": "Decisions",
      "actions": "Action items",
      "open_questions": "Open questions",
      "none": "None"
    },
    "lead": "The team confirmed next week's planning. The client call moves to Thursday afternoon, and nobody has booked a room for it yet.",
    "topics": [
      {
        "title": "Planning",
        "body": "The planning for next week is confirmed for Monday.",
        "source": { "utterance_id": "0192f3a2-1c00-7000-8000-000000000001", "start_ms": 0, "speaker": "Me" }
      },
      {
        "title": "Client call",
        "body": "The call with Studio Noord moves to Thursday afternoon. Nobody took on the room.",
        "source": { "utterance_id": "0192f3a2-1c00-7000-8000-000000000041", "start_ms": 660000, "speaker": "Sam" }
      }
    ],
    "decisions": [
      {
        "text": "The client call moves to Thursday afternoon.",
        "source": { "utterance_id": "0192f3a2-1c00-7000-8000-000000000047", "start_ms": 730000, "speaker": "Sam" }
      }
    ],
    "actions": [
      {
        "text": "Share the agenda",
        "owner": "Noor",
        "due": "Friday",
        "source": { "utterance_id": "0192f3a2-1c00-7000-8000-000000000023", "start_ms": 370000, "speaker": "Noor" },
        "state": { "done": true }
      },
      {
        "text": "Book a room for the client call",
        "owner": null,
        "due": null,
        "source": null,
        "state": { "done": false, "owner": "Sam" }
      }
    ],
    "open_questions": [
      {
        "text": "Who books the room for the client call?",
        "source": { "utterance_id": "0192f3a2-1c00-7000-8000-000000000052", "start_ms": 940000, "speaker": "Sam" }
      }
    ]
  }
}
```

`source` is null when no transcript line supports the item. `state` holds only what the owner set, and an owner in `state` wins over the generated one. `structure` is null when the summary was made on the Markdown path, and the record then carries provenance only. The speaker lanes of the meeting map are not stored. Core computes them from the utterances when the meeting is read (T9c).

### How a record attaches

A record describes exactly one body text. Reading a summary returns the record only when one exists for the current body revision, with the right meeting id and a known schema version. Anything else, including an unreadable or newer record, counts as no record and is not an error. The view then shows today's Markdown reading view.

Pennel never deletes records, as it never deletes history files. Because a record attaches by body revision and stays on disk, most steps keep today's code.

| Step | Today | With records |
| --- | --- | --- |
| Generate | Returns a candidate with Markdown and writes nothing | The candidate also carries its record. Still nothing is written |
| Discard | Drops the candidate | Nothing to clean up |
| Accept | Saves the Markdown and marks it edited | Takes the record with the Markdown. Core writes it only when its `body_revision` matches that Markdown and its structure renders to it, then saves as today |
| Automatic run, `summarize_meeting` | Generates and saves, not edited | The same, with the record written first |
| Tick an item, add an owner | A body edit | A new core call, described under item state |
| Edit the body | Saves and marks it edited | Unchanged. The new body has no record, so the structure is detached |
| History | Keeps the previous Markdown | Unchanged. Each earlier body still has its record on disk |
| Restore | Writes the earlier Markdown back | Unchanged. The restored body finds its record by revision, item state included |

The record is written before `meeting.md`, in the order history uses. A crash in between leaves a record that no body points to, which is harmless. "Mark as reviewed" accepts the current text without a record, and the body's existing record keeps applying.

### What the model returns

Each transcript line in the prompt starts with a line reference, for example `[u37 12:10] Sam: ...`. The model returns one JSON object with `labels`, `lead`, `topics`, `decisions`, `actions` and `open_questions`. Every item may cite one reference. The model never writes a time or a speaker. Core copies both from the cited utterance, so a time is always inside the recording and a speaker is always the one the transcript shows. Notes and transcript stay wrapped as data, and the prompt says they are never instructions. The existing rules on tentative statements, estimated names and unknown owners stay in the prompt.

### Validation and fallback

Core checks the reply before anything is saved:

- The reply must hold one JSON object with the expected types. A code fence around it is tolerated. The lead must not be empty.
- Lists are cut at 12 topics and 40 items. Item text becomes one line of at most 400 characters.
- A reference must name a line that call received. An unknown reference clears `source` and keeps the item.
- `speaker` is filled only when the utterance is attributed to one person. Unknown, mixed, overlapping and unreviewed speech keeps its time and gets no speaker.
- `owner` is kept only when the name occurs in the transcript, its speaker labels or the typed notes. `due` is kept only when the phrase occurs in the transcript or notes. Otherwise the field is emptied and the item stays. This enforces in code what the prompt already asks.
- Topics are ordered by time. Topics without a time follow, and are left off the map.

When the reply fails the first check, core runs the final call once more with today's Markdown prompt and saves that summary with a provenance-only record. A long meeting reuses its chunk summaries, so the cost is one extra call. Both calls pass the same cancellation and automatic-policy checks. If the second call fails, today's error path applies and the previous document stays.

The structure is display data. The view shows its text as text, links only to validated sources, and nothing in it starts an action: an owner or a due phrase never creates a reminder, a message or a calendar entry. Logs record the outcome code, counts and timings, never content. Nothing is sent anywhere new. The same provider receives the same notes and transcript, plus line references.

### Long transcripts and Apple Intelligence

Above 40,000 characters the summary is built from chunk summaries, which drop timestamps today. The chunk prompt will ask for the supporting line reference at the start of each bullet. Core removes any reference the chunk did not receive before the bullets go on. The final call cites references from the bullets, and core resolves them against the whole transcript. An item whose reference did not survive keeps its text and has no time.

Apple Intelligence follows the same rule through its repeated reduction rounds, where fewer references survive. Its 6,000-byte input also has to hold the structured prompt and the notes, and today's fixed prompt already takes about 2,600 bytes. CI cannot run the on-device model. Apple therefore stays on today's Markdown prompt until a device run shows that the structured prompt fits and parses.

### Typed notes

With typed notes, today's output is enhanced notes: every note line is kept verbatim under the summary section. In the redesigned document the notes sit directly below the summary, and the personal copy will always include them (decided for T5a), so copying them would print them twice. On the structured path the notes are context. The model may use them for the lead, topics, decisions and actions, and no line is copied. An item supported only by notes has no source. The Markdown path keeps the verbatim rule.

### Item state

Ticking an item or adding an owner goes through a new core call that names the action by position and compares revisions like every other save. It requires an attached structure. Core updates `state`, renders the body again, writes the record for the new body and saves `meeting.md` with its source binding and its edited flag as they were. The structure stays attached, the exported Markdown shows the tick, and automatic replacement is not blocked.

When a regeneration replaces a summary, by candidate or automatically, core copies `state` to the new action that cites the same utterance, or else has the same text. State without a match is not carried. The replaced revision and its record stay in history.

### Rendering to Markdown

Core renders the body from the structure with one deterministic function.

```markdown
## Summary

The team confirmed next week's planning. The client call moves to Thursday afternoon, and nobody has booked a room for it yet.

### Planning (00:00)

The planning for next week is confirmed for Monday.

## Decisions

- The client call moves to Thursday afternoon. (Sam, 12:10)

## Action items

- [x] Noor — Share the agenda (Friday)
- [ ] Sam — Book a room for the client call

## Open questions

- Who books the room for the client call? (Sam, 15:40)
```

Headings come from `labels`, which the model writes in the meeting language as it translates headings today. A missing label falls back to English. Action lines keep the form the current prompt asks for. An empty section prints the `none` label. Records attach by body hash, so the renderer can change later without detaching older summaries.

### Provenance

`provenance` records the provider account, its kind, the model, the time and the template. The "Summary by" chip and the template name on the heading line read it. Markdown-path summaries get a record too, so the chip also works for templates with their own sections and after a fallback. A body without a record shows no chip.

### Length

Brief, Standard and Detailed filter what the view shows. They never call a provider and never change the body or an export. The choice is one setting per device, kept with the other desktop view preferences, with Standard as the default. Which sections each level shows is a frontend constant.

### Templates

A template gets the structure when it has no `## ` sections of its own, which is `TemplateSpec.default_sections`. That covers the four built-ins and custom templates that only give instructions. The instruction is still sent. A template that lists its own sections keeps today's prompt and reading view. A customised built-in whose saved text lists headings counts as having its own sections, as it does today.

### Suggestions

The summary step is the only source of the summary's Decisions, Action items and Still open. Suggestions need a qualified model, a known language and a transcript of at most 512 utterances, and from T6b their own consent. The summary has to be complete without them, and its body must not change because the owner reviewed a card.

When both exist, the Review tab shows the suggestions and the summary shows its own rows, so one event can appear in both. Reviewing a suggestion never adds, removes or rewords a summary row. To correct a row, the owner edits the summary. Both sides carry utterance ids, so a later change can pair a row with a suggestion of the same kind and utterance without a new format. Changed commitments appear only in Review.

### Shareable summary

The shareable summary gets no structure. It keeps today's prompt, its own file and its transcript-only source, and it never reads a personal record or item state. Records can hold content derived from typed notes, so they are personal.

### Sync, mobile and Claude access

Sync already accepts the new file name. Records travel only between the owner's devices. Mobile and Claude access keep reading the Markdown body. Mobile has to pass a candidate's record back on accept, which changes the candidate and accept types on both clients. Until mobile does, a summary accepted on the phone is saved without a record and shows as Markdown on the desktop. Older builds ignore the files, and a body they edit detaches like any other edit.

## Consequences

Readers of the Markdown need no change. Edit, restore, history and outside edits of `meeting.md` need no new code, because a record attaches by body hash. The fallback is today's path.

Core has two prompts to maintain. A rejected structure costs one extra provider call. The candidate and accept signatures change on desktop and mobile. Item state adds a write path. Records pile up in the meeting folder the way history files do. The shareable Markdown keeps three sections while the personal one has more. T9b adds "Summary structure" to `GLOSSARY.md`, and T9d updates the reading-view contract in `DESIGN.md`.

Left out on purpose: a structure for the shareable summary and for templates with their own sections, editing a due date, adding or removing rows without Edit, pairing rows with suggestions, a different summary text per length, and a "Summary by" chip on a hand-edited summary.

## Alternatives considered

Parsing the model's Markdown into a structure would need no second call. Headings are translated into any meeting language and models vary their list syntax, so the parser would be guessing.

One fixed file with the body revision inside, like the shareable summary, is simpler to write. Restore would then need the structure copied into every history entry, and a sync conflict on either file could pair a body with the wrong structure.

Timestamps written by the model cannot be checked. Any value parses, two utterances can start in the same second, and chunk summaries drop times. A reference can be checked against the lines the call received.

Keeping ticks outside the body would leave the stored Markdown untouched. Export, mobile and Claude access would then show boxes the owner already ticked as open.

Making the structure the stored summary and the Markdown an export would break every reader of `meeting.md`, including people who edit the file in another app.

## Migration

No migration is needed. Existing meetings have no record and keep today's reading view until the owner generates a new summary. No file is rewritten, `meeting.md` gains no field and the history format does not change.

## Test plan

Rust tests in `core/` prove the invariants, written before the code:

- A structured candidate writes nothing, and discarding it leaves no record.
- Accept writes a record only when it matches the accepted Markdown. A mismatched record is dropped and the Markdown is still saved.
- Editing the body detaches the structure. Restoring the earlier revision attaches the same record again.
- Ticking an action keeps the structure attached, the source binding unchanged and automatic admission open.
- A regeneration carries item state to the action that cites the same utterance.
- An unknown reference keeps the item without time or speaker. References a chunk never received are removed, on the chunked and the Apple path.
- An owner or due phrase the source does not contain is emptied.
- A malformed reply falls back to the Markdown prompt with one extra call, and revoking the automatic policy between the two calls cancels.
- Shareable generation sends today's prompt and its output is the same with and without personal records.
- A template with its own sections uses the Markdown prompt and gets a provenance-only record.
- Record names are syncable and follow a meeting rename.
- A record with an unknown schema version or another meeting's id is ignored without an error.
- The structured prompt wraps notes and transcript as data and fits the Apple budget.
- The rendered body keeps the action line form and the model's labels.

## Accepted recommendations

1. With typed notes, may the built-in templates stop copying note lines into the summary? Recommendation: yes. The notes sit below the summary and in the personal copy.
2. May the summary and the suggestions stay independent, with the summary step as the only source of its rows? Recommendation: yes. Pair them later if owners find the overlap confusing.
3. When an automatic run replaces a summary whose items you ticked or assigned, should it carry that state to matching items, or skip the run? Recommendation: carry it. The replaced version stays in history.
4. May the shareable summary stay on today's three-section prompt? Recommendation: yes. Giving it topics is a separate task with its own privacy tests.
5. May Apple Intelligence ship on the Markdown path until a device run passes? Recommendation: yes.
6. Is the length choice one setting per device with Standard as the default? Recommendation: yes. The prototype hides Still open until Detailed, and showing it from Standard would be safer.
7. Is it acceptable that a hand-edited summary shows no "Summary by" chip? Recommendation: yes for now. No record describes that text.
