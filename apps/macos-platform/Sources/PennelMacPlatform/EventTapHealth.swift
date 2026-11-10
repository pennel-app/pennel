import CoreGraphics
import Foundation

struct EventTapHealth {
    static let requiredEvents: CGEventMask = {
        let events: [CGEventType] = [
            .flagsChanged, .keyDown, .keyUp,
            .leftMouseDown, .rightMouseDown, .otherMouseDown,
        ]
        var mask: CGEventMask = 0
        for event in events {
            mask |= CGEventMask(1) << event.rawValue
        }
        return mask
    }()

    let authorized: Bool
    let valid: Bool
    let enabled: Bool
    let events: CGEventMask?

    var isAvailable: Bool {
        guard authorized, valid, enabled, let events else { return false }
        return events & Self.requiredEvents == Self.requiredEvents
    }
}

enum EventTapRecovery {
    static func restore(
        isAvailable: () -> Bool, authorized: () -> Bool,
        remove: () -> Void, create: () -> Bool
    ) -> Bool {
        if isAvailable() { return true }
        remove()
        guard authorized() else { return false }
        guard create(), isAvailable() else { remove(); return false }
        return true
    }
}

enum EventTapInspection {
    static func list() -> [CGEventTapInformation]? {
        var count: UInt32 = 0
        guard CGGetEventTapList(0, nil, &count) == .success else { return nil }
        guard count > 0 else { return [] }
        var entries = [CGEventTapInformation](repeating: CGEventTapInformation(), count: Int(count))
        let capacity = count
        guard CGGetEventTapList(capacity, &entries, &count) == .success else { return nil }
        return Array(entries.prefix(min(Int(capacity), Int(count))))
    }

    // Creating a port can succeed with keyboard events stripped from its mask.
    // Track this port's new ID rather than accepting another tap in the same process.
    static func newTapID(previous: Set<UInt32>, current: [CGEventTapInformation], process: pid_t) -> UInt32? {
        let added = current.filter {
            !previous.contains($0.eventTapID) && $0.tappingProcess == process &&
                $0.tapPoint == .cgSessionEventTap && $0.options == .listenOnly
        }
        return added.count == 1 ? added[0].eventTapID : nil
    }
}
