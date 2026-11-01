# Pennel design system

This is the approved desktop redesign contract. Runtime and device acceptance are recorded separately from design approval.

## Visual language

Pennel is a calm writing app with the immediacy of a small native desktop utility. Warm paper and ink, familiar controls and a barely darker sidebar let the user's text lead. Figtree carries controls, navigation and reading on every platform; Instrument Serif carries large titles and the wordmark. The chrome font role follows the shared interface font. The original Swift app at `91775c1` is the visual reference. The active shared tokens and project rules take precedence over older neutral/sans-only descriptions.

One main window hosts Dictation, Meetings, History and Settings. Switching sections reuses that window and preserves the selected meeting, dictation-pad text, History target and relevant reading position. Give Dictation, Meetings, History, Settings and New recording persistent navigation entry points. Settings replaces the library navigation with its categories; its back action returns to the previous work. The dictation pill never activates or takes focus. A recording replaces New recording with a route back to the same active meeting. Closing the main window is distinct from quitting the app; onboarding, the recording prompt and pill keep their dedicated presentation.

Settings opens to task readiness: Dictation, Meeting transcription, and Text processing, and remembers the last selected category when reopened. Models summarises the same tasks; Text processing explains its shared use for dictation cleanup, summaries and questions. Keep connection management and existing deep links reachable through advanced categories. Category navigation is vertical, usable at the minimum window size, and always reveals keyboard focus. While Settings is open, replace the meeting-library sidebar with its category navigation and a visible way back to the previous work. Keep active recording state and its return action visible in that navigation; opening Settings never starts or stops capture. Task setup stays beside its originating task and returns to the same meeting, draft and reading position. Menu, tray, shortcut and legacy Settings links all use the main-window route.

Use spacing to group related controls before adding a container. Prefer unframed reading content, compact source lists, aligned setting rows and progressive detail. Orange identifies selected state, focus or an actionable problem; primary controls use a neutral foreground/background pair. Reading and editing surfaces remain opaque. On macOS, navigation and frontend toolbar surfaces may reveal a successfully attached native material host. Motion conveys an actual state change and respects reduced motion.

## Shared tokens

`assets/brand/tokens.json` is the source of truth. `python3 scripts/generate-theme.py` emits:

- `apps/desktop/src/theme.generated.css` for shared webview content.
- `apps/desktop/src/pill-metrics.generated.ts` for the shared pill geometry.

`assets/ui/icons/manifest.json` owns the compact shared 24 pt icon paths and 1.7 pt rounded strokes. The same generator emits the local SVGs used by the active desktop interface. The retired Mac shell's Swift theme/icon outputs are no longer generated. Mobile branding uses the canonical assets and its retained MobileKit helpers.

Edit the token source, then regenerate. `--check` validates output reproducibility and text contrast; `--self-test` verifies palette and generator failure cases. The generator has no third-party runtime dependency.

Core color roles are Surface, Chrome, Sidebar, Foreground, Muted, Border, Subtle, Hover, Field, Selection, Button, Button text, Accent and Focus ring. Components use these roles rather than adding private hex values. The approved light secondary text is slightly darkened to retain AA contrast on selected-row backgrounds.

Use the shared 4 pt spacing grid, 6 pt control radii, 8 pt segmented-control radii and 12 pt panel/window radii. Captions start at 11 pt, controls at 12–13 pt and reading content at 14 pt. Figtree section headings use the section/title tokens; large Instrument Serif titles use the page/display/onboarding tokens. Do not apply serif type to controls or fake a bold serif weight. Use sentence case and restrained tracking.

Compatibility names remain semantic: Paper maps to Surface, Background to Subtle, Ink to Foreground and Line to Border. These values adapt to appearance. Fixed artwork colors and dictation-pil colors have separate roles.

## Appearance

System / Systeem follows the operating system. Light / Licht and Dark / Donker override it for every open product window, menu, dialog, picker and editor. Apply the preference before the window is shown and update it live without replacing focused inputs.

