import CoreGraphics
import Foundation

/// Installed only while a binding, capture, or an outstanding withheld release needs it.
/// Every decision is immediate; the callback and lifetime are serialized by the main run loop.
final class ShortcutTap: @unchecked Sendable {
    private let onInput: (CGEventType, CGEvent) -> Bool
    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    init(onInput: @escaping (CGEventType, CGEvent) -> Bool) { self.onInput = onInput }
    deinit { stop() }
    var isRunning: Bool { tap.map { CFMachPortIsValid($0) && CGEvent.tapIsEnabled(tap: $0) } ?? false }
    func start() -> Bool {
        if isRunning { return true }
        stop()
        let callback: CGEventTapCallBack = { _, type, event, refcon in
            guard let refcon else { return Unmanaged.passUnretained(event) }
            let owner = Unmanaged<ShortcutTap>.fromOpaque(refcon).takeUnretainedValue()
            if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
                if let tap = owner.tap { CGEvent.tapEnable(tap: tap, enable: true) }
                return Unmanaged.passUnretained(event)
            }
            return owner.onInput(type, event) ? nil : Unmanaged.passUnretained(event)
        }
        let mask = CGEventMask(1 << CGEventType.keyDown.rawValue | 1 << CGEventType.keyUp.rawValue | 1 << CGEventType.flagsChanged.rawValue)
        guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap, eventsOfInterest: mask, callback: callback, userInfo: Unmanaged.passUnretained(self).toOpaque()) else { return false }
        self.tap = tap
        guard let source = CFMachPortCreateRunLoopSource(nil, tap, 0) else { stop(); return false }
        self.source = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        return isRunning
    }
    func stop() {
        if let tap { CGEvent.tapEnable(tap: tap, enable: false); CFMachPortInvalidate(tap) }
        if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        tap = nil; source = nil
    }
}
