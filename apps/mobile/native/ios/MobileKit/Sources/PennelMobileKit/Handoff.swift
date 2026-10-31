import CoreFoundation
import Foundation

public enum SessionPhase: String, Codable, Sendable {
    case off, ready, listening, processing
}

public struct SessionState: Codable, Equatable, Sendable {
    public var phase: SessionPhase
    public var expiresAt: Date?
    public var heartbeatAt: Date
    public var level: Float
    public var partialText: String?
    public var previewRequestID: UUID?

    private enum CodingKeys: String, CodingKey {
        case phase, expiresAt, heartbeatAt, level, partialText
        case previewRequestID = "previewRequestId"
    }

    public init(phase: SessionPhase, expiresAt: Date?, heartbeatAt: Date, level: Float, partialText: String? = nil, previewRequestID: UUID? = nil) {
        self.phase = phase
        self.expiresAt = expiresAt
        self.heartbeatAt = heartbeatAt
        self.level = level
        self.partialText = partialText
        self.previewRequestID = previewRequestID
    }
}

public struct HandoffCommand: Codable, Equatable, Sendable {
    public enum Action: String, Codable, Sendable {
        case start, stop, cancel
    }

    public var id: UUID
    public var action: Action
    public var at: Date

    public init(id: UUID, action: Action, at: Date) {
        self.id = id
        self.action = action
        self.at = at
    }
}

public struct HandoffResult: Codable, Equatable, Sendable {
    public var id: UUID
    public var text: String?
    public var error: String?
    public var errorCode: String?
    public var at: Date

    public init(id: UUID, text: String?, error: String?, errorCode: String? = nil, at: Date) {
        self.id = id
        self.text = text
        self.error = error
        self.errorCode = errorCode
        self.at = at
    }
}

public enum HandoffSignal: String, Sendable {
    case command = "app.pennel.handoff.command"
    case session = "app.pennel.handoff.session"
    case result = "app.pennel.handoff.result"

    public func notificationName(namespace: String = HandoffConfiguration.current.namespace) -> String {
        namespace + "." + (rawValue.split(separator: ".").last.map(String.init) ?? rawValue)
    }

    public func post() {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(notificationName() as CFString), nil, nil, true)
    }

    /// Notifications carry no content; the handler only means "read the file again". Keep the returned token alive to keep observing.
    public func observe(_ handler: @escaping @Sendable () -> Void) -> Observation {
        Observation(name: notificationName(), handler: handler)
    }

    public final class Observation: @unchecked Sendable {
        private let name: String
        private let box: Unmanaged<HandlerBox>

        fileprivate init(name: String, handler: @escaping @Sendable () -> Void) {
            self.name = name
            box = Unmanaged.passRetained(HandlerBox(handler))
            CFNotificationCenterAddObserver(
                CFNotificationCenterGetDarwinNotifyCenter(), box.toOpaque(),
                { _, observer, _, _, _ in
                    guard let observer else { return }
                    Unmanaged<HandlerBox>.fromOpaque(observer).takeUnretainedValue().handler()
                },
                name as CFString, nil, .deliverImmediately)
        }

        deinit {
            CFNotificationCenterRemoveObserver(
                CFNotificationCenterGetDarwinNotifyCenter(), box.toOpaque(), CFNotificationName(name as CFString), nil)
            box.release()
        }
    }

    private final class HandlerBox: @unchecked Sendable {
        let handler: @Sendable () -> Void
        init(_ handler: @escaping @Sendable () -> Void) { self.handler = handler }
    }
}

/// The id of the command a keyboard is waiting on. Re-created clients in one process share a slot per handoff
/// directory, so a keyboard dismissed and shown again resumes the same dictation; a different slot never
/// matches another instance's id.
public final class PendingHandoff: @unchecked Sendable {
    private let lock = NSLock()
    private var id: UUID?
    private var since: Date?
    private var consumed: [UUID] = []

    public init() {}

    private static let registryLock = NSLock()
    nonisolated(unsafe) private static var registry: [String: PendingHandoff] = [:]

    static func shared(for directory: URL) -> PendingHandoff {
        registryLock.withLock {
            let key = directory.standardizedFileURL.path
            if let existing = registry[key] { return existing }
            let created = PendingHandoff()
            registry[key] = created
            return created
        }
    }

    var current: UUID? { lock.withLock { id } }

