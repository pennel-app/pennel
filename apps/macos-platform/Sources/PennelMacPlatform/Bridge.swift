import AppKit
import ApplicationServices
import AVFoundation
import EventKit
import CoreAudio
import Foundation
import ObjectiveC.runtime
import Speech

typealias EventSink = @convention(c) (UnsafeMutableRawPointer?, UInt32, Bool) -> Void
typealias TextSink = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Void

private func completion(_ sink: TextSink, _ context: UnsafeMutableRawPointer?, value: String?, error: String? = nil) {
    let value = value.flatMap { strdup($0) }
    let error = error.flatMap { strdup($0) }
    defer { free(value); free(error) }
    sink(context, value.map { UnsafePointer<CChar>($0) }, error.map { UnsafePointer<CChar>($0) })
}

private func writeError(_ target: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ error: Error) {
    target?.pointee = bridgeString(error.localizedDescription)
}

@_cdecl("pennel_mac_string_free")
public func stringFree(_ pointer: UnsafeMutablePointer<CChar>?) { free(pointer) }

@_cdecl("pennel_mac_is_main_thread")
public func isMainThread() -> Bool { Thread.isMainThread }

@_cdecl("pennel_mac_preferred_languages")
public func preferredLanguagesJSON() -> UnsafeMutablePointer<CChar>? {
    bridgeJSON(Locale.preferredLanguages)
}

@_cdecl("pennel_mac_permissions")
public func permissionsJSON() -> UnsafeMutablePointer<CChar>? {
    func audioState(_ value: AVAuthorizationStatus) -> String {
        switch value {
        case .authorized: "granted"
        case .denied: "denied"
        case .restricted: "restricted"
        default: "not_determined"
        }
    }
    func calendarState(_ value: EKAuthorizationStatus) -> String {
        switch value {
        case .fullAccess: "granted"
        case .denied: "denied"
        case .restricted: "restricted"
        case .writeOnly: "write_only"
        default: "not_determined"
        }
    }
    let speech: String
    switch SFSpeechRecognizer.authorizationStatus() {
    case .authorized: speech = "granted"
    case .denied: speech = "denied"
    case .restricted: speech = "restricted"
    default: speech = "not_determined"
    }
    let appleSpeech: Bool
    if #available(macOS 26, *) { appleSpeech = true } else { appleSpeech = false }
    return bridgeJSON([
        "microphone": audioState(AVCaptureDevice.authorizationStatus(for: .audio)),
        "accessibility": AXIsProcessTrusted() ? "granted" : "denied",
        "calendar": calendarState(EKEventStore.authorizationStatus(for: .event)),
        "speech": speech,
        "system_audio": "unknown",
        "apple_speech": appleSpeech,
    ])
}

@_cdecl("pennel_mac_request_permission")
public func requestPermission(_ rawKind: UnsafePointer<CChar>?) -> Bool {
    guard let kind = decodeString(rawKind), ["microphone", "accessibility", "calendar", "speech", "system_audio"].contains(kind) else { return false }
    Task { @MainActor in
        switch kind {
        case "microphone":
            if MicrophoneAccess.state == .denied {
                if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") { NSWorkspace.shared.open(url) }
            } else {
                _ = await AVCaptureDevice.requestAccess(for: .audio)
            }
        case "calendar": _ = try? await EKEventStore().requestFullAccessToEvents()
        case "speech":
            if speechPermissionShouldOpenSettings(SFSpeechRecognizer.authorizationStatus()) {
                if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition") { NSWorkspace.shared.open(url) }
            } else {
                SFSpeechRecognizer.requestAuthorization { _ in }
            }
        case "accessibility":
            _ = AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") { NSWorkspace.shared.open(url) }
        default:
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture") { NSWorkspace.shared.open(url) }
        }
    }
    return true
}

func speechPermissionShouldOpenSettings(_ status: SFSpeechRecognizerAuthorizationStatus) -> Bool {
    status == .denied || status == .restricted
}

@_cdecl("pennel_mac_foreground")
public func foregroundJSON() -> UnsafeMutablePointer<CChar>? {
    onMain {
        guard let app = NSWorkspace.shared.frontmostApplication else { return bridgeString("null") }
        return bridgeJSON(["pid": app.processIdentifier, "bundle_id": app.bundleIdentifier ?? ""])
    }
}

