# Shared Pennel desktop app

Device connection availability, recovery boundaries and current verification: [device connection](device-connection.md).

`apps/desktop/` hosts one Tauri 2 application for Mac, Windows and Linux. Its TypeScript/Vite interface links through Rust commands to one `PennelEngine`. The legacy SwiftUI Mac shell is retired under [cleanup #75](https://github.com/pennel-app/pennel/issues/75). The embedded macOS platform library remains the owner of Apple integrations; packaged-device and signed-upgrade acceptance remain separate.

The main window **Pennel** hosts the meeting library and Settings. Its compact toolbar keeps **New recording**, sidebar visibility and Find available. The library sidebar keeps Dictation, Meetings, History and Settings beside search, meeting questions and meetings grouped by date. An active or unfinished recording replaces the new-recording action with a route back to that exact session; it does not start another capture. Settings opens inside the same main window from the tray/menu, in-app links or `Cmd/Ctrl+,`. Its back action returns to the previous meeting or recording, preserving drafts, the selected view and reading position. History and Dictation use the same workspace window. The recording prompt and nonactivating dictation pill remain separate. Platform-specific features stay visible with capability/permission explanations.

Settings starts at **Overview / Overzicht**, showing readiness for **Dictation**, **Meeting transcription**, and **Summaries and questions**, each with **Change** or **Finish setup**, and remembers the selected category when reopened. Its vertical category navigation replaces the library sidebar while Settings is open and groups ordinary work settings, Appearance/Account, and advanced Providers/Models. Active recording state and a return action remain visible there without starting or stopping capture. Existing detailed links remain supported and route into the main window. **Ready to try / Klaar om te proberen** reports configuration and queryable access, not proof that a real provider or microphone test has succeeded.

Task-local setup is a dialog over the initiating view, with connection details kept in that flow. Preferences save as made; API keys have an explicit Save action. Back or Escape returns to the same document without committing a typed, unsaved key. Done saves a typed key through that same explicit action and waits for persistence; a failed save keeps the input and dialog available. Returning does not undo already-saved configuration. Meetings keep personal notes, selected views, source navigation and reading position through processing and setup. Missing summary configuration has a visible **Set up summaries** action even if an earlier summary already exists; questions provide their own setup route.

## Appearance and first use

**System / Systeem**, **Light / Licht** and **Dark / Donker** use the shared semantic tokens in `assets/brand/tokens.json`: warm paper and ink with a barely darker sidebar. `make theme` generates the desktop CSS, pill metrics and shared SVG icons; `make theme-check` checks that the committed outputs match. Figtree carries navigation, controls and reading content on every platform. Instrument Serif carries large titles and the wordmark. The floating pill has separate fixed colors so changing window appearance does not invert it.

Webview zoom shortcuts are disabled. Enlarged-text fixtures exercise responsive layout, but do not establish OS text-size or human accessibility acceptance. On macOS, native Liquid Glass backs navigation and toolbar surfaces on version 26 and newer; older versions use system vibrancy. Reduce Transparency and Increase Contrast select opaque backing, and Reduce Motion suppresses transitions. Reading and editing content stays opaque. See the [macOS material audit, visual references and verification](macos-liquid-glass.md).

Standard choice popups, action menus and meeting-row context menus use Tauri's native system menus. Their triggers retain Pennel styling; menu fonts, animation and accessibility follow the operating system. Labels, available actions and disabled states remain shared. Keyboard activation, arrow navigation and Escape are supported; owned browser fixtures use an equivalent renderer menu without invoking the native bridge. Native menu resources stay owned until selection, replacement or view disposal.

Meeting navigation highlights its destination before waiting for draft persistence. Independent detail/status reads run together; speaker setup, templates and summary annotations fill in after the document appears. Later navigation invalidates older responses, while save-before-leave, hide and quit barriers remain in place. The library retains keyed rows, keyboard focus and reading position; arrows, Home/End and type-to-select navigate its source list.

The visible library refreshes on local mutations, sync changes and focus, with a thirty-second recovery refresh. Settings pauses library queries and retains invalidations for the return to work. Lightweight runtime snapshots update the recording shortcut and clock without reading notes or copying transcripts. A transcript revision requests full content only after it changes; unchanged live rows retain their DOM/text nodes. Filesystem-bearing commands run off the native UI thread and retain durable writes. Repeatable synthetic Mac WebKit measurements and their limits are recorded in [desktop performance](desktop-performance.md).

First use offers **Try dictation** or **Record a meeting**, with an interface-language choice on Welcome. New profiles recommend compatible local speech recognition in **Fast / Snel** mode; **Other options** reveals speech language, engine/model and provider alternatives. Setup carries the chosen task through its relevant access requirements. Dictation practice requires a real nonempty result and then offers **Open dictation**. The meeting path shows **Ready for your first meeting** and **Continue to recording**: it explicitly is not recording yet, and opens the real recording workflow for checking sources and starting capture. That navigation is not evidence of an actual recorded meeting.

Optional voice enrollment remains under Settings → Meetings rather than extending ordinary first use. Older saved Language and Voice steps stay resumable. Model downloads and provider credentials use the existing core and OS credential services. A saved wizard step is not proof that a model or permission remains available. Existing profiles keep their providers, models, dictation mode and unfinished setup. Windows and Linux open the workspace at launch; Mac uses the menu bar lifecycle below.

Microphone access is required for recording. Global shortcuts, external placement, meeting audio and optional speech/calendar features have their own permission requirements; unsupported controls remain visible with an explanation in **Instellingen**. On an Apple keyboard, set **System Settings → Keyboard → Press 🌐 key to → Do Nothing** when using `fn`. Logitech MX keyboards can use **Rechter Option** under **Instellingen → Dicteren → Sneltoets** instead.

The dictation home reports **Ready to dictate**, or **Ready to try in Pennel** when external shortcut/placement access is unavailable. **Try it here** opens owned practice without changing saved onboarding goal/progress; **Dictation settings** is secondary. Practice uses a temporary Fast setting and never uses external paste or clipboard delivery.

The separately available dictation pad retains an editable local draft. **Dicteerpad ook extern plakken** is off by default and applies only to recordings made in that pad. Enabling it captures the previous external target when recording starts; it does not change normal shortcut dictation in other apps. If the captured target cannot be used, the text remains in the pad.

Window closing and navigation invalidate pending owned external delivery. Meeting recording and the normal background runtime retain their existing lifetime. Real device and platform acceptance are recorded separately in [redesign acceptance](redesign-acceptance.md).

### Interface language

The shared desktop app supports **English** and **Nederlands** on macOS, Windows and Linux. First use offers **System language**, **English** and **Nederlands**; the same choice is available in **Settings → Appearance**. System language is the default and shows the language it currently resolves to. Pennel chooses the first supported language in the system preference list, including regional variants, and falls back to English. System preferences are read on launch; an operating-system language change applies when Pennel next starts.

Changing the interface preference saves and relabels open desktop windows, menus and recording overlays immediately. It does not reload a webview, replace active text fields, stop a recording, or change speech recognition, provider/model choices, prompts, template contents, transcripts or notes. Existing profiles keep their onboarding progress, and language remains available when reopening the introduction.

Translations live in the shared `assets/locales/` catalogs. Frontend `l()`/`liveText()` values explicitly mark app-owned text and attributes for in-place updates; raw user content is never matched or scanned for translation. Rust resolves preferences and supplies localized native UI metadata. Application-owned diagnostics and canonical defaults use English; the catalogs retain older Dutch error aliases. Unmodified built-in names follow the interface, while default summary headings follow the meeting language and custom template content stays literal. See the [language policy](language-policy.md). No remote translation service or extra dependency is used. Historical native Mac UI source remains available in Git.

## Mac menu bar lifecycle

The shared Tauri app stays in the Mac menu bar while its product windows are hidden. Opening the workspace or onboarding exposes normal application menus and a Dock/Cmd-Tab entry; closing the last product window returns it to accessory mode. The recording prompt and pill remain nonactivating. First launch or unfinished setup opens onboarding. Its completion action opens the selected dictation or meeting workflow; later cold launches stay in the menu bar.

Click the Pennel icon for the native action menu. **Start opname**, **Stop opname** and **Opname afronden…** reflect the recording runtime; the icon distinguishes an active recording and the dictation hint reports actual shortcut availability. **Open Pennel** opens the main workspace, or resumes unfinished onboarding. Opening the running app through Finder or Spotlight does the same. Dictation, Meetings, History and Settings all use that same main window, including menu/tray actions and legacy links.

Closing a product window saves its drafts before hiding it, without changing the other windows. If saving fails or times out, Pennel keeps that window open with an error. Reopening restores its owned session and refreshes the view; presentation polling pauses while hidden. Global dictation, the pill and meeting recording continue independently of window visibility. **Stop Pennel** uses the existing shutdown path to save drafts and finalize recordings.

## Dictation destinations and live text

### Saving and recovery

The shared app keeps completed dictation text available even when History cannot write to SQLite. **Nog niet bewaard** entries appear first in History, with **Kopiëren**, **Opnieuw bewaren** and explicit discard. Saving again never repeats recognition or pastes into another application. A failed save cancels automatic placement in the original destination; recovery notices refer to the exact dictation rather than whichever result finished last. Clipboard-write failures also stop placement instead of sending a paste command with stale clipboard contents.

Temporary recovery entries live only in the running process. Pennel tries to save them before quitting or updating and stays open when saving still fails. Explicitly discarding an entry works even when SQLite is unavailable. New dictation is blocked at twenty retained/in-flight results or the five-MiB recovery budget; an unexpectedly oversized completed result is kept intact and blocks further capture until resolved. Closing a window does not discard temporary results. A process or computer crash cannot preserve memory-only text.

Meeting audio chunks are durably written before recognition. Recognition queues file references rather than unbounded PCM; the capture ingress is bounded. Transcript replacement is atomic and recovery audio is removed only after its matching transcript is committed. Repeated storage failures stop capture with a persistent warning and retain the remaining material for **Opnieuw bewaren**, which does not rerun successful recognition. Pending WAVs are discovered after a restart. New recovery filenames and optional millisecond transcript timestamps prevent short chunks in the same second from overwriting one another; existing whole-second files remain readable, and ordinary timestamp display/export is unchanged.

Two consecutive failed recognition chunks pause further recognition requests for that recording while audio continues to be saved. The recording screen explains the pause; failed chunks can be transcribed explicitly after recording. Fatal capture/storage errors stop recording, retain notes and title drafts, and prevent a new recording from replacing unsaved material.

Speech provider, model, language, dictionary and credentials are frozen when a meeting starts. The live screen reports the frozen, nonsecret recognition configuration. Later settings edits apply to the next recording; summaries and questions use the model chosen when requested. Cloud account changes are blocked during an active meeting, while normal token refresh for the same account remains available.

Finished meetings expose **Opnieuw transcriberen vanaf opname** / **Transcribe again from recording**, separately from retrying failed chunks. Confirmation shows the selected local meeting model and language. Audio is decoded one track at a time, retaining its original silence and timeline; known local dictation exclusions are masked on both tracks. Recognition settings and dictionary are frozen for the job. This action never uses cloud speech recognition or requests a new summary, and it does not switch the selected model.

The old transcript remains available while recognition runs. A successful replacement keeps a private, versioned copy of the previous transcript, speaker names, summary metadata and any failed-chunk audio under `.pennel-transcript-backups`. A durable transaction journal restores that version after an interrupted save, before the next library open, index or sync. Notes and recordings stay in place. The retained summary is marked outdated. Existing speaker labels are rebuilt with downloaded local speaker models when available; names transfer only through voice matching, otherwise the physical microphone/computer roles remain. Conflicting edits to the same meeting are blocked while progress is shown.

Speaker recognition now records physical transcript sources before relabelling a shared microphone. Older diarized transcripts without source metadata require both saved tracks: a voice labelled “Anderen” alone cannot establish whether it came from the microphone or computer. Missing required recordings, models, or unreadable exclusion metadata prevent replacement. Legacy recordings without saved dictation intervals cannot reconstruct past exclusions.

The timestamp regression and replacement/error recovery paths are covered by automated tests. Real Parakeet versus Whisper Large v3 Turbo quality comparison and recording/retranscription on an actual Mac remain acceptance work; model weights were unavailable in the implementation environment. The existing Linux Mac-AAC compatibility decode failure is tracked separately from the core timestamp fix.

**Dicteren** opens the global workflow. Click a text field in another app, hold the configured shortcut and release to paste there. The main Pennel window does not need to be open. **Open dicteerpad** is an explicit secondary action; its existing local-draft and optional external-paste behavior remain separate from global dictation.

Global dictation freezes an opaque external target when recording stops, before asynchronous transcription and cleanup. Provider, language, mode and dictionary are frozen at recording start; app-specific style context comes from the same stop target as delivery. A changed or unavailable target is never replaced with another recipient. The completed result remains in History with a recovery message. Mac additionally checks the captured Accessibility text-field identity; Windows/X11 currently guard the app, window and process-launch identity rather than exact field identity. Wayland retains its explicit copy-only behavior.

`LiveDictationSession` owns preview IDs, monotonic revisions and cancellation in the shared core. Apple uses the existing selected system provider through `SpeechAnalyzer` with volatile results. Local models use one cancellable decode at a time over a bounded six-second audio tail. Cloud providers remain one final ASR request; their raw result can appear before cleanup, without extra preview requests. Preview cannot write history, run cleanup/commands, read selections or paste. The authoritative final pass uses the same frozen settings. Closing or cancelling an owned pad/practice session also cancels pending finalization, preventing later result/history commits.

The non-activating pill shows up to two recent lines and keeps the last caption while processing. Session, revision and owner guards reject stale text and audio-level updates. Its dimensions and colors come from shared tokens. Apple preview stop closes and drains callbacks, then waits for analyzer cleanup off the UI thread before definitive recognition.

Logitech MX keyboards do not transmit Apple's Fn/Globe keycode. They can use **Rechter Option** in **Instellingen → Dicteren → Sneltoets**; `fn` remains available for Apple keyboards. [Logitech's documentation](https://support.logi.com/hc/en-us/articles/29752695205527-Logitech-keyboard-FN-key-behavior-on-macOS-or-iOS).

The delivery and caption behavior was compared with [FluidVoice's stop-target policy](https://github.com/altic-dev/FluidVoice/blob/0fccdeb014462e6f82889af38c7e9de92d87a130/Sources/Fluid/Services/DictationTargetPolicy.swift) and [live recognition loop](https://github.com/altic-dev/FluidVoice/blob/0fccdeb014462e6f82889af38c7e9de92d87a130/Sources/Fluid/Services/ASRService.swift#L6982-L7278). Provider limitations differ; Pennel does not claim that a batch cloud endpoint streams during recording.

## Architecture and lifetime

| Layer | Responsibility |
| --- | --- |
| `core/` | Providers, processing, gestures/paste ordering, call debounce/rules, recording lifecycle, timestamped ingestion, stores, models, search/chat/templates/diarization/export |
| Tauri Rust host | One engine, application/background lifetime, command/event contracts, recording and dictation workers, native adapters, window/tray management and updates |
| Shared TypeScript views | Render snapshots and send explicit actions; no microphone capture or credential persistence |
| `apps/macos-platform/` | Swift/Core Audio/AVFoundation/Accessibility/EventKit/Apple Speech/Foundation Models behind an owned C ABI |
| Windows adapters | Win32 keyboard/clipboard/window services, WASAPI/audio sessions, Media Foundation AAC and endpoint-dependent echo processing |
| Linux adapters | X11 hooks/context, PulseAudio/PipeWire metadata and monitor capture, fixed GStreamer AAC pipelines, Wayland clipboard delivery |

The runtime survives main-window closing/reopening. Dictation, the meeting library and live-recording routes, History and Settings share one main window; onboarding, the recording prompt and pill retain their own presentation. Navigation preserves saved pad text, meeting notes and section context, and exact History recovery links select the intended entry inside main. Audio passes directly from native capture into Rust as 16 kHz mono Float32, with one recording timeline and interruption gaps. JavaScript receives state, levels, progress and targeted owned-session results. The Swift library has no SwiftUI or embedded Rust core; [its bridge documentation](../apps/macos-platform/README.md) describes ownership and main-queue requirements.

The main sidebar switches between Dictation, Meetings and History without opening another product window. Dictation remembers the home or pad surface, and Settings returns to the originating section. Pad edits are saved before a section switch. While an owned pad dictation is listening or processing, finish or explicitly cancel it before leaving; an attempted switch explains the requirement and leaves the session intact. This does not change global shortcut dictation or its nonactivating pill.

Recording uses `idle → starting → recording → stopping → finalizing → idle/error`. The runtime rejects concurrent starts and meeting-folder changes while active, and session identifiers protect newer meetings from older completions. Automatic call detection samples activity every two seconds, requires a sustained five-second candidate, and ends after thirty seconds without microphone activity. It stops only recordings associated with that detected call. The recording prompt expires after sixty seconds.

## Platform behavior

| Capability | macOS | Windows | Linux X11 | Linux Wayland |
| --- | --- | --- | --- | --- |
| Dictation trigger | Fn/right modifiers, native event tap | Right Ctrl/Alt or Ctrl+Super hook | Same gestures through XRecord | Configured compositor shortcut invokes manual toggle |
| Record shortcut | ⌘⇧R native tap | Ctrl+Shift+R registration | Ctrl+Shift+R registration | Configured shortcut invokes meeting toggle |
| Selection/insertion | Accessibility, guarded copy fallback, ⌘V | Guarded copy, foreground PID checks, SendInput | Guarded copy/paste, X11 foreground context | Copy result and paste manually; selection reading unavailable |
| Call observations | Core Audio process activity + browser titles | Audio sessions/process trees + browser titles | PulseAudio/PipeWire metadata + X11 browser titles | Native call-app metadata; browser classification unavailable |
| Meeting audio | AVAudioEngine + Core Audio process tap | Microphone + WASAPI loopback | Microphone + PulseAudio/PipeWire monitor | Same audio-server capture, subject to device access |
| Echo processing | Apple voice processing and existing fallbacks | Available endpoint effects; runtime probe | Unavailable; use headphones | Unavailable; use headphones |
| Calendar / Apple Speech | EventKit / macOS 26+ speech | Visible but unavailable | Visible but unavailable | Visible but unavailable |
| Apple Intelligence text model | On-device Foundation Models on eligible macOS 26+ Macs; live availability check | Visible but unavailable | Visible but unavailable | Visible but unavailable |
| M4A archive/playback/diarization | AVFoundation | Media Foundation decode/encode, native output | GStreamer AAC plus native output | Same codec support, subject to installed helpers |
| Local speech models (Whisper, Parakeet, …) | Metal | CPU, x86-64-v3 (AVX2); older processors see the models disabled with a reason | Same as Windows | Same as Windows |
| Pennel Cloud (login, meeting sync, hosted models) | Shared app: Account tab | Shared app: Account tab | Shared app: Account tab | Shared app: Account tab |

This table describes implemented paths, not completed hardware certification. Call classification is heuristic. Safari/WebKit attribution, app helper variants, device routes and real meetings remain acceptance cases.

The [Expo iPhone app](mobile.md) uses the same Rust core and exposes the corresponding settings capabilities. These are visible but switched off there, with the reason shown in the app:

| Capability on iPhone | Reason shown |
| --- | --- |
| CLI agents (Claude Code, Codex, …) | `Werkt alleen op een computer.` |
| Local models | `Lokale modellen komen later naar iPhone. Apple-dicteren werkt al zonder internet.` |
| Echo cancellation | `Niet nodig op iPhone: Pennel neemt alleen de microfoon op.` |
| Call detection, calendar, per-app and per-meeting recording rules | `Op iPhone neemt Pennel alleen op wanneer jij op Start opname tikt.` |
| Style per app | `iOS laat Pennel niet zien in welke app je typt. Op iPhone geldt de standaardstijl.` |
| Hotkey (dictation and record shortcut) | `Op iPhone start je met het Pennel-toetsenbord of via Snel starten: Actieknop, Tik achterop of Bedieningspaneel.` |
| Speaker recognition and naming (physical meetings are one microphone track, written as "Anderen") | `Sprekers herkennen is niet beschikbaar. Op iPhone neemt Pennel één microfoonspoor op; iedereen staat onder ‘Anderen’.` |

### Built-in Apple models

In **Providers → Provider toevoegen**, add **Apple-dicteren** for dictation/meeting transcription and **Apple Intelligence** for cleanup, commands, summaries and meeting questions. Neither account has an API key or server URL. The provider actions assign the relevant model slots; the **Modellen** tab shows macOS-managed models without an editable model name. Speech permission is requested explicitly from the Apple-dicteren card. macOS manages speech-language assets and may install them on first use.

Apple Intelligence uses `FoundationModels.SystemLanguageModel.default` on the device, with a fresh session and no tools. Device eligibility, Apple Intelligence being disabled and models still preparing use English diagnostic sources and localized presentation; **Check availability** rechecks that status. Pennel does not switch to a cloud provider when the model is unavailable. Windows and Linux keep the Apple choices visible but disabled with their macOS requirement. Foundation Models is weak-linked so the app retains its macOS 14.4 minimum; the model itself requires macOS 26 or newer.

The shared core applies a smaller prompt budget for Apple Intelligence. Long transcripts use repeated map/reduce, while meeting questions select relevant sources and preserve the numbering of the sources actually supplied. Oversized notes, templates or dictation receive an explicit length error instead of losing text. Native generation checks the exact token count on macOS 26.4 and newer, reserves response space and applies the request timeout. Existing cleanup recovery retains the raw dictation when processing fails.

### Model setup and verification

Models and recording surfaces distinguish the destination for audio from optional text processing. **Ingesteld** means the selected configuration is present; it does not claim that a provider request succeeded. **Test spraakherkenning** and **Test tekstverwerking** are explicit actions using bundled synthetic audio or fixed sample text, never a user's dictation, meeting, dictionary or pad. The destination is shown before the request. A result is marked **Test geslaagd** only for the configuration tested, and is invalidated after model, endpoint, language or credential changes. Catalog connection testing remains separate and failed catalog requests can be retried.

Model downloads have one owner per model. Onboarding and Settings can attach to the same worker and replay its latest progress. Attaching is atomic and never starts a new download when the old snapshot is stale; only an explicit download action can do that. Cancelling keeps partial bytes, and changing the selected speech engine does not implicitly cancel another window's download.

Meeting answers can open the exact transcript, summary or personal-note passage and return to the original conversation position. Changed or ambiguous passages are reported instead of selecting unrelated text. Dialogs, detail tabs and context menus retain names, keyboard operation and focus restoration. These shared-renderer behaviors do not establish screen-reader or platform-device acceptance; use the [pilot checklist](improvement-pilot.md).

### Optional decision and passage-ranking connections

Cleaned-up dictation uses conservative ordered-content safeguards before accepting generated text. Changed values, negation, question intent, language or unsupported wording retain the original dictation through the existing History and delivery workflow. Dictionary spelling permits clear case/accent/spacing correspondence rather than guessed names. See [cleanup fidelity and offline evidence](cleanup-fidelity.md) for the supported normalization limits.

Dictation settings expose **Verify cleanup meaning**, independently from the optional Decision processing connection. It starts off and remains unavailable until the exact pipeline and content-language policy are qualified. No production model is qualified in this version; a connection sample does not enable verification. The settings explain saved enablement and the configured destination, while active sessions retain their original destination. Requested but unavailable verification keeps raw text, with no retry or alternate provider.

Advanced Models settings expose **Decision processing** and **Passage ranking**. Both
start disabled, including on existing profiles. They reuse explicitly selected OpenRouter
or Custom server accounts and their OS-stored keys. Custom accounts can have an optional
key; a keyless local service remains supported. Removing an account disables its optional
connections without selecting a replacement.

Decision processing supports OpenRouter Decisions/System One, direct TypeSafe,
Perplexity/Mercury and the official loopback Decider endpoint. Passage ranking uses TEI.
The API format determines the request path; the existing account URL remains authoritative.
Remote connections require HTTPS; HTTP is accepted only for a literal loopback address or
localhost. On iPhone, loopback refers to the phone itself, not a Mac on the same network.

Save the model and, for OpenRouter, an exact upstream route. Advanced fields can pin an
expected returned model, set a conservative serialized-input byte budget and bound the
timeout. These byte limits are not a tokenizer estimate. Oversized inputs fail before
transmission instead of being truncated. OpenRouter requests deny data collection, require
provider ZDR and disable fallbacks; these settings do not establish regional residency or
account logging policies.

**Test with sample** explicitly sends synthetic text only. Its result records the returned
model/provider and confirms that the connection processed that sample; it does not
establish accuracy or approve a model for automatic processing. Edits, account changes and
cancellation invalidate late verification results. Ordinary dictation, summaries and
meeting answers retain their existing providers and behavior.

The typed Rust/native interface preserves raw distributions and confidence metadata,
unknown usage/cost and model/provider identity. Confidence is not calibrated correctness.
TEI scores remain relevance scores. Extraction uses validated Unicode-scalar offsets with
an exclusive end. An offered choice ID exactly equal to `abstain` denotes explicit
abstention; `unknown` remains an ordinary domain label. No new model runtime, download,
automatic feature or hosted Pennel Cloud decision service is included.

### Pennel Cloud

The shared app has an **Account** tab (last in Settings) with login, usage, meeting sync, the recovery code, devices and account deletion, plus a **Pennel Cloud** provider card. The retired Mac shell's disabled Account/provider surfaces are historical migration context.

The shared Account tab has controls beyond the original plan: **Nu synchroniseren**, **Herstelcode maken**, **Verwijderingen bevestigen** (the mass-delete brake: a sync that would remove many files waits for confirmation) and a notice while a shown recovery code has not been confirmed.

The device-connection candidate in [app #77](https://github.com/pennel-app/pennel/issues/77) puts guided library setup before account administration. It distinguishes a new library, an existing locked library, recovery backup and a validated paused key. The same desktop controls expose QR display, manual code entry and trusted-device approval on macOS, Windows and Linux. The iPhone scans the QR; both sides can use the manual code. Device approval transfers the existing encrypted-library access and signs in the new device without rotating the recovery code or resetting meetings. Production admission remains gated on protocol/deployment/device acceptance. Installed Windows/Linux behavior is not established by local Mac checks.

**Claude-toegang** (Account tab, below the recovery code): a switch, off by default on every computer, that lets Claude search and read this computer's meetings through Pennel Cloud's MCP server. While signed in and switched on, `ClaudeAccessRunner` (`src-tauri/src/claude_access.rs`) keeps a long poll open and answers each relayed tool call with the core's read-only `claude_access_answer`; signing out or deleting the account switches it off. The section shows the connection status and the connector URL and `claude mcp add` command to copy. Same on every desktop platform. Protocol and privacy rules: [claude-access.md](claude-access.md).

What is synced: text files in the meeting folders (Markdown notes, transcripts) and `.templates`. Audio is not synced. Content is end-to-end encrypted ("end-to-end versleuteld") on the device before upload, with a recovery code to restore the vault on another device. The server can see the number of objects, their sizes, times and which device uploads.

The recovery code is shown, never written to disk or logs. Copying it excludes it from clipboard history on Windows and on X11 (ownership is kept so pasting keeps working). On macOS it is marked transient/concealed so clipboard managers that honour these types skip it; macOS's own clipboard history behaviour is unverified. On Wayland it uses `wl-copy` with the history-exclusion option; older wl-clipboard versions fall back to plain `wl-copy`, which clipboard managers may record.

### Hotkeys and clipboard

Hold to talk, double-tap for hands-free, press again to submit, and use Shift for commands. Modifier use interrupts hold-to-talk. A command's foreground PID is checked before selection and before delivery; the paste queue preserves recording order.

Mac snapshot/restore preserves pasteboard representations. Windows preserves native clipboard formats, excludes dictated text from clipboard history/cloud clipboard, and detects concurrent user copies. Elevated Windows applications can block synthetic input through UIPI. Linux X11 restores text or one image; rich export does not imply preservation of every prior clipboard format. Paste delivery errors are reported rather than silently treated as success.

The pill and recording prompt use platform no-activation behavior; the pill is click-through. Mac uses an NSPanel without replacing the Tauri window's class. Full-screen, Spaces, mixed-DPI/multi-monitor placement and real foreground focus require packaged desktop tests.

### Linux X11 and Wayland

X11 uses XRecord for dictation gestures, XTest for synthetic keys and `xprop` for active-window/application context. It requires an accessible X11 session. Wayland deliberately disables those paths even when XWayland is also present.

Configure desktop shortcuts for these commands:

```sh
pennel --toggle-dictation
pennel --toggle-meeting
```

For dictation, press the shortcut once to start and again to finish. Pennel uses `wl-copy` to place the result on the Wayland clipboard; paste with Ctrl+V. Selection reading, synthetic paste and foreground-app styles are unavailable there. The meeting command starts/stops a manual recording. When using AppImage, use the AppImage's full executable path in the desktop shortcut.

Native call applications can still be observed through PulseAudio/PipeWire on Wayland. Browser classification requires X11 window titles, so use manual browser-meeting recording on Wayland. Missing `pactl`, an unreachable audio server or missing codecs produce a reason in capability state. Codec availability is cached for the launch; restart after installing missing packages.

Linux M4A services use fixed GStreamer pipelines and bounded subprocesses; audio is passed through file descriptors rather than command-line text. [Linux packaging](linux-packaging.md) lists build/runtime dependencies and explains external AppImage helpers. The supported configuration uses Base/Good/libav and does not require the Bad plug-in set.

### Audio, errors and privacy

New meetings retain `audio/microphone.m4a`, `audio/system.m4a`, local Markdown and private recording diagnostics. Legacy `ik.m4a` and `anderen.m4a` files remain readable without migration. See [recording layout and provenance](recording-format.md). System-capture failure can leave microphone-only recording with a visible reason. If Mac microphone startup fails with voice processing enabled, capture retries once with a fresh engine without echo cancellation and shows the headphone warning. Permission and missing-input errors retain their specific recovery messages. Mac preserves device-change recovery and silent-capture fallbacks, keeps monitoring replacement engines and adapts conversion when device formats change. A microphone/device failure, archive overflow/write failure or missing codec must not be interpreted as a successful complete recording.

Dictation during a meeting suppresses the microphone's transcript input and marks the interval. This is **not raw-audio redaction**: archives can still contain the dictated audio. Diarization decodes are bounded to the existing three-hour limit.

### Speaker recognition

**Sprekers herkennen** runs after a meeting (opt-in) or from the meeting's transcript. It uses the two speaker models (34 MB, one download in Settings → Meetings) or, with **Via Pennel Cloud**, the signed-in account, which processes audio in memory and does not store it. Older saved voice-enrollment setup remains resumable. The desktop decodes `anderen.m4a` first and `ik.m4a` only when the core will split the microphone track (no usable system track and a trained voice), so an hour-long call never holds both tracks in memory. An older Sortformer download still splits voices, but cannot recognise them across meetings.

**Leer je stem** records the microphone for at most 60 seconds through the same capture path as dictation (Swift capture on Mac, CPAL elsewhere). Samples stay in memory and only the voiceprint is stored; it cannot start while a meeting records or a dictation runs. Every platform offers the same controls. A build without local recognition shows **Op dit toestel** disabled with its reason and offers Pennel Cloud.

Keys remain in Keychain, Windows Credential Manager or Secret Service. No API key is returned in an application snapshot. Logs contain timings, lengths and errors rather than dictated text, meeting content or browser titles. Chosen remote providers receive the audio/text needed for the selected task; local mode does not add a cloud account or telemetry.

## Data, preview and upgrades

### Re-transcribe saved meeting audio

In a finished meeting's **Transcript** tab, **Retranscribe recording** / **Opnieuw transcriberen vanaf opname** rebuilds the complete transcript from the saved microphone and computer-audio tracks. This is separate from **Retry**, which only retries failed chunks. Choose and download a local speech model under Models → Meetings first; the confirmation shows the model and speech language. The model, language and dictionary are frozen when processing starts, and audio is never sent to a transcription service by this action.

Tracks are decoded sequentially at 16 kHz, preserving leading silence and their shared timeline. Missing required tracks, decoding errors, model errors and persistence errors leave the previous transcript available. Processing runs outside the webview and exposes per-track progress; competing edits to the same meeting are refused. Notes and audio are retained. Previous transcript, meeting metadata and remaining recovery chunks are kept in a private versioned `.pennel-transcript-backups` folder inside the meeting; this folder is not indexed or synced.

After a successful replacement the existing summary is marked outdated; regenerating it remains a separate explicit action. Previously enabled speaker labels are rebuilt using available local speaker models, with names transferred only through voice matching. If those models are unavailable, the transcript keeps the microphone/computer track labels and offers a speaker-model explanation. Voiceprint caches are bound to the transcript version so old voice numbers cannot silently refer to a different voice after replacement.

Known dictation exclusion intervals are retained locally and silenced before reprocessing. Older recordings without that metadata still contain their original captured audio; historical exclusion intervals cannot be reconstructed. Continuous timestamped audio treats rounding differences below one millisecond as contiguous samples, preserving the normal 15–30-second recognition windows instead of flushing tiny fragments.

| Build | Data directory | Credentials |
| --- | --- | --- |
| Canonical Mac | `~/Library/Application Support/Pennel` | `dev.pennel.Pennel` Keychain service |
| Canonical Windows | `%LOCALAPPDATA%\Pennel` | Existing `app.pennel.desktop` service |
| Canonical Linux | `$XDG_DATA_HOME/Pennel`, normally `~/.local/share/Pennel` | Existing `app.pennel.desktop` Secret Service entries |
| Preview/debug | Corresponding `Pennel Preview` directory | Separate preview service |
| iPhone | `<app container>/Library/Application Support/Pennel`; meetings in `Documents/Meetings` | `app.pennel.ios` Keychain service, this device only |

Canonical meetings use `~/Pennel` or the configured library. Previews pin their library to `<preview data>/Meetings`, reject external paths/symlink escapes, and do not migrate native credentials. `tauri.preview.conf.json` changes the bundle identity to `dev.pennel.Pennel.preview`; debug builds are also isolated.

For permission-bearing Mac previews, the shared bundler reuses an existing certificate through `scripts/preview-signing-identity.py`. Explicit `APPLE_SIGNING_IDENTITY` or `PENNEL_SIGN_IDENTITY` takes precedence; otherwise the selector retains a still-valid certificate SHA in ignored `build/preview-signing-identity.sha1`, then tries the known local certificates or an existing Apple Development identity. With no suitable certificate it reports the ad-hoc fallback. It never creates/imports a certificate or changes certificate trust. Existing signing identities are retained.

Updating an allowed preview with ad-hoc signing changes its designated requirement and can invalidate microphone, speech and Accessibility approvals. macOS can show an old Accessibility switch as enabled while rejecting the new signature; the obsolete entry then needs to be replaced before granting the current app access. Preserve the preview's signing identity on subsequent builds. Debug/release bundles that share the preview identifier must not both remain registered with Launch Services when selecting the app in privacy settings.

Mac shortcut readiness requires a healthy event tap, not just a non-null tap handle or saved permission. The Mac monitor verifies its own tap identity, full requested event mask, port validity and enabled state, then recover registration after permission changes or listener failure. A partially authorized tap is discarded. Local previews accept `PENNEL_SIGN_IDENTITY` to reuse a stable existing signing certificate.

Core startup obtains an exclusive data lease, validates the migration journal and takes a consistent settings/SQLite/failed-audio snapshot before normalization or retention. Existing data snapshots live under `migration-backup-v1`; `migration.json` records startup migration. Library leases and active-session guards prevent conflicting writers and folder moves. These files support recovery and are not a substitute for user backups.

Canonical Mac startup preserves the native bundle identity and Keychain service, rejects a concurrently running older Pennel/Musli app, and handles legacy Musli paths/keys without deleting source data. Failed credential migration is reported for recovery. Permission preservation still depends on actual signing identity and OS behavior; test upgrades using signed packages.

Canonical Mac logs remain `~/Library/Logs/Pennel/pennel.log`; other builds keep `logs/pennel.log` inside their data directory. **Logboek openen** resolves the current build's file.

## Build and test

Use Rust stable, Node 22+, Python 3 and CMake. Mac requires Xcode with the macOS 26 SDK or newer. Windows uses the MSVC toolchain and Windows SDK. Linux needs WebKitGTK/tray, X11/audio development libraries and the runtime helpers listed in [Linux packaging](linux-packaging.md).

```sh
make desktop-preview
make desktop-test
(cd apps/desktop && npm test)
```

`make desktop-preview` installs locked frontend dependencies and builds isolated preview packages. On Mac it builds the app-only Tauri package and creates `build/Pennel Preview.dmg` with `hdiutil`, so preview packaging also works without an interactive Finder session. `make desktop-test` builds the frontend and runs backend tests; run `npm test` as well for frontend regression coverage. `make desktop` builds canonical packages. Installer output is under `apps/desktop/src-tauri/target[/<target>]/release/bundle/` unless a debug build was requested.

Mac canonical packages preserve the existing bundle identity, data directory and Keychain service, including upgrade handling for older installations. Use `make desktop-preview` for isolated data and credentials; do not run an older canonical app against the same data. `make theme-check` verifies the generated shared desktop tokens and icons. See [redesign acceptance](redesign-acceptance.md) for current capture commands and historical comparison evidence.

For hot reload:

```sh
cd apps/desktop
npm ci
../../scripts/with-toolchain.sh npx tauri dev
```

The toolchain wrapper uses system tools or an existing ignored `build/toolchain/` installation; it installs nothing. Mac local models use Metal. Windows/Linux use CPU by default; an explicitly selected Vulkan build uses `npx tauri build -- --features vulkan` and requires the Vulkan SDK.

Distributable Windows/Linux x64 builds in CI and the release helper compile the local speech engine (transcribe.cpp/ggml) for a fixed **x86-64-v3** baseline: SSE4.2, AVX, AVX2, FMA, F16C and BMI2 on; host tuning (`GGML_NATIVE`), AVX-VNNI, AVX-512 and AMX off. `TRANSCRIBE_X86_CONSERVATIVE=ON` keeps every other ggml SIMD flag off. The flags are appended after existing CMake arguments, only the native dependency's release-target cache is cleared, and the generated CMake caches must show exactly this baseline. Earlier releases only set the conservative option, which turned every SIMD tier off, so local Whisper and Parakeet ran scalar code. A one-off CI benchmark on AVX2 runners, with 10 s of audio and the median of three runs, measured these times. Windows: Whisper small 45.7 s → 4.0 s and Parakeet 0.6B 17.9 s → 1.6 s. Linux: 24.9 s → 4.1 s and 10.5 s → 1.6 s. See [portable candidate configuration](linux-packaging.md#candidate-and-update-artifacts). Direct development builds tune for the build machine and are not certified portable across CPUs.

Every Intel Core processor since Haswell (2013) and every AMD processor since Zen (2017) meets the baseline, which covers the Core and Ryzen processors that Windows 11 supports. Some Pentium Gold, Celeron and Atom processors on that list lack AVX. Before a local speech model or the older Sortformer speaker model loads, the core checks the processor (`core/src/cpu_support.rs`). On a processor without these features, the **Lokaal** provider and local model downloads are shown disabled with "Deze processor is te oud voor lokale modellen. Kies een cloudprovider.", and a load attempt returns that reason instead of crashing on an illegal instruction. Speaker recognition uses the prebuilt ONNX Runtime, which selects its instruction set at run time, so it stays available. Runtime CPU dispatch for ggml (`dynamic-backends` in transcribe-cpp-sys) would also cover those processors, but it needs a shared library plus per-ISA modules in every installer. It is not used yet.

Verify shared Rust and affected native integrations:

```sh
make verify-core
swift test --package-path apps/macos-platform
make packaging-test
```

For everyday core work, `make test` runs the ordinary Rust test profile without an unconditional release-library build. `make core` explicitly builds the release library. `make verify-core` runs that release build followed by the tests and stops if either step fails; use it when validating the release core or affected FFI/packaging integration. The [lightweight Codex environment](../.codex/README.md) leaves dependency installation and native builds explicit.

Mobile native preparation generates its own matched lean Rust framework and bindings directly in ignored mobile artifacts. The embedded Mac platform library has no SwiftUI interface or second Rust engine.

### Local model catalog

Local recognition uses [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp). The bundled [model catalog](https://github.com/handy-computer/models) supplies download sizes, supported languages and benchmark data. Update it with `scripts/update-catalog.sh <version>`.

### Repository layout

- `core/` — shared product logic, runtime state machines and stores; UniFFI serves the mobile native runtime.
- `apps/desktop/` — shared Tauri interface, Rust host and Windows/Linux adapters.
- `apps/macos-platform/` — embedded Swift services for Apple APIs.
- `apps/mobile/` — Expo interface, native iPhone services/extensions and generated-app plugins; see [mobile ownership](mobile.md#native-ownership).
- `assets/brand/` — shared design guide, icons and fonts.
- `scripts/` — build, capture, inspection, release preparation and acceptance tools.

## Verification and release state

Automated checks and local package inspection are recorded in [shared-app-migration.md](shared-app-migration.md). Linux's isolated codec round-trip and synthetic PulseAudio observations verify code paths; they are not real calls, installed-package tests or Wayland compositor acceptance. Cross-target Windows type checking does not establish Windows execution. CI configuration does not imply a workflow has run for this change.

The standard CI matrix builds Mac, Windows and Linux artifacts without requiring release keys. App changes on `main` publish [GitHub releases](https://github.com/pennel-app/pennel/releases) containing Mac `.dmg`, Windows `.exe`/`.msi` and Linux `.deb`/`.AppImage` packages. Mac releases use the configured stable self-signed certificate when available; they are not Developer ID notarized releases, and Windows installers are unsigned. The Homebrew cask handles Mac download quarantine. After installation, `brew upgrade --cask pennel` gets later cask updates.

The manual candidate workflow uses existing signing keys/certificates and performs no publication. Release helpers retain the legacy Sparkle upgrade path and require acceptance of the exact Mac, Windows and Linux updater artifacts before preparing an update manifest. The Tauri updater is enabled only when a public key is configured; previews without release configuration do not auto-update. Linux AppImages support in-app updates; Debian installations use a verified package or package manager. See [releasing](releasing.md) and [candidate and update artifacts](linux-packaging.md#candidate-and-update-artifacts).

Installed-size, download-size, memory and latency claims require measurements of the actual release packages. Track remaining desktop acceptance and distribution tasks in the [desktop roadmap](desktop-roadmap.md); the [historical open-items list](open-items.md) remains context rather than a current bug list.

The [usability and adoption study kit](ux-adoption.md) prepares two formative rounds of five nontechnical professionals, blank observation sheets, EN/NL invitation drafts, and a voluntary 15–20-person/four-week cohort. These are unperformed human validation steps. English/Dutch light/dark fixtures, minimum-size checks and automated regressions establish only the behavior they exercise; VoiceOver, actual recording/insertion, participant comprehension and retained use require their own evidence.

Pennel Cloud is built in the shared app and awaits device acceptance (see open-items T11-T13); the Linux clipboard paths for the recovery code have not been exercised on a Linux device.

Retiring the old source shell does not establish real Mac/Windows/X11/Wayland desktop, migration, signed-update or performance acceptance. Run those gates against the actual distributed candidates. The prepared legacy Sparkle archive/appcast provides the native-to-shared transition; the shared app uses the Tauri updater. Public release availability and distribution claims must follow those results.

The look retains Pennel's warm reading surfaces, Instrument Serif titles and real app icons. macOS chrome retains the shared Figtree typography, a compact integrated command row and accent source-list icons. Choices use restrained segmented controls, switches and native popup menus. `scripts/capture-desktop-windows.sh` captures the dedicated capture build's own windows over an owned synthetic backdrop, using isolated fixture data; `scripts/capture-desktop-reference.mjs` renders frontend fixtures headless. The [macOS audit](macos-liquid-glass.md) records Apple Notes/Finder references, layer ownership and native-material limits.
