import AppKit
import ApplicationServices

/// The part of an accessibility element the participant reader needs, so the parser can run on fixtures.
protocol ParticipantNode {
    var role: String { get }
    var subrole: String { get }
    var label: String { get }
    var text: String { get }
    var domClasses: [String] { get }
    var identifier: String { get }
    var selected: Bool { get }
    var checked: Bool? { get }
    var modal: Bool? { get }
    var size: CGSize? { get }
    var children: [Self] { get }
    var childrenReadable: Bool { get }
}

extension ParticipantNode {
    var subrole: String { "" }
    var identifier: String { "" }
    var selected: Bool { false }
    var checked: Bool? { nil }
    var modal: Bool? { nil }
    var size: CGSize? { nil }
    var childrenReadable: Bool { true }
}

enum TeamsCallWindowKind: Equatable {
    case call, other, unreadable
}

struct TeamsCallWindowState {
    var kind: TeamsCallWindowKind
    var focused = false
    var minimized = false
    var visible = true
}

struct MeetingSpeakerSnapshot: Equatable {
    /// False when no participant tile was readable, e.g. the window is minimized or not a meeting.
    var available: Bool
    var participants: [String]
    var speaking: [String]
    var processId: UInt32? = nil
    var processStartedAtMs: UInt64? = nil
    /// Opaque identity of one visible meeting window in one process lifetime.
    var windowId: String? = nil
    var unavailableReason: String? = nil
    var duplicateNames = false
}

/// Reads participant tiles from the Microsoft Teams desktop app (verified against Teams 26289 on macOS).
///
/// A tile is a menu item or image whose accessible label starts with the participant's display name,
/// followed by comma-separated state. The tile's own text element holds the full display name, which
/// matters for "Last, First" names. Teams marks the speaking outline with the `vdi-frame-occlusion`
/// class on a direct child group of the tile; every other class on it is a generated style hash.
enum TeamsParticipantParser {
    static let speakingClass = "vdi-frame-occlusion"
    /// The local user's tile; their speech is already on the microphone track.
    static let selfLabelPrefixes = ["myself video", "mijn video", "mijzelf video", "eigen video"]
    private static let tileRoles: Set<String> = ["AXMenuItem", "AXImage"]
    private static let leafRoles: Set<String> = ["AXStaticText", "AXButton", "AXTextField", "AXTextArea", "AXLink"]
    private static let maxDepth = 48

    /// Classifies call state without reading text values or collecting another window's roster.
    static func callWindowKind<Node: ParticipantNode>(_ root: Node) -> TeamsCallWindowKind {
        var remaining = 4_096
        var complete = true
        var sawMainNavigation = false
        func containsCall(_ node: Node, depth: Int) -> Bool {
            guard remaining > 0, depth < maxDepth, !node.role.isEmpty else { complete = false; return false }
            remaining -= 1
            if tileRoles.contains(node.role) {
                let label = node.label.lowercased()
                if hasTileState(node) || selfLabelPrefixes.contains(where: { label.hasPrefix($0) }) { return true }
            }
            // An unsupported/audio-only call may also be a complete control tree. Only a known
            // main-navigation shape positively establishes a non-call shell.
            if isMainNavigation(node) { sawMainNavigation = true }
            // Chat text values are never inspected; navigation labels require its control shape.
            if leafRoles.contains(node.role) { return false }
            let children = node.children
            guard node.childrenReadable else { complete = false; return false }
            for child in children where containsCall(child, depth: depth + 1) { return true }
            return false
        }
        if containsCall(root, depth: 0) { return .call }
        return complete && (sawMainNavigation || isWindowSharingButton(root)) ? .other : .unreadable
    }

    private static func isWindowSharingButton<Node: ParticipantNode>(_ node: Node) -> Bool {
        // macOS 27.0.1 / Teams 26289 exposes its native sharing-session titlebar button as a
        // separate top-level AXDialog. Only the measured, content-free leaf surface is excluded.
        guard node.role == "AXWindow", node.subrole == "AXDialog", node.modal == false,
              node.size == CGSize(width: 66, height: 20), node.childrenReadable, node.children.count == 1,
              let button = node.children.first, button.role == "AXButton", button.label == "WindowSharingSessionButton",
              button.childrenReadable, button.children.isEmpty, node.label == "Window"
        else { return false }
        return true
    }