@_cdecl("pennel_mac_pick_style_application")
func pickStyleApplicationJSON(_ rawOwner: UnsafeMutableRawPointer?, _ rawTitle: UnsafePointer<CChar>?, _ rawPrompt: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    guard let rawOwner, let title = decodeString(rawTitle), let prompt = decodeString(rawPrompt) else {
        completion(sink, context, value: nil, error: "The app picker has no owning window.")
        return
    }
    let owner = onMain { Unmanaged<NSWindow>.fromOpaque(rawOwner).takeUnretainedValue() }
    Task { @MainActor in
        do {
            guard let selection = try await pickStyleApplication(title: title, prompt: prompt, owner: owner) else {
                completion(sink, context, value: "null")
                return
            }
            let data = try JSONSerialization.data(withJSONObject: ["id": selection.id, "name": selection.name])
            completion(sink, context, value: String(decoding: data, as: UTF8.self))
        } catch {
            completion(sink, context, value: nil, error: error.localizedDescription)
        }
    }
}

@_cdecl("pennel_mac_localize_style_application_picker")
public func localizeStyleApplicationPicker(_ rawTitle: UnsafePointer<CChar>?, _ rawPrompt: UnsafePointer<CChar>?) {
    guard let title = decodeString(rawTitle), let prompt = decodeString(rawPrompt) else { return }
    DispatchQueue.main.async { updateStyleApplicationPicker(title: title, prompt: prompt) }
}

@_cdecl("pennel_mac_external_target_start")
public func externalTargetStart() {
    onMain { MainActor.assumeIsolated { ExternalPasteCoordinator.shared.startTracking() } }
}

@_cdecl("pennel_mac_external_target_capture")
public func externalTargetCapture() -> UnsafeMutablePointer<CChar>? {
    let target = onMain { MainActor.assumeIsolated { ExternalPasteCoordinator.shared.capture() } }
    guard let target else { return bridgeString("null") }
    return bridgeJSON(["token": target.token, "name": target.name, "bundle_id": target.bundleID])
}

@_cdecl("pennel_mac_external_target_capture_current")
public func externalTargetCaptureCurrent() -> UnsafeMutablePointer<CChar>? {
    let target = onMain { MainActor.assumeIsolated { ExternalPasteCoordinator.shared.captureCurrentExternal() } }
    guard let target else { return bridgeString("null") }
    return bridgeJSON(["token": target.token, "name": target.name, "bundle_id": target.bundleID])
}

@_cdecl("pennel_mac_external_target_cancel")
public func externalTargetCancel(_ rawToken: UnsafePointer<CChar>?) {
    guard let token = decodeString(rawToken) else { return }
    onMain { MainActor.assumeIsolated { ExternalPasteCoordinator.shared.cancel(token) } }
}

@_cdecl("pennel_mac_external_target_is_current")
public func externalTargetIsCurrent(_ rawToken: UnsafePointer<CChar>?) -> Bool {
    guard let token = decodeString(rawToken) else { return false }
    return onMain { MainActor.assumeIsolated { ExternalPasteCoordinator.shared.isCurrent(token) } }
}

@_cdecl("pennel_mac_external_target_paste")
func externalTargetPaste(_ rawText: UnsafePointer<CChar>?, _ rawToken: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    guard let text = decodeString(rawText), let token = decodeString(rawToken) else {
        completion(sink, context, value: nil, error: "Invalid paste destination")
        return
    }
    Task { @MainActor in
        do {
            try await ExternalPasteCoordinator.shared.paste(text: text, token: token)
            completion(sink, context, value: "")
        } catch {
            completion(sink, context, value: nil, error: error.localizedDescription)
        }
    }
}

@_cdecl("pennel_mac_other_instances")
public func otherInstancesJSON() -> UnsafeMutablePointer<CChar>? {
    onMain {
        let ownPID = ProcessInfo.processInfo.processIdentifier
        let identities = Set(["dev.pennel.Pennel", "dev.musli.Musli"])
        let instances = NSWorkspace.shared.runningApplications.compactMap { app -> [String: Any]? in
            guard app.processIdentifier != ownPID, let bundleID = app.bundleIdentifier,
                  identities.contains(bundleID), !app.isTerminated else { return nil }
            return ["pid": app.processIdentifier, "bundle_id": bundleID]
        }
        return bridgeJSON(instances)
    }
}

