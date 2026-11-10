import AppKit
import ApplicationServices
import AVFoundation
import EventKit
import CoreAudio
import Foundation
import ObjectiveC.runtime
import Speech

typealias EventSink = @convention(c) (UnsafeMutableRawPointer?, UInt32, Bool) -> Void
typealias ShortcutEventSink = @convention(c) (UnsafeMutableRawPointer?, UInt32, UInt32) -> Void
typealias CaptureScopeSink = @convention(c) (UnsafeMutableRawPointer?, Bool) -> Void
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
    var matcher: ShortcutMatcher
    var captureSink: TextSink?
    var captureContext: UnsafeMutableRawPointer?
    let sink: (UInt32, UInt32) -> Void

    init(table: ShortcutTable, sink: @escaping (UInt32, UInt32) -> Void) {
        matcher = ShortcutMatcher(table: table); self.sink = sink
    }
    func start() -> Bool {
        monitor = HotkeyMonitor(onInput: { [weak self] type, event in
            guard let self, self.active, !self.matcher.capturing else { return }
            // The swallowing layer owns key events while installed; flags/mouse remain listen-only.
            if type == .flagsChanged || type == .leftMouseDown || type == .rightMouseDown || type == .otherMouseDown || self.shortcut?.isRunning != true {
                _ = self.process(type, event)
            }
        }, onReset: { [weak self] in
            guard let self, self.active else { return }
            self.resetHold()
        })
        shortcut = ShortcutTap(onInput: { [weak self] type, event in
            guard let self, self.active else { return false }
            if type == .flagsChanged && !self.matcher.capturing { return false }
            return self.process(type, event)
        })
        guard monitor?.start() == true else { return false }
        return refreshTap()
    }
    func resetHold() {
        if matcher.isHeld { sink(2, 0); sink(1, 0) }
        matcher.setTable(matcher.table)
        shiftHeld = false; sink(3, 0)
    }
    func setTable(_ table: ShortcutTable) -> Bool {
        guard monitor?.start() == true else { matcher.setTable(table); shortcut?.stop(); return false }
        if table == matcher.table { return refreshTap() }
        if table.hold != matcher.table.hold { resetHold() }
        matcher.setTable(table, preserveHold: true); return refreshTap()
    }
    func refreshTap() -> Bool {
        if matcher.needsSwallowingTap { return shortcut?.start() == true }
        shortcut?.stop(); return true
    }
    func setListening(_ value: Bool) -> Bool { matcher.setListening(value); return refreshTap() }
    func capture(_ sink: TextSink?, _ context: UnsafeMutableRawPointer?) -> Bool {
        if sink != nil && (matcher.listening || !AXIsProcessTrusted()) { return false }
        captureSink = sink; captureContext = context
        matcher.setCapture(sink != nil)
        guard refreshTap() else {
            captureSink = nil; captureContext = nil; matcher.setCapture(false); return false
        }
        return true
    }
    private func process(_ type: CGEventType, _ event: CGEvent) -> Bool {
        let input: ShortcutInput
        switch type {
        case .keyDown: input = .down
        case .keyUp: input = .up
        case .flagsChanged: input = .flags
        case .leftMouseDown, .rightMouseDown, .otherMouseDown: input = .mouse
        default: return false
        }
        let code = event.getIntegerValueField(.keyboardEventKeycode)
        let capturing = matcher.capturing
        let decision = matcher.route(input, code: code, flags: event.flags.rawValue, repeating: event.getIntegerValueField(.keyboardEventAutorepeat) != 0)
        for signal in decision.signals {
            switch signal {
            case .down: sink(0, 0)
            case .up: sink(1, 0)
            case .interrupt: sink(2, 0)
            case .shift(let down): shiftHeld = down; sink(3, down ? 1 : 0)
            case .action(let action): sink(4, action)
            }
        }
        if capturing, let captureSink, input != .mouse {
            // AppKit reapplies the current layout without modifiers, including AZERTY base characters.
            let character = (type == .keyDown || type == .keyUp) ? NSEvent(cgEvent: event)?.characters(byApplyingModifiers: []) : nil
            let value: [String: Any] = ["code": code, "flags": event.flags.rawValue, "repeat": event.getIntegerValueField(.keyboardEventAutorepeat) != 0, "kind": type == .keyDown ? "down" : type == .keyUp ? "up" : "flags", "key": character.map { $0 as Any } ?? NSNull()]
            if let data = try? JSONSerialization.data(withJSONObject: value), let json = String(data: data, encoding: .utf8) {
                json.withCString { captureSink(captureContext, $0, nil) }
            }
        }
        // Drain withheld releases before removing the tap; never invalidate it inside its callback.
        if !matcher.needsSwallowingTap {
            DispatchQueue.main.async { [weak self] in guard let self, self.active else { return }; _ = self.refreshTap() }
        }
        return decision.withhold
    }
    func stop() {
        active = false; captureSink = nil; captureContext = nil
        monitor?.stop(); shortcut?.stop()
    }
}