    private static func isMainNavigation<Node: ParticipantNode>(_ node: Node) -> Bool {
        if isPersonalNavigation(node) { return true }
        guard ["AXTabGroup", "AXToolbar", "AXGroup"].contains(node.role), node.identifier == "app-bar" else { return false }
        let controls = node.children.filter { ["AXTab", "AXRadioButton", "AXButton"].contains($0.role) }
        let required: Set<String> = ["activity-button", "chat-button", "calendar-button"]
        let shown = Set(controls.map(\.identifier))
        return node.childrenReadable && required.isSubset(of: shown)
            && controls.contains { $0.identifier == "chat-button" && $0.selected }
    }

    private static func isPersonalNavigation<Node: ParticipantNode>(_ node: Node) -> Bool {
        // Measured Teams personal/free shell: navigation uses labelled group wrappers, empty
        // identifiers and checkbox AXValues. Restrict recognition to the complete static shape.
        let names: Set<String> = ["Chat", "Meet", "People", "Copilot", "Communities", "Calendar", "Activity"]
        guard node.role == "AXGroup", node.childrenReadable,
              node.children.count == names.count
        else { return false }
        // Establish the checkbox-only structure before reading any group labels.
        var controls: [Node] = []
        for wrapper in node.children {
            guard wrapper.role == "AXGroup", wrapper.childrenReadable, wrapper.children.count == 1,
                  let control = wrapper.children.first, control.role == "AXCheckBox", control.checked != nil
            else { return false }
            controls.append(control)
        }
        guard node.label == "Apps" else { return false }
        var shown = Set<String>()
        var checkedCount = 0
        for (wrapper, control) in zip(node.children, controls) {
            guard names.contains(wrapper.label), shown.insert(wrapper.label).inserted,
                  control.label == wrapper.label, let checked = control.checked
            else { return false }
            if checked { checkedCount += 1 }
        }
        return shown == names && checkedCount == 1
    }

    private static func hasVideoState(_ label: String) -> Bool {
        let states: Set<String> = ["video is on", "video is off", "video is ingeschakeld", "video is uitgeschakeld", "video staat aan", "video staat uit", "video is aan", "video is uit"]
        return label.lowercased().components(separatedBy: ", ").dropFirst().contains { states.contains($0) }
    }

    private static func hasTileState<Node: ParticipantNode>(_ node: Node) -> Bool {
        if hasVideoState(node.label) { return true }
        // Personal/free Teams omits video state on camera-off menu items. A context menu alone
        // also appears in chat, so require the measured video-occlusion marker within the tile.
        guard node.role == "AXMenuItem", node.label.hasSuffix(", Context menu is available") else { return false }
        var remaining = 32
        func containsVideoMarker(_ candidate: Node, depth: Int) -> Bool {
            guard remaining > 0, depth <= 3 else { return false }
            remaining -= 1
            if candidate.domClasses.contains("vdi-occlusion") || candidate.domClasses.contains(speakingClass) { return true }
            guard candidate.childrenReadable else { return false }
            return candidate.children.filter { $0.role == "AXGroup" }.contains { containsVideoMarker($0, depth: depth + 1) }
        }
        return containsVideoMarker(node, depth: 0)
    }

    static func snapshot<Node: ParticipantNode>(windows: [Node]) -> MeetingSpeakerSnapshot {
        scan(windows).snapshot
    }

    /// Also returns the elements that directly hold tiles, so later samples can skip the rest of the window.
    static func scan<Node: ParticipantNode>(_ roots: [Node]) -> (snapshot: MeetingSpeakerSnapshot, containers: [Node]) {
        var result = Collected<Node>()
        for root in roots {
            if !result.add(tile(root)) { collect(root, depth: 0, into: &result) }
        }
        let snapshot = MeetingSpeakerSnapshot(available: !result.participants.isEmpty, participants: result.participants, speaking: result.speaking, duplicateNames: result.duplicateNames)
        return (snapshot, result.containers)
    }

    private struct Collected<Node> {
        var participants: [String] = []
        var speaking: [String] = []
        var containers: [Node] = []
        var duplicateNames = false

        mutating func add(_ tile: (name: String, speaking: Bool)?) -> Bool {
            guard let tile else { return false }
            if participants.contains(where: { $0.caseInsensitiveCompare(tile.name) == .orderedSame }) { duplicateNames = true } else { participants.append(tile.name) }
            if tile.speaking, !speaking.contains(tile.name) { speaking.append(tile.name) }
            return true
        }
    }