@_cdecl("pennel_mac_read_selection")
func readSelection(_ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    Task { @MainActor in
        guard AXIsProcessTrusted() else { completion(sink, context, value: nil, error: "Give Pennel Accessibility access"); return }
        let text = await SelectionReader.read()
        completion(sink, context, value: text)
    }
}

@_cdecl("pennel_mac_paste")
func paste(_ rawText: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    let text = decodeString(rawText) ?? ""
    Task { @MainActor in
        guard AXIsProcessTrusted() else { completion(sink, context, value: nil, error: "Give Pennel Accessibility access"); return }
        await TextInserter.insert(text)
        completion(sink, context, value: "")
    }
}

@_cdecl("pennel_mac_copy_text")
public func copyText(_ rawText: UnsafePointer<CChar>?, _ rawHTML: UnsafePointer<CChar>?) -> Bool {
    let text = decodeString(rawText) ?? ""
    let html = decodeString(rawHTML)
    return onMain {
        let item = NSPasteboardItem()
        guard item.setString(text, forType: .string) else { return false }
        if let html, !item.setString(html, forType: .html) { return false }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        return pasteboard.writeObjects([item])
    }
}

@_cdecl("pennel_mac_copy_secret")
public func copySecret(_ rawText: UnsafePointer<CChar>?) -> Bool {
    let text = decodeString(rawText) ?? ""
    return onMain {
        let item = NSPasteboardItem()
        guard item.setString(text, forType: .string) else { return false }
        // Conventions honoured by clipboard managers so the secret stays out of their history.
        item.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType"))
        item.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        return pasteboard.writeObjects([item])
    }
}

private final class NativeHotkeys {
    var monitor: HotkeyMonitor?
    var shortcut: ShortcutTap?
    var active = true
    var shiftHeld = false
    var recordShortcut = RecordShortcutPreference()
    let sink: EventSink
    let context: UnsafeMutableRawPointer?

    init(sink: @escaping EventSink, context: UnsafeMutableRawPointer?) { self.sink = sink; self.context = context }

    func start(trigger: NativeHotkeyTrigger) -> Bool {
        monitor = HotkeyMonitor(trigger: trigger, onEdge: { [weak self] edge, _ in
            guard let self, self.active else { return }
            self.sink(self.context, edge == .down ? 0 : 1, false)
        }, onInterrupt: { [weak self] in
            guard let self, self.active else { return }
            self.sink(self.context, 2, false)
        }, onShift: { [weak self] down in
            guard let self, self.active else { return }
            self.shiftHeld = down
            self.sink(self.context, 3, down)
        })
        shortcut = ShortcutTap { [weak self] in
            guard let self, self.active, self.recordShortcut.enabled else { return }
            self.sink(self.context, 4, false)
        }
        guard monitor?.start() == true else { return false }
        return true
    }

    func setRecordShortcut(_ enabled: Bool) -> Bool {
        recordShortcut.update(enabled, start: { shortcut?.start() == true }, stop: { shortcut?.stop() })
    }

    func stop() { active = false; _ = setRecordShortcut(false); monitor?.stop(); shortcut?.stop() }
}

@_cdecl("pennel_mac_hotkey_start")
func hotkeyStart(_ rawTrigger: UnsafePointer<CChar>?, _ sink: @escaping EventSink, _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer? {
    let trigger = NativeHotkeyTrigger(rawValue: decodeString(rawTrigger) ?? "fn") ?? .fn
    return onMain {
        guard AXIsProcessTrusted() else { return nil }
        let keys = NativeHotkeys(sink: sink, context: context)
        guard keys.start(trigger: trigger) else { keys.stop(); return nil }
        return Unmanaged.passRetained(keys).toOpaque()
    }
}

@_cdecl("pennel_mac_hotkey_set_trigger")
public func hotkeySetTrigger(_ handle: UnsafeMutableRawPointer?, _ rawTrigger: UnsafePointer<CChar>?) {
    guard let handle, let trigger = NativeHotkeyTrigger(rawValue: decodeString(rawTrigger) ?? "") else { return }
    onMain { Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue().monitor?.setTrigger(trigger) }
}

@_cdecl("pennel_mac_hotkey_set_record_shortcut_enabled")
public func hotkeySetRecordShortcut(_ handle: UnsafeMutableRawPointer?, _ enabled: Bool) -> Bool {
    guard let handle else { return false }
    return onMain { Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue().setRecordShortcut(enabled) }
}

@_cdecl("pennel_mac_hotkey_held")
public func hotkeyHeld(_ handle: UnsafeMutableRawPointer?, _ shift: Bool) -> Bool {
    guard let handle else { return false }
    return onMain {
        let keys = Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue()
        return shift ? keys.shiftHeld : keys.monitor?.isKeyDown == true
    }
}

@_cdecl("pennel_mac_hotkey_stop")
public func hotkeyStop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    onMain { Unmanaged<NativeHotkeys>.fromOpaque(handle).takeRetainedValue().stop() }
}

