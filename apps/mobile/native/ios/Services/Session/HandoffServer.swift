import Foundation
import os
import PennelMobileKit

/// The app side of the keyboard hand-off: reads `command.json` when the keyboard signals, writes `session.json` and
/// `result.json` and signals back. Files hold no audio; `result.json` holds the dictated text for the keyboard only.
@MainActor
final class HandoffServer {
    private let store: HandoffStore
    private let observesSignals: Bool
    private var observation: HandoffSignal.Observation?
    private var handledCommand: UUID?

    private static let log = Logger(subsystem: "app.pennel.ios", category: "handoff")

    init(store: HandoffStore, observesSignals: Bool = true) {
        self.store = store
        self.observesSignals = observesSignals
    }

    func startObserving(_ onCommand: @escaping @MainActor @Sendable () -> Void) {
        guard observesSignals, observation == nil else { return }
        observation = HandoffSignal.command.observe {
            Task { @MainActor in onCommand() }
        }
    }

    func stopObserving() {
        observation = nil
    }

    /// A command left over from an earlier session must not start a dictation in a new one.
    func ignorePendingCommand() {
        handledCommand = store.read(HandoffCommand.self, as: .command)?.id
    }

    /// Each command is handed out once; Darwin notifications can arrive more than once per write.
    func takeCommand() -> HandoffCommand? {
        guard let command = store.read(HandoffCommand.self, as: .command), command.id != handledCommand else { return nil }
        handledCommand = command.id
        return command
    }

    func publish(_ state: SessionState) {
        do {
            try store.write(state, as: .session)
        } catch {
            Self.log.error("session.json not written: \(ErrorText.code(error), privacy: .public)")
        }
        if !AppPaths.isQAProfile { HandoffSignal.session.post() }
    }

    /// Spec §5.2: dictated text leaves the App Group once inserted or after `HandoffClient.resultLifetime`.
    func purgeExpiredResult(now: Date) {
        store.purgeExpiredResult(now: now, lifetime: HandoffClient.resultLifetime)
    }

    func publish(_ result: HandoffResult) {
        do {
            try store.write(result, as: .result)
        } catch {
            Self.log.error("result.json not written: \(ErrorText.code(error), privacy: .public)")
        }
        if !AppPaths.isQAProfile { HandoffSignal.result.post() }
    }
}