    private static func collect<Node: ParticipantNode>(_ node: Node, depth: Int, into result: inout Collected<Node>) {
        guard depth < maxDepth, !leafRoles.contains(node.role) else { return }
        var holdsTiles = false
        for child in node.children {
            if result.add(tile(child)) {
                holdsTiles = true
            } else {
                collect(child, depth: depth + 1, into: &result)
            }
        }
        if holdsTiles { result.containers.append(node) }
    }

    static func tile<Node: ParticipantNode>(_ node: Node) -> (name: String, speaking: Bool)? {
        guard tileRoles.contains(node.role) else { return nil }
        let label = node.label
        guard hasTileState(node), !selfLabelPrefixes.contains(where: { label.lowercased().hasPrefix($0) }) else { return nil }
        let shown = firstText(node, depth: 0).flatMap { label.hasPrefix($0) ? $0 : nil }
        let name = (shown ?? label.components(separatedBy: ", ")[0]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return nil }
        let speaking = node.children.contains { $0.domClasses.contains(speakingClass) }
        return (name, speaking)
    }

    private static func firstText<Node: ParticipantNode>(_ node: Node, depth: Int) -> String? {
        if node.role == "AXStaticText", !node.text.isEmpty { return node.text }
        guard depth < 8 else { return nil }
        for child in node.children {
            if let text = firstText(child, depth: depth + 1) { return text }
        }
        return nil
    }
}

/// Each attribute read is a cross-process call, so values are fetched once and kept for the snapshot.
final class AccessibilityParticipantNode: ParticipantNode {
    let element: AXUIElement

    init(_ element: AXUIElement) { self.element = element }

    private func value(_ attribute: String) -> AnyObject? {
        var value: AnyObject?
        return AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success ? value : nil
    }

    lazy var role: String = value(kAXRoleAttribute) as? String ?? ""
    lazy var subrole: String = value(kAXSubroleAttribute) as? String ?? ""
    lazy var label: String = {
        var values: CFArray?
        let names = [kAXTitleAttribute, kAXDescriptionAttribute] as CFArray
        guard AXUIElementCopyMultipleAttributeValues(element, names, AXCopyMultipleAttributeOptions(), &values) == .success,
              let values = values as? [AnyObject], values.count == 2
        else { return "" }
        let title = values[0] as? String ?? ""
        return title.isEmpty ? values[1] as? String ?? "" : title
    }()
    lazy var identifier: String = value(kAXIdentifierAttribute) as? String ?? ""
    lazy var selected: Bool = value(kAXSelectedAttribute) as? Bool ?? false
    lazy var checked: Bool? = {
        guard role == "AXCheckBox", let state = value(kAXValueAttribute) as? NSNumber,
              state.doubleValue == 0 || state.doubleValue == 1
        else { return nil }
        return state.doubleValue == 1
    }()
    lazy var modal: Bool? = value(kAXModalAttribute) as? Bool
    lazy var size: CGSize? = {
        guard let raw = value(kAXSizeAttribute), CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
        var size = CGSize.zero
        guard AXValueGetValue(unsafeBitCast(raw, to: AXValue.self), .cgSize, &size) else { return nil }
        return size
    }()
    lazy var text: String = value(kAXValueAttribute) as? String ?? ""
    lazy var domClasses: [String] = value("AXDOMClassList") as? [String] ?? []
    private lazy var childReading: (nodes: [AccessibilityParticipantNode], readable: Bool) = {
        var value: AnyObject?
        let result = AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value)
        if role == "AXButton", result == .noValue || result == .attributeUnsupported {
            // A native leaf may omit AXChildren. Prove that omission from the complete
            // attribute list; an unavailable read or a declared-but-unreadable child list fails.
            var names: CFArray?
            if AXUIElementCopyAttributeNames(element, &names) == .success,
               let names = names as? [String], !names.contains(kAXChildrenAttribute) { return ([], true) }
        }
        guard result == .success, let children = value as? [AXUIElement] else { return ([], false) }
        return (children.map(AccessibilityParticipantNode.init), true)
    }()
    var children: [AccessibilityParticipantNode] { childReading.nodes }
    var childrenReadable: Bool { childReading.readable }

}