@_cdecl("pennel_mac_call_snapshot")
public func callSnapshot(_ rawApps: UnsafePointer<CChar>?, _ rawBrowsers: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    func strings(_ raw: UnsafePointer<CChar>?) -> [String] {
        guard let text = decodeString(raw), let data = text.data(using: .utf8) else { return [] }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String] ?? []
    }
    let apps = strings(rawApps)
    let browsers = strings(rawBrowsers)
    return onMain {
        let users = MicUsageMonitor(callApps: apps, browsers: browsers).snapshot()
        return bridgeJSON(users.sorted { $0.bundleId < $1.bundleId }.map {
            ["app_id": $0.bundleId, "is_call": $0.isCall, "is_playing": $0.isPlaying] as [String: Any]
        })
    }
}

private var calendarProvider: CalendarProvider?

@_cdecl("pennel_mac_calendar_snapshot")
public func calendarSnapshot() -> UnsafeMutablePointer<CChar>? {
    onMain {
        if calendarProvider == nil { calendarProvider = CalendarProvider() }
        let provider = calendarProvider!
        let current: Any = provider.currentEvent().map { event in
            ["title": event.title, "calendar_id": event.calendarId, "calendar_name": event.calendarName, "attendees": event.attendees] as [String: Any]
        } ?? NSNull()
        return bridgeJSON([
            "has_access": provider.hasAccess,
            "calendars": provider.calendars().map { ["id": $0.id, "name": $0.name] },
            "current_event": current,
        ])
    }
}

@_cdecl("pennel_mac_capture_start")
func captureStart(_ source: UInt32, _ sink: @escaping AudioSink, _ context: UnsafeMutableRawPointer?, _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
    onMain {
        guard source <= 1 else { errorOut?.pointee = bridgeString("Unknown audio source"); return nil }
        let capture = NativeCapture()
        do {
            try capture.start(source: source, sink: sink, context: context)
            return Unmanaged.passRetained(capture).toOpaque()
        } catch { writeError(errorOut, error); return nil }
    }
}

@_cdecl("pennel_mac_capture_stop")
public func captureStop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    onMain { Unmanaged<NativeCapture>.fromOpaque(handle).takeRetainedValue().stop() }
}

@_cdecl("pennel_mac_meeting_start")
func meetingStart(_ rawDirectory: UnsafePointer<CChar>?, _ rawApp: UnsafePointer<CChar>?, _ echo: Bool, _ sink: @escaping AudioSink, _ context: UnsafeMutableRawPointer?, _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
    guard let directory = decodeString(rawDirectory) else { errorOut?.pointee = bridgeString("No meeting folder specified"); return nil }
    let app = decodeString(rawApp)
    return onMain {
        let capture = NativeMeetingRecorder()
        do {
            let processes: (() -> [AudioObjectID])? = app.map { app in { MicUsageMonitor.processObjects(bundleId: app) } }
            try capture.start(directory: URL(fileURLWithPath: directory), sink: sink, context: context, tapProcesses: processes, echoCancellation: echo)
            return Unmanaged.passRetained(capture).toOpaque()
        } catch { _ = capture.stop(); writeError(errorOut, error); return nil }
    }
}