Both appearances have the same geometry, content and control states. Light uses warm paper with a barely darker sidebar; dark uses the token-defined warm ink surfaces. Readable text contrast is at least 4.5:1, including metadata and placeholders. Color never carries recording, error or selection meaning without an accompanying label, icon or accessible state.

The dictation pill stays Ink with Paper text and a Soft wave audio/processing indicator in both appearances. Do not invert it by reusing ordinary window foreground and background roles. Speaker identity uses the shared four-color set with appearance-specific readable variants; labels also identify the speaker.

## Onboarding

Keep onboarding in its existing dedicated window. Do not draw an in-content title bar or imitate system window controls. A small Pennel label and stable actions provide orientation; keep the action reachable when content must scroll.

Welcome offers Try dictation or Record a meeting. Carry that task through speech setup, relevant access and the first real task. Recommend a supported local setup with a clear one-time download and processing explanation; model names, Apple dictation and user-chosen providers remain available under Other options. The footer owns the next action and truthful download/progress states. Keep interface language independent of speech language and user content.

Show real model metadata, progress and errors. Permission controls describe their task and provide the actual recovery action. Calendar and system audio are introduced when meetings need them. Denied or unknown access must not become an unexplained disabled dead end.

Dictation practice uses the actual configured speech engine and a visible editable result field. The configured shortcut and manual recording action share the same session. Practice never falls back to another app or the clipboard. A real nonempty result is required for successful dictation practice; setting up later preserves a resumable state. The meeting path opens the actual recording workflow rather than treating a simulated dictation as meeting success. Optional voice enrollment belongs in the meeting context and must not extend ordinary first use; older saved voice-step progress remains recoverable.

## Model setup

Models shows the current service, readable model name, processing destination and readiness beside each task's Change or Finish setup action. Overview, Models and task recovery use one chooser. Start with On this computer, Pennel Cloud and My own provider; retain unavailable choices with an explanation. Keep connection setup, permissions and explicit downloads in that flow, showing destinations and download sizes before acting. Search remote models by name or ID, with retry and manual ID entry available when the catalog is unavailable.

Model selection and sample testing leave the active task unchanged until Use this setup succeeds. Cancel or Escape returns to the originating work without applying the draft. Connection/key saves, downloads and sign-in are separate explicit actions and can remain after cancellation; say so in the chooser. Missing local assets never select a different model implicitly. Apply changes one task atomically and keeps other choices intact. Before editing a shared connection, name its affected tasks. A sample test uses only bundled synthetic content, reports the configuration actually tested and loses its success state when that configuration changes, including during Apply. Subscription clients and remote server endpoints belong under My own provider, with their processing destination explained explicitly.

Experimental connections start collapsed under Experimental connections — setup and testing only. Explain that ordinary dictation, meeting processing and search do not use them. Missing connections get an explanatory empty state and Add connection action rather than a blank dropdown. Keep required OpenRouter upstream routing visible; optional identity, input-budget and timeout fields belong in Advanced. Preserve saved settings and English/Dutch parity; this desktop setup refinement does not redesign the iPhone UI.

## Workspace and data entry

Use persistent Dictation, Meetings and History section controls in the main sidebar. Returning to Dictation restores its previous home or pad surface and saved text; History recovery links highlight the requested result in main. An owned pad dictation must finish or be explicitly cancelled before leaving while it is listening or processing. Navigation must explain that requirement and keep the active session intact rather than silently cancel it.

Dictation leads with a clear readiness state and the shortcut in another app. Try it here opens safe owned practice without changing saved first-use progress; Dictation settings is secondary when the task is ready. The persistent editable pad remains separately available. Its optional external-paste setting applies only to pad sessions and shows the captured target app. Text remains in the pad if external delivery is unavailable or cancelled. Global dictation freezes its current external destination when recording stops and never retargets after asynchronous processing. Completion distinguishes inserted, copied, saved and retained-for-recovery results.