/// Samples the visible participant state of a supported meeting app. Reads only tile names and the
/// speaking outline; no names or content are retained between snapshots.
enum MeetingSpeakerObserver {
    static let teamsBundleId = "com.microsoft.teams2"
    private static let lock = NSLock()
    nonisolated(unsafe) private static var exposedProcesses = Set<String>()
    private static let fullScanInterval: TimeInterval = 10
    private struct ContainerCache {
        let pid: pid_t
        let processStartedAtMs: UInt64
        let window: AXUIElement
        let windowNumber: UInt32
        let windowId: String
        let containers: [AXUIElement]
        let refreshAfter: Date
    }
    nonisolated(unsafe) private static var containerCache: ContainerCache?

    private static func unavailable(_ reason: String) -> MeetingSpeakerSnapshot {
        // A later readable window must establish a fresh association rather than reuse stale tiles.
        lock.withLock { containerCache = nil }
        return MeetingSpeakerSnapshot(available: false, participants: [], speaking: [], unavailableReason: reason)
    }

    static func snapshot(appId: String) -> MeetingSpeakerSnapshot {
        guard appId == teamsBundleId else { return unavailable("unsupported") }
        guard AXIsProcessTrusted() else { return unavailable("permission_denied") }
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: appId)
        guard apps.count == 1, let app = apps.first, let launched = app.launchDate else {
            return unavailable(apps.count > 1 ? "ambiguous_process" : "app_unavailable")
        }
        guard !app.isHidden else { return unavailable("window_hidden") }
        let pid = app.processIdentifier
        guard pid > 0 else { return unavailable("app_unavailable") }
        let processStartedAtMs = UInt64(max(0, launched.timeIntervalSince1970 * 1_000))
        let root = AXUIElementCreateApplication(pid)
        exposeWebContent(root, processKey: "\(pid):\(processStartedAtMs)")