    /// The pending id, unless it was set longer than `lifetime` ago: a keyboard shown again much later must not
    /// auto-insert an old dictation. An expired id is dropped.
    func current(now: Date, lifetime: TimeInterval) -> UUID? {
        lock.withLock {
            if let since, now.timeIntervalSince(since) > lifetime {
                id = nil
                self.since = nil
            }
            return id
        }
    }

    func set(_ new: UUID, at date: Date = Date()) {
        lock.withLock {
            id = new
            since = date
        }
    }

    /// Clears only if `expected` is still the pending id, so a late consumer cannot drop a newer command.
    func clear(ifMatching expected: UUID) {
        lock.withLock {
            if id == expected {
                id = nil
                since = nil
            }
        }
    }

    func consume(_ value: UUID) -> Bool {
        lock.withLock {
            guard !consumed.contains(value) else { return false }
            consumed.append(value)
            if consumed.count > 128 { consumed.removeFirst(consumed.count - 128) }
            return true
        }
    }

    func wasConsumed(_ value: UUID) -> Bool { lock.withLock { consumed.contains(value) } }
}

public struct HandoffClient: Sendable {
    public static let heartbeatTimeout: TimeInterval = 5
    public static let resultLifetime: TimeInterval = 10 * 60

    private let store: HandoffStore
    private let pending: PendingHandoff

    public init(store: HandoffStore, pending: PendingHandoff? = nil) {
        self.store = store
        self.pending = pending ?? PendingHandoff.shared(for: store.directory)
    }

    @discardableResult
    public func send(_ action: HandoffCommand.Action, now: Date = Date()) throws -> UUID {
        let id = UUID()
        if action == .cancel, let previous = pending.current { _ = pending.consume(previous) }
        pending.set(id, at: now)
        do {
            try store.write(HandoffCommand(id: id, action: action, at: now), as: .command)
        } catch {
            pending.clear(ifMatching: id)
            throw error
        }
        HandoffSignal.command.post()
        return id
    }

    public func session(now: Date = Date()) -> SessionPhase {
        guard let state = store.read(SessionState.self, as: .session) else { return .off }
        if now.timeIntervalSince(state.heartbeatAt) > Self.heartbeatTimeout { return .off }
        if let expiresAt = state.expiresAt, expiresAt <= now { return .off }
        return state.phase
    }

    public func level() -> Float {
        store.read(SessionState.self, as: .session)?.level ?? 0
    }

    public func partialText(for requestID: UUID, now: Date = Date()) -> String? {
        guard let state = store.read(SessionState.self, as: .session), state.phase == .listening,
              (-2...Self.heartbeatTimeout).contains(now.timeIntervalSince(state.heartbeatAt)),
              state.expiresAt.map({ $0 > now }) != false,
              state.previewRequestID == requestID, let text = state.partialText,
              !text.isEmpty, text.count <= 2_048 else { return nil }
        return text
    }

    /// Results older than `resultLifetime` are never inserted; they are removed and the pending id is dropped.
    public func takeResult(now: Date = Date()) -> HandoffResult? {
        guard let result = pendingResult(now: now), markInserted(result) else { return nil }
        return result
    }

    public func pendingResult(now: Date = Date()) -> HandoffResult? {
        store.purgeExpiredResult(now: now, lifetime: Self.resultLifetime)
        guard let id = pending.current(now: now, lifetime: Self.resultLifetime),
              let result = store.read(HandoffResult.self, as: .result), result.id == id,
              !pending.wasConsumed(id)
        else { return nil }
        return result
    }

    /// A changed keyboard context leaves the result for an explicit recovery action.
    public func deferPendingResult(_ result: HandoffResult) {
        pending.clear(ifMatching: result.id)
    }

    /// For the explicit "Laatste dictaat invoegen" tap after the keyboard was away. Call `markInserted` once it is typed.
    public func lastUninserted(now: Date = Date()) -> HandoffResult? {
        guard let result = store.read(HandoffResult.self, as: .result),
              result.text != nil,
              now.timeIntervalSince(result.at) <= Self.resultLifetime
        else { return nil }
        if pending.wasConsumed(result.id) {
            store.clearResult(id: result.id)
            return nil
        }
        return result
    }

    @discardableResult
    public func markInserted(_ result: HandoffResult) -> Bool {
        guard pending.consume(result.id) else { return false }
        pending.clear(ifMatching: result.id)
        store.clearResult(id: result.id)
        return true
    }
}