The floating pill shows at most two recent lines of real recognition text above a small listening/working indicator. It does not activate Pennel or capture input focus. Provisional text replaces earlier revisions rather than accumulating duplicates; working retains the current caption, and cancel, hide or a new session clears it. Batch providers show their final recognition text after recording rather than implying streaming. Shared pill metrics define the same bounded desktop geometry on all supported platforms.

Meeting notes dominate the detail surface. Notes, Transcript and Questions stay compact and close to the content they affect. Title, metadata and reading content share a clear alignment. Keep elapsed time and Stop stationary as text grows. The same meeting continues from recording through processing, review, source inspection and copy/export. Infrequent template, regeneration, export and file actions belong in consistent local controls or a contextual menu. Search and cited answers preserve selection and timestamp navigation and provide a clear return to the originating answer.

An unavailable summary or regeneration action always has a visible explanation and useful setup action, including when an older summary exists. Task setup uses the existing model/provider capabilities and credential store, preserves the originating document and returns to it after completion or cancellation. Never switch a provider or send a transcript merely to make a setup screen appear successful.

Keep draft text, selection, scroll position and focused controls stable during model progress, theme updates, permission refresh and navigation. Preserve recording status when users visit another section. A generic success message must not occupy every otherwise empty surface; show acknowledgement after a relevant action.

## Components and acceptance

Every control has clear default, hover, keyboard focus, selected, disabled, loading and error states. A disabled primary button keeps its label readable and does not inherit a secondary hover background. Empty states explain what belongs there and provide a useful first action. Unavailable platform features keep the same control and a short explanation in the selected interface language. Keyboard focus, logical headings and important status announcements remain usable at minimum sizes and with enlarged text; motion respects reduced motion.

Standard popup choices and action/context menus use the operating system's native menu through Tauri. Their trigger geometry, labels, ordering, actions and disabled semantics are shared; native menu rendering follows each platform. Product content uses the existing opaque surface tokens. Source-list refreshes preserve stable rows, focus, selection and scroll, and pending navigation acknowledges the destination while waiting for durable draft saves.

## macOS materials and chrome

The existing macOS service library hosts the Tauri webview inside a public `NSGlassEffectView.contentView` on macOS 26 and newer. Glass is exposed through the navigation, titlebar region, toolbar and recording prompt. The toolbar controls and product views remain in TypeScript. A single host provides one material geometry; individual HTML controls do not gain separate native glass shapes or merging effects.

Use regular glass and let AppKit render it. On macOS 14.4–15, use the system sidebar/popover visual effect material. Reduce Transparency and Increase Contrast select opaque navigation/control backings; Increase Contrast also strengthens focus and control boundaries. Reduce Motion suppresses new transitions. Native environment changes update styles in place, retaining draft text, focus and selection. Native attachment failure leaves an opaque, usable interface.

Let AppKit position the real traffic lights and preserve the accessible window title. An empty unified native toolbar supplies the standard window geometry; every action control remains in the shared frontend. Reserve the measured native control extent and keep interactive controls outside the drag region. Use the shared 52 point toolbar band, 36 point circular/capsule surfaces, 18 point glyphs and 12 point action-group gaps. Sidebar visibility belongs at the trailing edge of the sidebar; the context title aligns with the content inset. Ordinary controls have at least 28 point height and source-list rows 32 point height. The shared frontend toolbar offers sidebar visibility, New recording/Return to recording and Find. Sidebar presentation uses the shared 230 point default and 200–320 point limits; collapsing or resizing it preserves the current work. The fixed Ink dictation pill retains its nonactivating, click-through behavior.

Inspect the shared app against the original reference at `91775c1` and retained captures, using deterministic fixtures in English/Dutch and light/dark. Use app windows or offscreen rendering, not the user's screen. Cover the native windows, first-use paths and meaningful error/loading states at normal and small sizes. The legacy SwiftUI shells are retired under [cleanup #75](https://github.com/pennel-app/pennel/issues/75); Git history retains their source. Hardware capture, focus, clipboard, shortcuts, VoiceOver and human comprehension require separate evidence. The [usability and adoption protocol](docs/ux-adoption.md) defines the formative rounds and voluntary cohort; design approval is not a completed study.
