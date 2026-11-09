import AppKit
import ApplicationServices

/// The part of an accessibility element the participant reader needs, so the parser can run on fixtures.
protocol ParticipantNode {
    var role: String { get }
    var label: String { get }
    var text: String { get }
    var domClasses: [String] { get }
    var children: [Self] { get }
}

struct MeetingSpeakerSnapshot: Equatable {
    /// False when no participant tile was readable, e.g. the window is minimized or not a meeting.
    var available: Bool
    var participants: [String]
    var speaking: [String]
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
    static let selfLabelPrefixes = ["Myself video"]
    private static let tileRoles: Set<String> = ["AXMenuItem", "AXImage"]
    private static let leafRoles: Set<String> = ["AXStaticText", "AXButton", "AXTextField", "AXTextArea", "AXLink"]
    private static let maxDepth = 48

    static func snapshot<Node: ParticipantNode>(windows: [Node]) -> MeetingSpeakerSnapshot {
        scan(windows).snapshot
    }

    /// Also returns the elements that directly hold tiles, so later samples can skip the rest of the window.
    static func scan<Node: ParticipantNode>(_ roots: [Node]) -> (snapshot: MeetingSpeakerSnapshot, containers: [Node]) {
        var result = Collected<Node>()
        for root in roots {
            if !result.add(tile(root)) { collect(root, depth: 0, into: &result) }
        }
        let snapshot = MeetingSpeakerSnapshot(available: !result.participants.isEmpty, participants: result.participants, speaking: result.speaking)
        return (snapshot, result.containers)
    }

    private struct Collected<Node> {
        var participants: [String] = []
        var speaking: [String] = []
        var containers: [Node] = []

        mutating func add(_ tile: (name: String, speaking: Bool)?) -> Bool {
            guard let tile else { return false }
            if !participants.contains(tile.name) { participants.append(tile.name) }
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
        guard label.contains(", "), !selfLabelPrefixes.contains(where: { label.hasPrefix($0) }) else { return nil }
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

    private lazy var identity: (role: String, label: String) = {
        var values: CFArray?
        let names = [kAXRoleAttribute, kAXTitleAttribute, kAXDescriptionAttribute] as CFArray
        guard AXUIElementCopyMultipleAttributeValues(element, names, AXCopyMultipleAttributeOptions(), &values) == .success,
              let values = values as? [AnyObject], values.count == 3
        else { return ("", "") }
        let title = values[1] as? String ?? ""
        return (values[0] as? String ?? "", title.isEmpty ? values[2] as? String ?? "" : title)
    }()

    var role: String { identity.role }
    var label: String { identity.label }
    lazy var text: String = value(kAXValueAttribute) as? String ?? ""
    lazy var domClasses: [String] = value("AXDOMClassList") as? [String] ?? []
    lazy var children: [AccessibilityParticipantNode] =
        (value(kAXChildrenAttribute) as? [AXUIElement] ?? []).map(AccessibilityParticipantNode.init)
}

/// Samples the visible participant state of a supported meeting app. Reads only tile names and the
/// speaking outline; no names or content are retained between snapshots.
enum MeetingSpeakerObserver {
    static let teamsBundleId = "com.microsoft.teams2"
    private static let unavailable = MeetingSpeakerSnapshot(available: false, participants: [], speaking: [])
    private static let lock = NSLock()
    nonisolated(unsafe) private static var exposedProcesses = Set<pid_t>()
    /// A full window scan reads thousands of elements; tile containers are re-read until the next full scan,
    /// which picks up joins, leaves and layout changes.
    private static let fullScanInterval: TimeInterval = 10
    private struct ContainerCache {
        let pid: pid_t
        let containers: [AXUIElement]
        let refreshAfter: Date
    }
    nonisolated(unsafe) private static var containerCache: ContainerCache?

    static func snapshot(appId: String) -> MeetingSpeakerSnapshot {
        guard appId == teamsBundleId, AXIsProcessTrusted(),
              let app = NSRunningApplication.runningApplications(withBundleIdentifier: appId).first
        else { return unavailable }
        let pid = app.processIdentifier
        let root = AXUIElementCreateApplication(pid)
        exposeWebContent(root, pid: pid)

        let cached = lock.withLock { () -> [AXUIElement] in
            guard let cache = containerCache, cache.pid == pid, Date() < cache.refreshAfter else { return [] }
            return cache.containers
        }
        if !cached.isEmpty {
            let snapshot = TeamsParticipantParser.scan(cached.map { AccessibilityParticipantNode($0) }).snapshot
            if snapshot.available { return snapshot }
        }

        var value: AnyObject?
        guard AXUIElementCopyAttributeValue(root, kAXWindowsAttribute as CFString, &value) == .success,
              let windows = value as? [AXUIElement]
        else { return unavailable }
        let scan = TeamsParticipantParser.scan(windows.map { AccessibilityParticipantNode($0) })
        lock.withLock {
            containerCache = scan.snapshot.available
                ? ContainerCache(pid: pid, containers: scan.containers.map(\.element), refreshAfter: Date().addingTimeInterval(fullScanInterval))
                : nil
        }
        return scan.snapshot
    }

    static func merge(_ a: MeetingSpeakerSnapshot, _ b: MeetingSpeakerSnapshot) -> MeetingSpeakerSnapshot {
        func union(_ x: [String], _ y: [String]) -> [String] { x + y.filter { !x.contains($0) } }
        return MeetingSpeakerSnapshot(
            available: a.available || b.available,
            participants: union(a.participants, b.participants),
            speaking: union(a.speaking, b.speaking)
        )
    }

    /// Chromium-based apps build their web accessibility tree only once an assistive client asks for it.
    private static func exposeWebContent(_ root: AXUIElement, pid: pid_t) {
        let isNew = lock.withLock { exposedProcesses.insert(pid).inserted }
        if isNew { AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue) }
    }
}
