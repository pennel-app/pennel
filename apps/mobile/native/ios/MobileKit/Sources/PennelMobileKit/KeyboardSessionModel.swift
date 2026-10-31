import Foundation
import Observation

/// What the dictation pad shows. Spec 4.5 lists one row per case.
public enum KeyboardPad: Equatable {
    case needsFullAccess, sessionOff, ready, listening, processing
    case failed(String)
}

/// UIKit-free state shared by the extension and Swift Package tests. The handoff carries commands and
/// explicit dictation results; unrelated typed text never enters this model.
@MainActor
@Observable
public final class KeyboardSessionModel {
    /// Commands are fire and forget; if the app has not reflected a start by then, it never acted on it.
    public static let acknowledgementGrace: TimeInterval = 1.5
    public static let resultGrace: TimeInterval = 3

    public private(set) var pad: KeyboardPad
    public private(set) var lastResult: HandoffResult?
    public private(set) var level: Float = 0
    public private(set) var partialText: String?

    private enum Gesture { case none, hold, handsFree }

    private let hasFullAccess: Bool
    private let client: HandoffClient
    private let now: () -> Date
    private let context: () -> KeyboardInsertionContext?
    private let onResult: (String) -> Void
    private let onFeedback: (FeedbackEvent) -> Void
    public let copy: KeyboardCopy

    private var gesture = Gesture.none
    private var startedAt: Date?
    private var activeStartID: UUID?
    private var stoppedAt: Date?
    private var failure: String?
    public private(set) var recoveryAction: KeyboardFailureRecovery = .recordAgain
    private var delivery = KeyboardDeliveryGuard()
    private var awaitingStartFeedback = false
    private var awaitingStopFeedback = false
    private var awaitingCancelFeedback = false
    private var observedSession: SessionPhase = .off

    public init(
        hasFullAccess: Bool, client: HandoffClient, now: @escaping () -> Date = Date.init,
        context: @escaping () -> KeyboardInsertionContext?,
        copy: KeyboardCopy = KeyboardCopy(language: .english),
        onFeedback: @escaping (FeedbackEvent) -> Void = { _ in }, onResult: @escaping (String) -> Void
    ) {
        self.hasFullAccess = hasFullAccess
        self.client = client
        self.now = now
        self.context = context
        self.copy = copy
        self.onResult = onResult
        self.onFeedback = onFeedback
        pad = hasFullAccess ? .sessionOff : .needsFullAccess
    }

    public var primaryActionLabel: String {
        switch pad {
        case .ready: copy.text(.ready)
        case .listening: copy.text(.stopDictation)
        case .processing: copy.text(.processing)
        case .sessionOff, .needsFullAccess: copy.text(.setupDictation)
        case .failed: copy.text(recoveryAction == .recordAgain ? .recordAgain : .checkAgain)
        }
    }

    public var primaryActionEnabled: Bool {
        switch pad {
        case .ready, .listening, .failed: true
        case .needsFullAccess, .sessionOff, .processing: false
        }
    }

    public var lastResultPreview: String? { lastResult?.text.map(KeyboardResultPreview.text) }

    public func refresh() {
        guard hasFullAccess else {
            partialText = nil
            pad = .needsFullAccess
            return
        }
        let current = now()
        let session = client.session(now: current)
        acknowledgeFeedback(session)
        if let result = client.pendingResult(now: current) {
            if result.text == nil || delivery.permits(result.id, in: context()) {
                if client.markInserted(result) {
                    if awaitingStopFeedback { onFeedback(.recordingStopped) }
                    finish(result)
                }
            } else {
                client.deferPendingResult(result)
                reset()
            }
        }
        level = session == .listening ? client.level() : 0
        if session == .listening, stoppedAt == nil, let activeStartID {
            partialText = client.partialText(for: activeStartID, now: current)
        } else {
            partialText = nil
        }
        lastResult = isIdle ? client.lastUninserted(now: current) : nil

        if let failure {
            pad = .failed(failure)
            return
        }
        switch session {
        case .off:
            reset()
            pad = .sessionOff
        case .listening:
            // The app may still read listening for a moment after our stop; it must not look tappable again.
            if let stoppedAt, current.timeIntervalSince(stoppedAt) < Self.resultGrace {
                pad = .processing
            } else {
                stoppedAt = nil
                pad = .listening
            }
        case .processing:
            pad = .processing
        case .ready:
            if let startedAt, gesture != .none, current.timeIntervalSince(startedAt) < Self.acknowledgementGrace {
                pad = .listening
            } else if let stoppedAt, current.timeIntervalSince(stoppedAt) < Self.resultGrace {
                pad = .processing
            } else {
                reset()
                pad = .ready
            }
        }
    }

