import AppKit
import ApplicationServices
import CoreAudio

/// Polls Core Audio for processes that currently capture audio input and classifies them as calls.
final class MicUsageMonitor: @unchecked Sendable {
    /// Title fragments of call tabs; specific enough that "Meeting notes" doesn't match.
    static let callTitleHints = ["Meet - ", "Google Meet", "Microsoft Teams", "| Teams", "Zoom Meeting", "Zoom Workplace", "Whereby", "Webex"]

    private let callApps: [String]
    private let browsers: [String]
    private let queue = DispatchQueue(label: "dev.pennel.micusage", qos: .utility)
    private var timer: DispatchSourceTimer?

    init(callApps: [String], browsers: [String]) {
        self.callApps = callApps
        self.browsers = browsers
    }

    func start(interval: TimeInterval = 2, onUpdate: @escaping @MainActor @Sendable (Set<MicUser>) -> Void) {
        stop()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: interval)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            let users = self.snapshot()
            Task { @MainActor in onUpdate(users) }
        }
        timer.resume()
        self.timer = timer
    }

    func stop() {
        timer?.cancel()
        timer = nil
    }

    func snapshot() -> Set<MicUser> {
        var recording = Set<String>()
        var playing = Set<String>()
        for process in Self.processList() {
            guard let bundleId = Self.bundleId(process) else { continue }
            let app = owner(of: bundleId)
            if Self.isRunning(process, kAudioProcessPropertyIsRunningInput) { recording.insert(app) }
            if Self.isRunning(process, kAudioProcessPropertyIsRunningOutput) { playing.insert(app) }
        }
        return Set(recording.map { app in
            MicUser(bundleId: app, isCall: isCall(app), isPlaying: playing.contains(app))
        })
    }

    /// Helper processes (e.g. `com.google.Chrome.helper`) are attributed to their main app.
    private func owner(of bundleId: String) -> String {
        (callApps + browsers).first(where: { bundleId.hasPrefix($0) }) ?? bundleId
    }

    private func isCall(_ app: String) -> Bool {
        if app == MeetingSpeakerObserver.teamsBundleId, let eligible = MeetingSpeakerObserver.callEligibility(appId: app) { return eligible }
        if callApps.contains(app) { return true }
        return browsers.contains(app) && Self.browserShowsCall(app)
    }

    private static func browserShowsCall(_ bundleId: String) -> Bool {
        NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).contains { app in
            windowTitles(pid: app.processIdentifier).contains { title in
                callTitleHints.contains { title.localizedCaseInsensitiveContains($0) }
            }
        }
    }

    private static func windowTitles(pid: pid_t) -> [String] {
        let element = AXUIElementCreateApplication(pid)
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXWindowsAttribute as CFString, &value) == .success,
              let windows = value as? [AXUIElement]
        else { return [] }
        return windows.compactMap { window in
            var title: CFTypeRef?
            guard AXUIElementCopyAttributeValue(window, kAXTitleAttribute as CFString, &title) == .success else { return nil }
            return title as? String
        }
    }

    /// Audio process objects of an app and its helpers, for a tap on just that app.
    static func processObjects(bundleId: String) -> [AudioObjectID] {
        processList().filter { Self.bundleId($0)?.hasPrefix(bundleId) == true }
    }

    private static func processList() -> [AudioObjectID] {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyProcessObjectList,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        let system = AudioObjectID(kAudioObjectSystemObject)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr, size > 0 else { return [] }
        var objects = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(system, &address, 0, nil, &size, &objects) == noErr else { return [] }
        return objects
    }

    private static func isRunning(_ process: AudioObjectID, _ selector: AudioObjectPropertySelector) -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var running: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        return AudioObjectGetPropertyData(process, &address, 0, nil, &size, &running) == noErr && running != 0
    }

    private static func bundleId(_ process: AudioObjectID) -> String? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioProcessPropertyBundleID,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var value: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(process, &address, 0, nil, &size, &value) == noErr, let value else { return nil }
        let id = value.takeRetainedValue() as String
        return id.isEmpty ? nil : id
    }
}