private func decodeShortcutTable(_ raw: UnsafePointer<CChar>?) -> ShortcutTable? {
    guard let text = decodeString(raw), let data = text.data(using: .utf8) else { return nil }
    return try? JSONDecoder().decode(ShortcutTable.self, from: data)
}

@_cdecl("pennel_mac_hotkey_start_bindings")
func hotkeyStartBindings(_ raw: UnsafePointer<CChar>?, _ sink: @escaping ShortcutEventSink, _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer? {
    guard let table = decodeShortcutTable(raw) else { return nil }
    return onMain {
        guard AXIsProcessTrusted() else { return nil }
        let keys = NativeHotkeys(table: table, sink: { kind, value in sink(context, kind, value) })
        guard keys.start() else { keys.stop(); return nil }
        return Unmanaged.passRetained(keys).toOpaque()
    }
}
@_cdecl("pennel_mac_hotkey_set_bindings")
public func hotkeySetBindings(_ handle: UnsafeMutableRawPointer?, _ raw: UnsafePointer<CChar>?) -> Bool {
    guard let handle, let table = decodeShortcutTable(raw) else { return false }
    return onMain { Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue().setTable(table) }
}
@_cdecl("pennel_mac_hotkey_set_listening")
public func hotkeySetListening(_ handle: UnsafeMutableRawPointer?, _ listening: Bool) -> Bool {
    guard let handle else { return false }
    return onMain { Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue().setListening(listening) }
}
@_cdecl("pennel_mac_hotkey_capture")
func hotkeyCapture(_ handle: UnsafeMutableRawPointer?, _ sink: TextSink?, _ context: UnsafeMutableRawPointer?) -> Bool {
    guard let handle else { return false }
    return onMain { Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue().capture(sink, context) }
}

// Retain the original ABI for existing platform clients. New clients always supply the core table.
private func legacyHold(_ trigger: NativeHotkeyTrigger) -> NativeShortcut {
    let mask: UInt64
    switch trigger { case .fn, .controlSuper: mask = 0x800000; case .rightControl: mask = 0x2000; case .rightOption: mask = 0x40; case .rightCommand: mask = 0x10 }
    return NativeShortcut(action: 0, code: nil, mods: [mask])
}
@_cdecl("pennel_mac_hotkey_start")
func hotkeyStart(_ raw: UnsafePointer<CChar>?, _ sink: @escaping EventSink, _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer? {
    let trigger = NativeHotkeyTrigger(rawValue: decodeString(raw) ?? "fn") ?? .fn
    return onMain {
        guard AXIsProcessTrusted() else { return nil }
        let keys = NativeHotkeys(table: ShortcutTable(hold: legacyHold(trigger), always: [], listening: []), sink: { kind, value in sink(context, kind, value != 0) })
        guard keys.start() else { keys.stop(); return nil }
        return Unmanaged.passRetained(keys).toOpaque()
    }
}
@_cdecl("pennel_mac_hotkey_set_trigger")
public func hotkeySetTrigger(_ handle: UnsafeMutableRawPointer?, _ raw: UnsafePointer<CChar>?) {
    guard let handle, let trigger = NativeHotkeyTrigger(rawValue: decodeString(raw) ?? "") else { return }
    onMain { let keys = Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue(); var table = keys.matcher.table; table.hold = legacyHold(trigger); _ = keys.setTable(table) }
}
@_cdecl("pennel_mac_hotkey_set_record_shortcut_enabled")
public func hotkeySetRecordShortcut(_ handle: UnsafeMutableRawPointer?, _ enabled: Bool) -> Bool {
    guard let handle else { return false }
    return onMain { let keys = Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue(); var table = keys.matcher.table; table.always = enabled ? [NativeShortcut(action: 1, code: 15, mods: [0x100000, 0x20000])] : []; return keys.setTable(table) }
}
@_cdecl("pennel_mac_hotkey_held")
public func hotkeyHeld(_ handle: UnsafeMutableRawPointer?, _ shift: Bool) -> Bool {
    guard let handle else { return false }
    return onMain { let keys = Unmanaged<NativeHotkeys>.fromOpaque(handle).takeUnretainedValue(); return shift ? keys.shiftHeld : keys.matcher.isHeld }
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

/// Runs on the caller's thread: a Teams scan takes tens of milliseconds and must not block the UI.
@_cdecl("pennel_mac_meeting_speakers_snapshot")
public func meetingSpeakersSnapshot(_ rawAppId: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    let snapshot = MeetingSpeakerObserver.snapshot(appId: decodeString(rawAppId) ?? "")
    return bridgeJSON([
        "available": snapshot.available,
        "participants": snapshot.participants,
        "speaking": snapshot.speaking,
        "process_id": (snapshot.processId as Any?) ?? NSNull(),
        "process_started_at_ms": (snapshot.processStartedAtMs as Any?) ?? NSNull(),
        "window_id": (snapshot.windowId as Any?) ?? NSNull(),
        "unavailable_reason": (snapshot.unavailableReason as Any?) ?? NSNull(),
        "duplicate_names": snapshot.duplicateNames,
        "call_ended": snapshot.callEnded,
        "call_active": snapshot.callActive,
    ] as [String: Any])
}

private var calendarProvider: CalendarProvider?

@_cdecl("pennel_mac_calendar_snapshot")
public func calendarSnapshot() -> UnsafeMutablePointer<CChar>? {
    onMain {
        if calendarProvider == nil { calendarProvider = CalendarProvider() }
        let provider = calendarProvider!
        let current: Any = provider.currentEvent().map(\.json) ?? NSNull()
        return bridgeJSON([
            "has_access": provider.hasAccess,
            "calendars": provider.calendars().map { ["id": $0.id, "name": $0.name] },
            "current_event": current,
        ])
    }
}

@_cdecl("pennel_mac_calendar_source_available")
func calendarSourceAvailable(_ rawCalendarId: UnsafePointer<CChar>?, _ rawSourceId: UnsafePointer<CChar>?) -> Bool {
    guard let calendarId = decodeString(rawCalendarId) else { return false }
    let sourceId = decodeString(rawSourceId)
    return onMain {
        if calendarProvider == nil { calendarProvider = CalendarProvider() }
        return calendarProvider!.sourceAvailable(calendarId: calendarId, sourceId: sourceId)
    }
}

@_cdecl("pennel_mac_pick_meeting_contact")
func pickMeetingContactJSON(_ rawOwner: UnsafeMutableRawPointer?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    guard let rawOwner else {
        completion(sink, context, value: nil, error: "The contact picker has no owning window.")
        return
    }
    let owner = onMain { Unmanaged<NSWindow>.fromOpaque(rawOwner).takeUnretainedValue() }
    Task { @MainActor in
        guard let selection = await pickMeetingContact(owner: owner) else {
            completion(sink, context, value: "null")
            return
        }
        guard let data = try? JSONSerialization.data(withJSONObject: selection.json) else {
            completion(sink, context, value: nil, error: "The selected contact could not be read.")
            return
        }
        completion(sink, context, value: String(decoding: data, as: UTF8.self))
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
    startMeetingPaths(rawDirectory, rawApp, rawMicrophone, rawSystem, echo, sink, context, nil, errorOut)
}

@_cdecl("pennel_mac_meeting_start_paths_with_scope")
func meetingStartPathsWithScope(_ rawDirectory: UnsafePointer<CChar>?, _ rawApp: UnsafePointer<CChar>?, _ rawMicrophone: UnsafePointer<CChar>?, _ rawSystem: UnsafePointer<CChar>?, _ echo: Bool, _ sink: @escaping AudioSink, _ context: UnsafeMutableRawPointer?, _ scopeSink: @escaping CaptureScopeSink, _ scopeContext: UnsafeMutableRawPointer?, _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
    startMeetingPaths(rawDirectory, rawApp, rawMicrophone, rawSystem, echo, sink, context,
                      { scopeSink(scopeContext, $0) }, errorOut)
}

private func startMeetingPaths(_ rawDirectory: UnsafePointer<CChar>?, _ rawApp: UnsafePointer<CChar>?, _ rawMicrophone: UnsafePointer<CChar>?, _ rawSystem: UnsafePointer<CChar>?, _ echo: Bool, _ sink: @escaping AudioSink, _ context: UnsafeMutableRawPointer?, _ scopeChanged: ((Bool) -> Void)?, _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> UnsafeMutableRawPointer? {
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
                              microphoneURL: URL(fileURLWithPath: microphone), systemURL: URL(fileURLWithPath: system),
                              scopeChanged: scopeChanged)
            return Unmanaged.passRetained(capture).toOpaque()
        } catch { _ = capture.stop(); writeError(errorOut, error); return nil }
    }
}

@_cdecl("pennel_mac_meeting_quiesce")
public func meetingQuiesce(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    onMain { Unmanaged<NativeMeetingRecorder>.fromOpaque(handle).takeUnretainedValue().quiesce() }
}

@_cdecl("pennel_mac_meeting_stop_status")
public func meetingStopStatus(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>? {
    guard let handle else { return bridgeString("null") }
    let capture = onMain { Unmanaged<NativeMeetingRecorder>.fromOpaque(handle).takeRetainedValue() }
    let tracks = onMain { capture.finishingTracks() }
    // Capture is already quiescent; archival drainage must not occupy AppKit's event loop.
    let meError = tracks.0?.finish()
    let othersError = tracks.1?.finish()
    return onMain {
        let error = capture.completeStop(meError: meError, othersError: othersError)
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
    transcribeSpeech(samples, count, rate, rawLanguage, rawVocabulary, sink, context, timed: false)
}

@_cdecl("pennel_mac_transcribe_timed")
func transcribeTimed(_ samples: UnsafePointer<Float>?, _ count: Int, _ rate: UInt32, _ rawLanguage: UnsafePointer<CChar>?, _ rawVocabulary: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) {
    transcribeSpeech(samples, count, rate, rawLanguage, rawVocabulary, sink, context, timed: true)
}

private func transcribeSpeech(_ samples: UnsafePointer<Float>?, _ count: Int, _ rate: UInt32, _ rawLanguage: UnsafePointer<CChar>?, _ rawVocabulary: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?, timed: Bool) {
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
            let result = try AppleSpeechTranscriber().transcribeTimed(samples: owned, sampleRate: rate, language: language, vocabulary: vocabulary)
            if timed {
                let data = try JSONEncoder().encode(result)
                completion(sink, context, value: String(data: data, encoding: .utf8))
            } else { completion(sink, context, value: result.text) }
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

@_cdecl("pennel_mac_speech_preview_start_timed")
func speechPreviewStartTimed(_ rawLanguage: UnsafePointer<CChar>?, _ rawVocabulary: UnsafePointer<CChar>?, _ sink: @escaping TextSink, _ context: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer? {
    let vocabulary: [String]
    if let json = decodeString(rawVocabulary) {
        guard let words = try? JSONDecoder().decode([String].self, from: Data(json.utf8)) else { return nil }
        vocabulary = words
    } else { vocabulary = [] }
    let callback = LiveSpeechSink(sink: sink, context: context)
    let preview = LiveSpeechPreview(language: decodeString(rawLanguage), vocabulary: vocabulary, onEvent: { event in
        guard let data = try? JSONEncoder().encode(event), let json = String(data: data, encoding: .utf8) else { return }
        callback.emit(json)
    })
    guard preview.start() else { preview.stop(); return nil }
    return Unmanaged.passRetained(preview).toOpaque()
}

@_cdecl("pennel_mac_speech_preview_feed")
public func speechPreviewFeed(_ handle: UnsafeMutableRawPointer?, _ samples: UnsafePointer<Float>?, _ count: Int) {
    guard let handle, let samples, count > 0 else { return }
    let preview = Unmanaged<LiveSpeechPreview>.fromOpaque(handle).takeUnretainedValue()
    preview.feed(Array(UnsafeBufferPointer(start: samples, count: count)))
}

@_cdecl("pennel_mac_speech_preview_feed_at")
public func speechPreviewFeedAt(_ handle: UnsafeMutableRawPointer?, _ samples: UnsafePointer<Float>?, _ count: Int, _ startMS: UInt64) {
    guard let handle, let samples, count > 0 else { return }
    let preview = Unmanaged<LiveSpeechPreview>.fromOpaque(handle).takeUnretainedValue()
    preview.feed(Array(UnsafeBufferPointer(start: samples, count: count)), startMS: startMS)
}

@_cdecl("pennel_mac_speech_preview_stop")
public func speechPreviewStop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    // stop drains the repeated sink before Rust releases its callback allocation.
    let preview = Unmanaged<LiveSpeechPreview>.fromOpaque(handle).takeRetainedValue()
    preview.stop()
    Task.detached { await preview.stopAndWait() }
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
