# Desktop UX owner review

This kit prepares the owner's desktop review of the [UX decision map](https://github.com/pennel-app/pennel/issues/142). It uses Pennel's actual frontend with invented data and a simulated backend. It records no human results automatically. First use, capture, insertion, native accessibility and installed-platform acceptance require the separate Preview checks below.

## Start a browser review

Install the locked desktop dependencies once with `npm ci` in `apps/desktop`, then run from the repository root:

```sh
node scripts/review-desktop-ux.mjs --flow navigation --locale en --appearance light
```

Use `--flow meeting` or `--flow setup` in later sessions. English/Dutch (`en`/`nl`) and light/dark are supported; `--port 4310` requests a fixed loopback port. The default uses a fresh port. Leave the command running while reviewing, and stop it with Ctrl+C. It prints task goals, scenario links, per-scenario reset links and the source commit with an explicit uncommitted-changes marker when applicable. Its ignored `build/desktop-ux-review/<session>/manifest.json` contains preparation metadata, never observations or task results.

Each launch and scenario has its own synthetic profile, including app presentation, saved choices, drafts, conversations and retained results. Reloading the same scenario retains supported work. Its reset link removes only that scenario's profile and returns to the initial invented state; other scenarios and ordinary browser storage stay intact. Capture and inference are simulated, including the sample text. No microphone, real provider, account or receiving app is used by browser fixtures.

Reset retains one content-free generation marker so older tabs cannot recreate cleared work. Reload an older tab before using it again. Unsaved History recovery remains process-local, matching the product; completed saved results survive a synthetic profile reload.

## One decision per session

Present one task goal at a time, without naming the route or control that solves it. Ask what the owner expects when an observed hesitation needs explanation. Record assistance explicitly. Owner familiarity is not evidence about first-time nontechnical users.

| Session | Starting scenarios and goals | Decision |
| --- | --- | --- |
| Navigation | Everyday work: prepare a draft, find earlier dictation and the client conversation, then continue the draft. Active meeting: navigate elsewhere and return. Retained dictation: preserve the unsaved result and recover. Include sidebar search, keyboard access and legacy History links. | [Does the two-space navigation make both jobs obvious?](https://github.com/pennel-app/pennel/issues/144) |
| Meeting document | Capture two successive conversations, retain a personal note, stop and reopen the first meeting. Review/edit its summary, inspect a source, return to the document and prepare an exact shareable copy. Repeat with missing text setup, save failure and delayed completion. | [Does one notes-first workspace cover capture through review?](https://github.com/pennel-app/pennel/issues/145) |
| Task setup | Start a new profile, configure the intended task and explain audio/text processing destinations. Inspect Other options, cancel a draft change, check shared-setting effects and choose summary automation. Use the existing-profile and failed-download scenarios as separate starting states. | [Does task-based setup explain readiness and processing destinations?](https://github.com/pennel-app/pennel/issues/146) |

Reuse the blank [session sheet](usability/session-sheet.csv). Set `round` to `owner-review`, use an owner-only session code, and record each actual attempt as U (unassisted), A (assisted), F (failed) or NA with a reason. Record browser evidence as `synthetic-browser-owner`; record native observations separately. Keep completed worksheets in private owner-controlled storage outside Git. The [participant protocol](ux-adoption.md) remains a separate, uncompleted study; do not enter an owner review into its five-person denominators or adoption results.

Claim a decision only when its live review begins. After the owner's answer, post a sanitized resolution and evidence pointer and index it on the map. Resolve at most one decision per session. Reproducible defects belong in implementation issues with the task, configuration, expected/observed behavior and a regression. Repair blockers and repeat affected tasks before recording acceptance. Preparation, screenshots and automated checks do not supply the owner's side of the review.

## Facilitator controls

These controls are available only in the owned browser fixture, outside the product's normal flows. They inject or release deterministic conditions; they do not demonstrate real audio, storage failure or provider behavior. Use a separate starting scenario or arrange a fault before presenting a task. Do not require the owner to discover developer-console controls.

```js
const fixture = window.__PENNEL_FIXTURE__;
fixture.review.failSaves(true);       // Later saves fail in the synthetic store.
fixture.review.failSaves(false);      // Repair, then let the owner choose Retry.
fixture.review.failDownloads("Synthetic download unavailable.");
fixture.review.failDownloads(null);   // Repair, then retry through the UI.
fixture.review.dictationDelay(10000); // Retain a working result for a navigation attempt.
fixture.review.completeDictation();  // Release pending synthetic dictation completion.
fixture.workspace.notesDelay(10000); // Delay note persistence before navigating.
fixture.workspace.releaseNotes();
```

`fixture.review.resetProfile()` clears the current profile's stored data; use the printed reset link to reload the initial state afterward. Automated interaction cases are separate from the human starting scenarios and must never run the owner's tasks in advance.

## Follow with Pennel Preview

Build the resulting exact candidate with `make desktop-preview`, through the required worktree storage guard. On Mac, pin the verified personal Apple signing identity and team before signing. Retain existing Preview data and credentials; never clear a profile or replace a configured identity as incidental preparation. Use invented material, owned meeting folders and unsent receiving-app drafts. Record the candidate commit, package SHA-256, OS/hardware, devices and selected speech/text routes.

Repeat dictation into an unsent draft, safe practice, meeting capture/Stop, retained-result recovery, shortcuts, clipboard behavior, native menus and close/reopen continuity. Check keyboard-only use, VoiceOver and 200% content enlargement in the actual Preview window. Capture only owned app windows if images are needed. Record synthetic failure injection separately from real behavior; never damage a real database or fill a disk to manufacture a failure.

The pinned main revision used for this kit has native WebView zoom shortcuts disabled and no custom keyboard-zoom handler. Treat an unavailable 200% native enlargement check as an open accessibility gap under [Mac user pilot and accessibility](https://github.com/pennel-app/pennel/issues/13); browser enlarged-text checks do not close it. Verify the owning implementation again when the candidate changes.

Assess [Does the proposed experience meet the release criteria?](https://github.com/pennel-app/pennel/issues/147) against that exact candidate. Keep unperformed native/device checks, participant studies, Windows/Linux installation acceptance and release gates open with their existing owners. iPhone adaptation and adoption measurement are outside this owner-review implementation. The map stays open wherever its destination or acceptance still lacks evidence.
