# iPhone keyboard: native look and one-tap session start

Approved by the user on 13 November 2026. Tracking: [keyboard experience #39](https://github.com/pennel-app/pennel/issues/39).

## Intent

The Pennel keyboard should look and feel like the native iOS keyboard (system colors, SF type, key shapes and spacing) with a modern Pennel dictation button in the top-right corner. Dictation should work like Wispr Flow: when Pennel's background session is ready, the button records immediately; when it is not, one tap opens Pennel, Pennel starts the bounded session, and after the user returns to the original app the keyboard starts listening.

## Findings that shaped the design

- Wispr Flow uses a time-limited background microphone session started by its containing app. Its keyboard opens the app with a URL. Automatic return relied on undocumented host-app identification, which stopped working in iOS 26.4; Wispr now shows a "Swipe back to your app" screen. Apple DTS confirms no public return API exists (Apple Developer Forums thread 826851).
- Opening the containing app from a keyboard works on iOS 26+ only with Full Access (KeyboardKit #968). Pennel dictation already requires Full Access.
- Pennel's `DictationSession` already implements the bounded `off → ready → listening → processing` session with `UIBackgroundModes: audio`, a Live Activity and the `<scheme>://sessie/start` link.

## Decision

This replaces the earlier #39 constraint that the keyboard never opens the containing app. The user accepts the App Review guideline 4.4.1 risk, consistent with comparable approved keyboards.

## Approaches

1. **Open Pennel, start session, swipe back (chosen).** Public APIs only; works on iOS 26.4+ and 27.
2. Automatic return through undocumented host identification. Rejected: broken on iOS 26.4+ and not reviewable.
3. Start only through Action Button, Shortcuts or Control Center. Kept as existing complementary paths; does not meet the one-tap request.

## Design

- **Look (keyboard only):** a keyboard-local native palette (white/gray keys in light, translucent gray in dark), the system keyboard backdrop behind clear content, SF system font, native key radius, shadow and gaps. The rest of the mobile app keeps its Figtree identity; the dictation button uses a high-contrast tinted glass capsule (white on dark, near-black on light).
- **Layout (follow-up, 13 November):** like Wispr Flow, the keyboard opens on numbers and punctuation; ABC reveals the letters. Keys are uniform and rounded, the space bar reads "Pennel" and return shows ↵. The globe key appears only where iOS shows no switch bar.
- **Top bar:** a Liquid Glass ≡ button on the left (emoji, snippets, AI writing, suggestions toggle), suggestions or live status in the middle, and a tinted Liquid Glass "Start" capsule with a waveform on the right. It shrinks to a waveform when ready, animates while listening and shows progress while processing.
- **Session off:** the button opens `<scheme>://sessie/start` (SwiftUI `openURL`, then the scene responder as fallback). It records a pending start. If opening fails, it shows "Open Pennel and tap Start session."
- **App:** the link starts the session in native code and Expo Router shows a full-screen "Swipe back" page with the remaining session time and a stop option.
- **Return:** while the pending start is younger than 60 seconds and the keyboard has a live insertion context, the keyboard starts hands-free listening as soon as the session reads ready. The pending start is cleared after use, on expiry, or on any explicit dictation action.
- No automatic app return, no undocumented host lookup, no microphone outside the existing bounded session.

## Left to the builder

Exact native palette values and metrics, copy wording and the swipe-back page layout.
