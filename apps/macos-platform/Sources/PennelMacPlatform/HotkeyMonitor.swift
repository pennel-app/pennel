import ApplicationServices
import CoreGraphics
import Foundation

/// Listens system-wide for the dictation modifier key via a listen-only event tap.
/// All access, including the timer's Sendable callback, stays on the main run loop.
final class HotkeyMonitor: @unchecked Sendable {
    private let onInput: (CGEventType, CGEvent) -> Void
    private let onReset: () -> Void
    private let onAvailabilityChange: (Bool) -> Void
    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    private var tapID: UInt32?
    private var healthPoll: Timer?
    private var reportedAvailability: Bool?

    init(onInput: @escaping (CGEventType, CGEvent) -> Void, onReset: @escaping () -> Void) {
        self.onInput = onInput
        self.onReset = onReset
        self.onAvailabilityChange = { _ in }
    }

    deinit { stop() }

    var isRunning: Bool {
        guard let tap, let tapID else { return false }
        let events = EventTapInspection.list()?.first { $0.eventTapID == tapID }?.eventsOfInterest
        return EventTapHealth(
            authorized: AXIsProcessTrusted(), valid: CFMachPortIsValid(tap),
            enabled: CGEvent.tapIsEnabled(tap: tap), events: events
        ).isAvailable
    }
    func start() -> Bool {
        let available = EventTapRecovery.restore(
            isAvailable: { self.isRunning }, authorized: { AXIsProcessTrusted() },
            remove: { self.clearTap() }, create: { self.installTap() }
        )
        if available, healthPoll == nil {
            healthPoll = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
                guard let self else { return }
                if !self.isRunning { _ = self.start() }
            }
        }
        reportAvailability(available)
        return available
    }

    private func installTap() -> Bool {
        guard let existing = EventTapInspection.list() else { return false }
        let previous = Set(existing.map(\.eventTapID))
        let callback: CGEventTapCallBack = { _, type, event, refcon in
            guard let refcon else { return Unmanaged.passUnretained(event) }
            let monitor = Unmanaged<HotkeyMonitor>.fromOpaque(refcon).takeUnretainedValue()
            monitor.handle(type: type, event: event)
            return Unmanaged.passUnretained(event)
        }
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .listenOnly,
            eventsOfInterest: EventTapHealth.requiredEvents,
            callback: callback,
            userInfo: Unmanaged.passUnretained(self).toOpaque()
        ) else { return false }
        self.tap = tap
        tapID = EventTapInspection.list().flatMap {
            EventTapInspection.newTapID(previous: previous, current: $0, process: ProcessInfo.processInfo.processIdentifier)
        }
        guard let source = CFMachPortCreateRunLoopSource(nil, tap, 0) else {
            return false
        }
        self.source = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        return true
    }

    func stop() {
        healthPoll?.invalidate()
        healthPoll = nil
        clearTap()
        reportAvailability(false)
    }

    private func clearTap() {
        let wasTracking = tap != nil
        if let tap {
            CGEvent.tapEnable(tap: tap, enable: false)
            CFMachPortInvalidate(tap)
        }
        if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        tap = nil
        source = nil
        tapID = nil
        if wasTracking { onReset() }
    }

    private func reportAvailability(_ available: Bool) {
        guard reportedAvailability != available else { return }
        reportedAvailability = available
        onAvailabilityChange(available)
    }

    private func handle(type: CGEventType, event: CGEvent) {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let tap, AXIsProcessTrusted() { CGEvent.tapEnable(tap: tap, enable: true) }
            return
        }
        onInput(type, event)
    }
}
