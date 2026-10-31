import Foundation
import PennelMobileKit
import UIKit

@MainActor
final class KeyboardWritingServer {
    private let engine: PennelEngine
    private let store: KeyboardWritingStore
    private let handoff: HandoffStore
    private var timer: Timer?
    private var task: Task<Void, Never>?
    private var current: UUID?
    private var job: KeyboardWritingJob?
    private let authorization: @MainActor () -> String
    private var backgroundTask: UIBackgroundTaskIdentifier = .invalid

    init?(engine: PennelEngine, authorization: @escaping @MainActor () -> String = { "" }) {
        guard let directory = HandoffStore.groupDirectory else { return nil }
        self.engine = engine
        self.authorization = authorization
        store = KeyboardWritingStore(directory: directory)
        handoff = HandoffStore(directory: directory)
    }

    func start() {
        guard timer == nil else { return }
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    private func refresh() {
        let now = Date()
        store.purge(at: now)
        let settings = engine.settings()
        let configuration = engine.keyboardWritingConfiguration()
        let identity = authorization()
        let session = handoff.read(SessionState.self, as: .session)
        let ready = Self.canReceive(appIsActive: UIApplication.shared.applicationState == .active, session: session, now: now)
        let account = settings.providers.first { $0.id == settings.llm.providerId }
        let providerReady = MeetingRuntime.languageModelReady(settings) && (account?.kind != .pennelCloud || engine.cloudSignedIn())
        let available = ready && current == nil && providerReady
        let provider = account?.name
        try? store.write(KeyboardWritingAvailability(at: now, available: available, configuration: configuration, providerName: provider, authorization: identity), as: .availability)
        if let current, store.isCancelled(id: current) { job?.cancel(); task?.cancel() }
        guard let request = store.takeRequest(at: now) else { return }
        guard available, request.configuration == configuration, request.authorization == identity,
              let action = Self.action(request.action), current == nil else {
            try? store.write(KeyboardWritingResult(id: request.id, at: now, errorCode: "unavailable"), as: .result)
            return
        }
        current = request.id
        let expired: @Sendable () -> Void = { [weak self] in
            Task { @MainActor in
                self?.job?.cancel()
                self?.task?.cancel()
                self?.endBackgroundTask()
            }
        }
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "Pennel selected-text action", expirationHandler: expired)
        let engine = engine
        let keys = Keychain.apiKeys()
        task = Task { [weak self] in
            let prepared = await Task.detached {
                Result { try engine.prepareKeyboardWriting(action: action, selection: request.text, expectedConfiguration: request.configuration, keys: keys) }
            }.value
            guard let self, self.current == request.id else { return }
            defer { self.current = nil; self.job = nil; self.task = nil; self.endBackgroundTask() }
            guard !Task.isCancelled, !self.store.isCancelled(id: request.id), request.isValid(at: Date()),
                  self.authorization() == request.authorization else {
                if case .success(let job) = prepared { job.cancel() }
                return
            }
            let result: Result<String, Error>
            switch prepared {
            case .failure(let error): result = .failure(error)
            case .success(let job):
                self.job = job
                result = await Task.detached { Result { try job.run() } }.value
            }
            guard !Task.isCancelled, !self.store.isCancelled(id: request.id), request.isValid(at: Date()),
                  engine.keyboardWritingConfiguration() == request.configuration,
                  self.authorization() == request.authorization else { return }
            switch result {
            case .success(let text): try? self.store.write(KeyboardWritingResult(id: request.id, at: Date(), text: text), as: .result)
            case .failure: try? self.store.write(KeyboardWritingResult(id: request.id, at: Date(), errorCode: "failed"), as: .result)
            }
        }
    }

    static func canReceive(appIsActive: Bool, session: SessionState?, now: Date) -> Bool {
        if appIsActive { return true }
        guard let session, session.phase == .ready, let expires = session.expiresAt, expires > now else { return false }
        return (-2...HandoffClient.heartbeatTimeout).contains(now.timeIntervalSince(session.heartbeatAt))
    }

    private func endBackgroundTask() {
        guard backgroundTask != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundTask)
        backgroundTask = .invalid
    }

    private static func action(_ value: String) -> KeyboardRewriteAction? {
        switch value {
        case "cleanup": .cleanup
        case "shorten": .shorten
        case "translateToEnglish": .translateToEnglish
        case "translateToDutch": .translateToDutch
        default: nil
        }
    }
}