@_cdecl("pennel_mac_meeting_status")
public func meetingStatus(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>? {
    guard let handle else { return bridgeString("null") }
    return onMain {
        let capture = Unmanaged<NativeMeetingRecorder>.fromOpaque(handle).takeUnretainedValue()
        return bridgeJSON(capture.healthSnapshot)
    }
}

@_cdecl("pennel_mac_meeting_start_paths")
func meetingStartPaths(_ rawDirectory: UnsafePointer<CChar>?, _ rawApp: UnsafePointer<CChar>?, _ rawMicrophone: UnsafePointer<CChar>?, _ rawSystem: UnsafePointer<CChar>?, _ echo: Bool, _ sink: @escaping AudioSink, _ context: UnsafeMutableRawPointer?, _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
    guard let directory = decodeString(rawDirectory), let microphone = decodeString(rawMicrophone), let system = decodeString(rawSystem) else {
        errorOut?.pointee = bridgeString("No recording paths provided")
        return nil
    }
    let app = decodeString(rawApp)
    return onMain {
        let capture = NativeMeetingRecorder()
        do {
            let processes: (() -> [AudioObjectID])? = app.map { app in { MicUsageMonitor.processObjects(bundleId: app) } }
            try capture.start(directory: URL(fileURLWithPath: directory), sink: sink, context: context,
                              tapProcesses: processes, echoCancellation: echo,
                              microphoneURL: URL(fileURLWithPath: microphone), systemURL: URL(fileURLWithPath: system))
            return Unmanaged.passRetained(capture).toOpaque()
        } catch { _ = capture.stop(); writeError(errorOut, error); return nil }
    }
}

@_cdecl("pennel_mac_meeting_stop_status")
public func meetingStopStatus(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>? {
    guard let handle else { return bridgeString("null") }
    return onMain {
        let capture = Unmanaged<NativeMeetingRecorder>.fromOpaque(handle).takeRetainedValue()
        let error = capture.stop()
        var snapshot = capture.healthSnapshot
        if let error { snapshot["terminal_error"] = error }
        return bridgeJSON(snapshot)
    }
}

@_cdecl("pennel_mac_meeting_stop")
public func meetingStop(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>? {
    guard let handle else { return nil }
    return onMain { Unmanaged<NativeMeetingRecorder>.fromOpaque(handle).takeRetainedValue().stop().flatMap(bridgeString) }
}

@_cdecl("pennel_mac_transcribe")
func transcribe(_ samples: UnsafePointer<Float>?, _ count: Int, _ rate: UInt32, _ rawLanguage: UnsafePointer<CChar>?, _ rawVocabulary: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    guard let samples, count > 0, rate > 0 else { completion(sink, context, value: nil, error: "No usable audio"); return }
    let owned = Array(UnsafeBufferPointer(start: samples, count: count))
    let language = decodeString(rawLanguage)
    let vocabulary: [String]
    if let json = decodeString(rawVocabulary) {
        guard let words = try? JSONDecoder().decode([String].self, from: Data(json.utf8)) else {
            completion(sink, context, value: nil, error: "Invalid speech vocabulary")
            return
        }
        vocabulary = words
    } else { vocabulary = [] }
    Task.detached {
        guard #available(macOS 26, *) else { completion(sink, context, value: nil, error: "Apple Dictation requires macOS 26 or later"); return }
        do {
            let text = try AppleSpeechTranscriber().transcribe(samples: owned, sampleRate: rate, language: language, vocabulary: vocabulary)
            completion(sink, context, value: text)
        } catch { completion(sink, context, value: nil, error: error.localizedDescription) }
    }
}

private final class LiveSpeechSink: @unchecked Sendable {
    let sink: TextSink
    let context: UnsafeMutableRawPointer?

    init(sink: @escaping TextSink, context: UnsafeMutableRawPointer?) {
        self.sink = sink
        self.context = context
    }

    func emit(_ text: String) { completion(sink, context, value: text) }
}

@_cdecl("pennel_mac_speech_preview_start")
func speechPreviewStart(_ rawLanguage: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer? {
    let callback = LiveSpeechSink(sink: sink, context: context)
    let preview = LiveSpeechPreview(language: decodeString(rawLanguage), onText: { callback.emit($0) })
    guard preview.start() else { preview.stop(); return nil }
    return Unmanaged.passRetained(preview).toOpaque()
}

@_cdecl("pennel_mac_speech_preview_feed")
public func speechPreviewFeed(_ handle: UnsafeMutableRawPointer?, _ samples: UnsafePointer<Float>?, _ count: Int) {
    guard let handle, let samples, count > 0 else { return }
    let preview = Unmanaged<LiveSpeechPreview>.fromOpaque(handle).takeUnretainedValue()
    preview.feed(Array(UnsafeBufferPointer(start: samples, count: count)))
}

@_cdecl("pennel_mac_speech_preview_stop")
public func speechPreviewStop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    // stop drains the repeated sink before Rust releases its callback allocation.
    let preview = Unmanaged<LiveSpeechPreview>.fromOpaque(handle).takeRetainedValue()
    preview.stop()
    if !Thread.isMainThread {
        let finished = DispatchSemaphore(value: 0)
        Task.detached { await preview.stopAndWait(); finished.signal() }
        finished.wait()
    }
}

@_cdecl("pennel_mac_decode_audio")
public func decodeAudio(_ rawPath: UnsafePointer<CChar>?, _ count: UnsafeMutablePointer<Int>?, _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutablePointer<Float>? {
    guard let path = decodeString(rawPath) else { return nil }
    do {
        let samples = try AudioDecoder.monoSamples(of: URL(fileURLWithPath: path))
        count?.pointee = samples.count
        guard !samples.isEmpty else { return nil }
        let result = UnsafeMutablePointer<Float>.allocate(capacity: samples.count)
        result.initialize(from: samples, count: samples.count)
        return result
    } catch { writeError(errorOut, error); return nil }
}

@_cdecl("pennel_mac_language_model_availability")
public func languageModelAvailabilityJSON() -> UnsafeMutablePointer<CChar>? {
    let state = FoundationLanguageModel.availability()
    return bridgeJSON(["status": state.status, "reason": state.reason as Any? ?? NSNull()])
}

@_cdecl("pennel_mac_language_model_complete")
func languageModelComplete(_ rawMessages: UnsafePointer<CChar>?, _ timeoutSecs: UInt64, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    guard let json = decodeString(rawMessages) else {
        completion(sink, context, value: nil, error: NativeLanguageModelError.invalidRequest.localizedDescription)
        return
    }
    let request: NativeLanguageModelRequest
    do { request = try NativeLanguageModelRequest(json: json) }
    catch {
        completion(sink, context, value: nil, error: error.localizedDescription)
        return
    }
    Task.detached {
        do {
            let text = try await FoundationLanguageModel.complete(messages: request.messages, timeoutSecs: timeoutSecs)
            completion(sink, context, value: text)
        } catch { completion(sink, context, value: nil, error: error.localizedDescription) }
    }
}

@_cdecl("pennel_mac_float_free")
public func floatFree(_ pointer: UnsafeMutablePointer<Float>?, _ count: Int) {
    pointer?.deinitialize(count: count)
    pointer?.deallocate()
}

@_cdecl("pennel_mac_play_audio")
public func playAudio(_ rawPath: UnsafePointer<CChar>?, _ offset: UInt64) -> UnsafeMutablePointer<CChar>? {
    guard let path = decodeString(rawPath) else { return bridgeString("No audio file specified") }
    return onMain {
        guard FileManager.default.fileExists(atPath: path) else { return bridgeString("Audio file not found") }
        AudioScrubber.shared.play(URL(fileURLWithPath: path), fromMs: offset)
        return nil
    }
}

@_cdecl("pennel_mac_stop_audio")
public func stopAudio() { onMain { AudioScrubber.shared.stop() } }

private final class PennelOverlayPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

private var overlayHostKey: UInt8 = 0

private final class OverlayHost: NSObject {
    let panel: PennelOverlayPanel

    init(window: NSWindow, interactive: Bool) {
        panel = PennelOverlayPanel(contentRect: window.frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        super.init()
        panel.identifier = NSUserInterfaceItemIdentifier("PennelOverlayPanel")
        panel.title = window.title
        panel.isReleasedWhenClosed = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.ignoresMouseEvents = !interactive
        panel.hasShadow = false
        panel.backgroundColor = .clear
        panel.isOpaque = false
        // Keep the webview object owned by Tauri, and move only its AppKit presentation container.
        // Changing an existing NSWindow's isa breaks AppKit's KVO bookkeeping on recent macOS.
        if let content = window.contentView {
            window.contentView = NSView(frame: content.frame)
            panel.contentView = content
        }
    }

    deinit {
        panel.orderOut(nil)
        panel.contentView = nil
        panel.close()
    }
}

private func overlayHost(_ window: NSWindow) -> OverlayHost? {
    objc_getAssociatedObject(window, &overlayHostKey) as? OverlayHost
}

@_cdecl("pennel_mac_configure_panel")
public func configurePanel(_ pointer: UnsafeMutableRawPointer?, _ interactive: Bool) {
    guard let pointer else { return }
    onMain {
        let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
        if let host = overlayHost(window) {
            host.panel.ignoresMouseEvents = !interactive
        } else {
            let host = OverlayHost(window: window, interactive: interactive)
            objc_setAssociatedObject(window, &overlayHostKey, host, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        }
    }
}

@_cdecl("pennel_mac_show_panel")
public func showPanel(_ pointer: UnsafeMutableRawPointer?) {
    guard let pointer else { return }
    onMain {
        let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
        guard let host = overlayHost(window) else { window.orderFrontRegardless(); return }
        host.panel.setFrame(window.frame, display: false)
        host.panel.orderFrontRegardless()
    }
}

/// Bottom-centre of a screen's visible area, shrunk to fit it.
func bottomOverlayFrame(visible: NSRect, width: CGFloat, height: CGFloat) -> NSRect {
    let size = NSSize(width: min(max(width, 1), visible.width), height: min(max(height, 1), visible.height))
    return NSRect(x: (visible.midX - size.width / 2).rounded(), y: visible.minY, width: size.width, height: size.height)
}

/// Places the overlay on the screen under the pointer before it is ordered in. Tauri's position and
/// size setters are applied asynchronously, so a first show would otherwise use the default centred frame.
@_cdecl("pennel_mac_show_panel_at_bottom")
public func showPanelAtBottom(_ pointer: UnsafeMutableRawPointer?, _ width: Double, _ height: Double) {
    guard let pointer else { return }
    onMain {
        let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
        let mouse = NSEvent.mouseLocation
        if let screen = NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) }) ?? NSScreen.main {
            let frame = bottomOverlayFrame(visible: screen.visibleFrame, width: width, height: height)
            window.setFrame(frame, display: false)
        }
        guard let host = overlayHost(window) else { window.orderFrontRegardless(); return }
        host.panel.setFrame(window.frame, display: false)
        host.panel.orderFrontRegardless()
    }
}

/// Top-right corner of a screen's visible area with a margin, shrunk to fit it.
func topRightOverlayFrame(visible: NSRect, width: CGFloat, height: CGFloat, margin: CGFloat) -> NSRect {
    let size = NSSize(width: min(max(width, 1), visible.width), height: min(max(height, 1), visible.height))
    let inset = min(margin, max(0, visible.width - size.width), max(0, visible.height - size.height))
    return NSRect(x: (visible.maxX - size.width - inset).rounded(), y: (visible.maxY - size.height - inset).rounded(), width: size.width, height: size.height)
}

/// Places the record prompt top-right on the screen under the pointer before it is ordered in,
/// for the same reason as `showPanelAtBottom`.
@_cdecl("pennel_mac_show_panel_at_top_right")
public func showPanelAtTopRight(_ pointer: UnsafeMutableRawPointer?, _ width: Double, _ height: Double, _ margin: Double) {
    guard let pointer else { return }
    onMain {
        let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
        let mouse = NSEvent.mouseLocation
        if let screen = NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) }) ?? NSScreen.main {
            let frame = topRightOverlayFrame(visible: screen.visibleFrame, width: width, height: height, margin: margin)
            window.setFrame(frame, display: false)
        }
        guard let host = overlayHost(window) else { window.orderFrontRegardless(); return }
        host.panel.setFrame(window.frame, display: false)
        host.panel.orderFrontRegardless()
    }
}

@_cdecl("pennel_mac_hide_panel")
public func hidePanel(_ pointer: UnsafeMutableRawPointer?) {
    guard let pointer else { return }
    onMain {
        let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
        if let host = overlayHost(window) { host.panel.orderOut(nil) }
        window.orderOut(nil)
    }
}

@_cdecl("pennel_mac_set_accessory")
public func setAccessory() { onMain { _ = NSApplication.shared.setActivationPolicy(.accessory) } }