        // Selecting one focused window avoids combining tiles from separate calls or a compact view.
        var focusedValue: AnyObject?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedWindowAttribute as CFString, &focusedValue) == .success,
              let focusedValue, CFGetTypeID(focusedValue) == AXUIElementGetTypeID()
        else { return unavailable("window_unavailable") }
        let window = unsafeBitCast(focusedValue, to: AXUIElement.self)
        var minimizedValue: AnyObject?
        guard AXUIElementCopyAttributeValue(window, kAXMinimizedAttribute as CFString, &minimizedValue) == .success,
              let minimized = minimizedValue as? Bool, !minimized
        else { return unavailable("window_hidden") }
        guard let bounds = windowBounds(window),
              let descriptions = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]],
              let windowNumber = visibleWindowNumber(pid: pid, bounds: bounds, windows: descriptions)
        else { return unavailable("window_not_visible") }

        // Global system audio can contain a second call in this same process. Enumerate every
        // top-level window, including covered/minimized ones, before accepting the focused call.
        var windowsValue: AnyObject?
        guard AXUIElementCopyAttributeValue(root, kAXWindowsAttribute as CFString, &windowsValue) == .success,
              let windows = windowsValue as? [AXUIElement], !windows.isEmpty,
              windows.contains(where: { CFEqual($0, window) })
        else { return unavailable("ambiguous_call") }
        let states = windows.map { candidate in
            var minimizedValue: AnyObject?
            let readableMinimized = AXUIElementCopyAttributeValue(candidate, kAXMinimizedAttribute as CFString, &minimizedValue) == .success
            let minimized = readableMinimized ? minimizedValue as? Bool : nil
            return TeamsCallWindowState(
                kind: minimized == nil ? .unreadable : TeamsParticipantParser.callWindowKind(AccessibilityParticipantNode(candidate)),
                focused: CFEqual(candidate, window),
                minimized: minimized ?? true,
                visible: windowBounds(candidate).flatMap { visibleWindowNumber(pid: pid, bounds: $0, windows: descriptions) } != nil
            )
        }
        if let reason = callAssociationReason(states) { return unavailable(reason) }

        let previous = lock.withLock { () -> ContainerCache? in
            guard let cache = containerCache, cache.pid == pid,
                  cache.processStartedAtMs == processStartedAtMs,
                  cache.windowNumber == windowNumber, CFEqual(cache.window, window)
            else { return nil }
            return cache
        }
        let windowId = previous?.windowId ?? UUID().uuidString
        var scan: (snapshot: MeetingSpeakerSnapshot, containers: [AccessibilityParticipantNode])
        if let previous, Date() < previous.refreshAfter, !previous.containers.isEmpty {
            scan = TeamsParticipantParser.scan(previous.containers.map(AccessibilityParticipantNode.init))
            if !scan.snapshot.available { scan = TeamsParticipantParser.scan([AccessibilityParticipantNode(window)]) }
        } else {
            scan = TeamsParticipantParser.scan([AccessibilityParticipantNode(window)])
        }
        guard scan.snapshot.available else { return unavailable("layout_unavailable") }
        lock.withLock {
            containerCache = ContainerCache(
                pid: pid, processStartedAtMs: processStartedAtMs, window: window,
                windowNumber: windowNumber, windowId: windowId,
                containers: scan.containers.map(\.element),
                refreshAfter: previous.flatMap { Date() < $0.refreshAfter ? $0.refreshAfter : nil }
                    ?? Date().addingTimeInterval(fullScanInterval)
            )
        }
        scan.snapshot.processId = UInt32(pid)
        scan.snapshot.processStartedAtMs = processStartedAtMs
        scan.snapshot.windowId = windowId
        return scan.snapshot
    }

    static func callAssociationReason(_ windows: [TeamsCallWindowState]) -> String? {
        let calls = windows.filter { $0.kind == .call }
        guard !windows.contains(where: { $0.kind == .unreadable }), calls.count == 1,
              let call = calls.first, call.focused
        else { return "ambiguous_call" }
        return call.minimized || !call.visible ? "window_not_visible" : nil
    }

    /// Only PID, visibility and geometry are inspected; titles and unrelated window content are ignored.
    static func visibleWindowNumber(pid: pid_t, bounds: CGRect, windows: [[String: Any]]) -> UInt32? {
        let matches: [UInt32] = windows.compactMap { window in
            guard (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
                  (window[kCGWindowIsOnscreen as String] as? Bool) == true,
                  (window[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
                  ((window[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0) > 0,
                  let rect = window[kCGWindowBounds as String] as? [String: Any],
                  let x = rect["X"] as? Double, let y = rect["Y"] as? Double,
                  let width = rect["Width"] as? Double, let height = rect["Height"] as? Double,
                  abs(x - bounds.minX) < 2, abs(y - bounds.minY) < 2,
                  abs(width - bounds.width) < 2, abs(height - bounds.height) < 2,
                  let number = window[kCGWindowNumber as String] as? NSNumber
            else { return nil }
            return number.uint32Value
        }
        guard matches.count == 1, let match = matches.first,
              let index = windows.firstIndex(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == match })
        else { return nil }
        // On-screen metadata includes fully covered windows. A foreground opaque window covering
        // this entire rectangle makes the participant state unavailable without taking screenshots.
        for window in windows[..<index] {
            guard (window[kCGWindowIsOnscreen as String] as? Bool) == true,
                  (window[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
                  ((window[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0) >= 0.95,
                  let rect = window[kCGWindowBounds as String] as? [String: Any],
                  let x = rect["X"] as? Double, let y = rect["Y"] as? Double,
                  let width = rect["Width"] as? Double, let height = rect["Height"] as? Double
            else { continue }
            if CGRect(x: x, y: y, width: width, height: height).contains(bounds) { return nil }
        }
        return match
    }

    private static func windowBounds(_ window: AXUIElement) -> CGRect? {
        var position: AnyObject?
        var size: AnyObject?
        guard AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &position) == .success,
              AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &size) == .success,
              let position, let size,
              CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID()
        else { return nil }
        var point = CGPoint.zero
        var dimensions = CGSize.zero
        guard AXValueGetValue(unsafeBitCast(position, to: AXValue.self), .cgPoint, &point),
              AXValueGetValue(unsafeBitCast(size, to: AXValue.self), .cgSize, &dimensions)
        else { return nil }
        return CGRect(origin: point, size: dimensions)
    }

    static func merge(_ a: MeetingSpeakerSnapshot, _ b: MeetingSpeakerSnapshot) -> MeetingSpeakerSnapshot {
        func union(_ x: [String], _ y: [String]) -> [String] { x + y.filter { !x.contains($0) } }
        return MeetingSpeakerSnapshot(
            available: a.available || b.available,
            participants: union(a.participants, b.participants),
            speaking: union(a.speaking, b.speaking),
            duplicateNames: a.duplicateNames || b.duplicateNames
        )
    }

    private static func exposeWebContent(_ root: AXUIElement, processKey: String) {
        let isNew = lock.withLock { exposedProcesses.insert(processKey).inserted }
        if isNew { AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue) }
    }
}