    public func tapPill() {
        guard hasFullAccess else { return }
        switch pad {
        case .sessionOff:
            break
        case .ready:
            begin(.handsFree)
        case .listening:
            if gesture != .hold { end() }
        case .failed:
            if recoveryAction == .recordAgain { recordAgain() } else { retry() }
        case .needsFullAccess, .processing:
            break
        }
    }

    public func holdBegan() {
        guard hasFullAccess else { return }
        switch pad {
        case .ready: begin(.hold)
        case .sessionOff: break
        default: break
        }
    }

    public func holdEnded() {
        if pad == .listening, gesture == .hold { end() }
    }

    public func cancel() {
        guard hasFullAccess, pad == .listening || pad == .processing else { return }
        guard send(.cancel) != nil else { return }
        let wasActive = observedSession == .listening || observedSession == .processing
        reset()
        awaitingCancelFeedback = wasActive
        refresh()
    }

    public func retry() {
        failure = nil
        reset()
        refresh()
    }

    public func recordAgain() {
        guard case .failed = pad, recoveryAction == .recordAgain else { return }
        retry()
        if pad == .ready { begin(.handsFree) }
    }

    public func insertLast() {
        guard let result = lastResult, context() != nil else { return }
        lastResult = nil
        if client.markInserted(result), let text = result.text {
            onResult(text)
            onFeedback(.resultDelivered)
        }
    }

    public func contextDidChange() {
        delivery.invalidate()
        if pad == .listening, gesture != .none { end() }
    }

    private var isIdle: Bool { gesture == .none && stoppedAt == nil }

    private func begin(_ new: Gesture) {
        guard let origin = context() else { return }
        delivery.begin(in: origin)
        guard let startID = send(.start) else { return }
        activeStartID = startID
        gesture = new
        startedAt = now()
        stoppedAt = nil
        awaitingStartFeedback = true
        pad = .listening
    }

    private func end() {
        // A second stop would replace the pending id and orphan the first result.
        guard stoppedAt == nil, let id = send(.stop) else { return }
        delivery.awaitResult(id)
        gesture = .none
        stoppedAt = now()
        partialText = nil
        awaitingStopFeedback = true
        pad = .processing
    }

    @discardableResult
    private func send(_ action: HandoffCommand.Action) -> UUID? {
        do {
            return try client.send(action, now: now())
        } catch {
            failure = copy.text(.fullAccess)
            recoveryAction = .checkSettings
            pad = .failed(copy.text(.fullAccess))
            onFeedback(.error)
            return nil
        }
    }

    private func finish(_ result: HandoffResult) {
        reset()
        if let text = result.text {
            onResult(text)
            onFeedback(.resultDelivered)
        } else {
            failure = copy.failure(code: result.errorCode, message: result.error)
            recoveryAction = copy.recoveryAction(code: result.errorCode, message: result.error)
            onFeedback(.error)
        }
    }

    private func acknowledgeFeedback(_ session: SessionPhase) {
        if awaitingStartFeedback, session == .listening {
            awaitingStartFeedback = false
            onFeedback(.recordingStarted)
        }
        if awaitingStopFeedback, session == .processing || (session == .ready && (observedSession == .listening || observedSession == .processing)) {
            awaitingStopFeedback = false
            onFeedback(.recordingStopped)
        }
        if awaitingCancelFeedback, session == .ready || session == .off {
            awaitingCancelFeedback = false
            onFeedback(.recordingCancelled)
        }
        observedSession = session
    }

    private func reset() {
        gesture = .none
        startedAt = nil
        activeStartID = nil
        partialText = nil
        stoppedAt = nil
        delivery.reset()
        awaitingStartFeedback = false
        awaitingStopFeedback = false
        awaitingCancelFeedback = false
    }
}
