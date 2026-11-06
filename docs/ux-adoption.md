# Usability and adoption study kit

Status: protocol prepared; no participant sessions or adoption outcomes are recorded here. The targets below are initial product decisions, not industry benchmarks or evidence that Pennel has met them. Track implementation and review in [issue #32](https://github.com/pennel-app/pennel/issues/32).

The separate [desktop owner-review kit](desktop-ux-owner-review.md) prepares the three current UX decisions with isolated browser scenarios and a later Pennel Preview pass. Owner feedback and synthetic task runs do not count as participant rounds or adoption evidence.

## Purpose and sequence

Evaluate whether nontechnical professionals and small teams can use Pennel for writing and meetings without understanding models, providers or API keys. Observe task completion separately from visual appeal. English, Dutch and mixed Dutch–English work receive explicit coverage; interface language and speech language are separate variables.

| Stage | People and duration | Evidence and gate |
| --- | --- | --- |
| Owned regression checks | No participants | Synthetic UI/audio/storage fixtures establish tested code paths only. Record candidate revision, configuration and results. |
| Formative round 1 | Five nontechnical professionals; 45–60 minutes each | Find first-use and everyday-flow barriers, then revise the candidate. This is qualitative research. |
| Formative round 2 | Five new nontechnical professionals; 45–60 minutes each | Repeat the same task goals on the revised candidate. Target at least four unassisted successes out of five valid attempts for each critical task. |
| Existing device pilot | 3–5 Mac users; one working week | Complete the separate [daily-use pilot](improvement-pilot.md) and exact-candidate hardware, privacy and recovery gates. Formative participants may volunteer, but record this as a distinct stage. |
| Adoption cohort | 15–20 volunteers; four weeks | After the relevant candidate/device gates pass, collect one voluntary check-in per week. Target at least 60% using Pennel on three or more days in week four. |

Windows and Linux enter participant use only after their own package/device acceptance. Do not pool synthetic, supervised preview, installed-package, real-device and cohort observations into a single pass claim. Keep the frozen native application available until the existing replacement gates pass.

## Preparation

- Assign a study owner and facilitator. Use participant codes such as R1-P01; keep contact details outside the repository and separate from observations.
- Use isolated Pennel Preview profiles, owned meeting folders and unsent test drafts. Record the commit, package SHA-256, OS, hardware, audio devices, interface/speech languages and selected processing routes. Never record keys, recovery codes, transcript contents, participant names or private file paths in the worksheets.
- Use invented, nonconfidential material: a short email, a two-person project discussion with a decision and an explicit next action, and a source-backed question. Prepare English and Dutch equivalents, including a brief mixed-language example. Record which task variant was used, not what the participant said.
- Configure any provider used for a study explicitly in advance under the owner's authorized test account. A study does not authorize creating accounts, purchasing service, entering participant credentials or sending private work to a provider. The ordinary local/account-free path remains available.
- Run the exact task script with a colleague first. Check that the chosen candidate can complete each task and that controlled failures are confined to test stores/endpoints. Never fill a real disk or damage a participant's database to create a failure.
- Include an actual Preview-window accessibility pass at 200% content enlargement. Zoom in/out with `Cmd+=` / `Cmd+-` on Mac or `Ctrl+=` / `Ctrl+-` on Windows/Linux; the `=` key supplies zoom in / plus without Shift. Pill/prompt windows stay fixed. Check readable content, reachable actions and visible focus; browser scaling or shortcut enablement alone is not hardware, VoiceOver or participant acceptance.
- Default to observation notes without audio or screen recording. If additional evidence is necessary, obtain specific consent and capture only the owned Preview window with synthetic material. Never capture the user's whole screen or unrelated applications.

For each round, recruit a mix of writing-heavy and meeting-heavy work, English- and Dutch-speaking people, and people with relevant accessibility needs. Do not recruit only developers, current AI enthusiasts or contributors. Allocate round 1's first task to three dictation-first and two meeting-first sessions; reverse that allocation in round 2. Each person then attempts both everyday workflows. Show the first-task result across all five people and report the two goal subgroups with their actual small denominators; never label either subgroup as a five-person validation.

## Session script

Tell the participant: “We are testing Pennel, not you. Please try the tasks as you normally would and tell us what you expect to happen. There are no right opinions. You can stop at any time.” Explain the chosen processing routes and what observations will be kept before asking for consent.

Allow about five minutes for context, 35–45 minutes for tasks, and five to ten minutes for visual impressions and debrief. Present one goal at a time. Do not name a button, tab, shortcut or settings category unless the task is specifically testing that named control.

| ID | Neutral task prompt | Success evidence |
| --- | --- | --- |
| F1 | “You have just installed Pennel. Use it for the first job on your task card.” The card asks either for a voice-written unsent email or notes from the supplied short conversation. | Correct task setup and an actual useful text/transcript result. Opening a recorder or marking onboarding complete alone is not a successful meeting task. |
| D1 | “Add the supplied sentence to this unsent email using your voice.” | Text reaches the intended field once. No unexpected recipient, focus change or replacement text. Record recognized/cleaned/delivered outcomes separately. |
| P1 | “Before you use this with work material, explain where the sound and written text would be processed with these choices.” | The explanation matches the selected audio and text routes. Local recognition must not be mistaken for local cleanup when cleanup is remote. |
| M1 | “Capture this conversation, add one note of your own, and finish when the conversation ends.” | Capture state is understandable; the saved meeting has usable transcript material and retains the personal note. Check real audio separately from synthetic scenarios. |
| M2 | “That meeting is finished. Capture the next short conversation.” | The participant independently finds a way to start the next recording; the first meeting remains available. |
| S1 | “Turn the saved conversation into a concise account of its decisions and next steps.” | The participant completes any necessary text setup and obtains a useful result in the same meeting, without losing personal notes. Record any invented or altered commitments without copying their content. |
| S2 | “Change Pennel's appearance, then continue reviewing the same point in this meeting.” | Settings stays in the main window. Returning preserves the selected meeting, view, reading position and personal notes; reopening Settings retains its category. During a separate active-recording scenario, the same detour must not stop or replace capture. |
| N1 | “Prepare an editable draft in Pennel. Look up an earlier dictation, then continue the draft.” | Dictation, Meetings, History and Settings remain in one main window. Saved pad text and the previous section context survive the roundtrip, and a recovery link selects the intended History entry. A listening/processing pad remains intact until explicitly finished or cancelled. |
| V1 | “Check what supports the marked point, then return to what you were reviewing.” | The supporting passage is correct and the participant returns to the original note or answer with its context intact. |
| E1 | “Prepare these notes for use outside Pennel in an editable form.” | A usable copy/export is obtained from the intended meeting. Keep the result local or in an unsent test draft; nothing is sent or published. |
| R1 | “The previous operation did not finish. Keep the useful work and continue.” Use a prepared provider/insertion failure in the owned test environment. | The participant finds retained text/transcript and a safe retry, copy or setup route. No repeated paste, lost draft or unexpected provider change. Mark fixture-based recovery separately from a live failure. |

Critical tasks are F1, D1, M1, M2, S1, S2, N1, V1, E1 and R1. P1 is a separate privacy-comprehension gate: misunderstanding a processing destination must be resolved before private-material use. Do not manufacture a nominal five-person denominator by counting tasks not actually attempted.

For a meeting-first session, the real capture in F1 can also supply M1's evidence; do not ask for another recording before M2. Reference that same observation in both task rows rather than claiming two separate captures. Dictation practice in F1 does not supply D1's evidence: insertion into the unsent receiving-app draft is a separate task.

After each task, ask “How easy or difficult was that?” on a 1–7 scale, where 1 is very difficult and 7 is very easy. Then ask “What did you expect?” when an observed hesitation needs explanation. Avoid suggestions such as “Would a bigger button help?”

If someone becomes stuck, first allow time and ask what they are looking for. If help is needed to continue, give it, record the intervention and mark the task assisted. Do not convert an assisted result into an unassisted pass because the participant eventually finishes.

Ask about looks only after the task sequence: “What drew your attention first?”, “What was easy or difficult to read?”, “Did anything feel out of place?” and “What would you change?” Record these responses separately from task outcomes. A positive impression does not cancel a failed task.

## Observation and decisions

Use [session-sheet.csv](usability/session-sheet.csv), one row per participant/task, and [round-summary.csv](usability/round-summary.csv), one row per task/candidate. Keep actual study records in private owner-controlled storage, not in GitHub or the source repository. The committed CSVs are blank templates.

Result codes are **U** (goal completed without a helpful hint), **A** (completed after assistance), **F** (failed or abandoned) and **NA** (not attempted or inapplicable, with reason). Record active task time and model-download waiting time separately; retain both elapsed values. Note only sanitized behavior such as “looked in another window” or “could not find the next recording,” never dictated words or meeting details.

For each critical task in a round, publish the valid denominator, U/A/F counts, and omissions. Admission requires at least **4 U out of 5 valid attempts** on the current candidate. Fewer than five valid attempts is incomplete evidence, not a pass. First-goal subgroups remain descriptive because they contain only two or three participants per round. Do not pool different candidate revisions to reach the target.

After round 1, prioritize barriers that block completion, hide or lose work, misrepresent capture/processing, or prevent accessibility. Record the change and relevant automated regression before round 2. Use new participants for round 2 so familiarity does not masquerade as a better first-use flow. Small qualitative rounds do not establish a population success rate or a statistically significant competitor ranking.

Silent data loss, wrong-target insertion, falsely reported capture success or a wrong processing destination blocks expansion regardless of the task-count target. An unresolved accessibility blocker needs its own recorded repair and retest. The real [desktop acceptance tasks](desktop-roadmap.md) and daily-use pilot remain required.

## Four-week voluntary cohort

Enroll a fixed group of 15–20 people who expect to write and meet during the study and who choose to use an accepted candidate. Record the enrollment count and dates before starting; do not replace nonusers to improve the result. Each person's weeks are days 1–7, 8–14, 15–21 and 22–28 after starting. Use [adoption-check-in.csv](usability/adoption-check-in.csv), with one row per participant/week.

Ask once per week, manually:

1. On how many days in the last seven did you use Pennel for dictation?
2. On how many days did you use it for meetings?
3. On how many distinct days did you use either? Count a day only once even if you used both.
4. Did a problem prevent you using it? Choose setup, capture, output quality, delivery, finding a control, accessibility, performance, other, or no problem. Describe only the behavior, without work content.
5. Would you choose to keep using Pennel next week? Why, without private details?

Keep all counts in 0–7 and check that days using either is at least the larger individual count and no greater than their sum or seven. No background counters, telemetry, automatic uploads, content samples or compulsory daily reports are added.

The initial adoption target is:

`week-four retention = participants reporting at least 3 distinct use days in days 22–28 / original enrolled participants`

Pass at **60% or more**: at least 9 of 15 or 12 of 20, rounding the required count up for other enrollment sizes. Nonresponse is not evidence of retained use; show check-in response rate alongside retention. Report dictation and meeting use separately so combined activity cannot hide an unused workflow. Record candidate changes during the cohort; do not describe a mixed-version result as acceptance of one release.

Participation and check-ins are optional. Explain withdrawal and deletion before enrollment. Keep only the agreed nonidentifying enrollment count for the fixed denominator; if consent requires changing it, disclose that change and its effect on the result. Remove individual records on request. As a study default, delete contact details 30 days after the last agreed follow-up and coded raw observations/check-ins 90 days after their study phase ends; retain only anonymous aggregate findings. Confirm the storage owner and these dates before recruiting.

## Invitation drafts

These are drafts for the study owner to review and send; this protocol sends no messages or invitations.

**Formative session — English**

> We are testing Pennel, a desktop app for dictation and meeting notes, with people who write and meet at work. Would you be interested in a 45–60 minute session? You do not need experience with AI tools. We will use invented material and an isolated preview; no account or private work is required, and we do not record your screen by default. We want to see what is clear and what gets in your way. Participation is optional and you can stop at any time. If interested, tell me your preferred language and any access needs so we can arrange a suitable session.

**Formatieve sessie — Nederlands**

> We testen Pennel, een desktopapp voor dicteren en vergadernotities, met mensen die voor hun werk schrijven en vergaderen. Heb je interesse in een sessie van 45–60 minuten? Je hoeft geen ervaring met AI-tools te hebben. We gebruiken verzonnen materiaal en een aparte preview; je hebt geen account of privéwerk nodig en we nemen je scherm niet standaard op. We willen zien wat duidelijk is en wat je in de weg zit. Meedoen is vrijwillig en je kunt altijd stoppen. Laat bij interesse weten welke taal je verkiest en of we rekening moeten houden met toegankelijkheidsbehoeften.

**Cohort — English**

> We are looking for volunteers to try an accepted Pennel preview during four weeks of ordinary writing and meetings. Once a week we will ask a few short questions about the days you used it and any obstacles. Pennel will not send usage reports or work content automatically. You decide when to use it, which processing route to choose, and whether to answer a check-in. Would you like the study details before deciding?

**Cohort — Nederlands**

> We zoeken vrijwilligers die vier weken lang een goedgekeurde Pennel-preview willen proberen tijdens hun gewone schrijfwerk en vergaderingen. Eén keer per week stellen we een paar korte vragen over de dagen waarop je Pennel gebruikte en eventuele hindernissen. Pennel verstuurt niet automatisch gebruiksrapporten of werkinhoud. Je kiest zelf wanneer je de app gebruikt, welke verwerking je kiest en of je een check-in beantwoordt. Wil je de onderzoeksinformatie ontvangen voordat je beslist?

## Evidence record and references

Each handoff states what changed, candidate revision and package hash, checks actually run, participant counts/results, and remaining gates. Keep open human/device criteria open in [issue #32](https://github.com/pennel-app/pennel/issues/32) and their owning acceptance issues. Preparing this kit, rendering a prototype, passing unit tests, or publishing a package completes none of the participant stages.

- [GOV.UK: moderated usability testing](https://www.gov.uk/service-manual/user-research/using-moderated-usability-testing) — realistic task goals without revealing the answer.
- [NN/G: aesthetic-usability effect](https://www.nngroup.com/articles/aesthetic-usability-effect/) — separate visual impressions from observed task difficulty.
- [Apple: onboarding](https://developer.apple.com/design/human-interface-guidelines/onboarding) — sensible defaults and contextual setup.
- [Pennel design contract](../DESIGN.md), [daily-use pilot](improvement-pilot.md), and [desktop roadmap](desktop-roadmap.md) — the product and acceptance boundaries.
