import Foundation
import PennelMobileKit

protocol DictationRunning: Sendable {
    func run(samples: [Float]) async -> Result<DictationResult, PennelError>
    func cancel()
    func startPreview(onText: @escaping @Sendable (String) -> Void)
    func feed(samples: [Float])
    func stopPreview()
    func cleanupVerificationAvailability() -> CleanupVerificationAvailability?
}

extension DictationRunning {
    func cancel() {}
    func startPreview(onText: @escaping @Sendable (String) -> Void) {}
    func feed(samples: [Float]) {}
    func stopPreview() {}
    func cleanupVerificationAvailability() -> CleanupVerificationAvailability? { nil }
}

/// Sends one finished recording through the core's dictation pipeline.
struct DictationRunner: DictationRunning {
    typealias PreviewFactory = @Sendable (String?, [String], @escaping @Sendable (String) -> Void) -> any DictationPreviewing
    static let sampleRate: UInt32 = 16_000

    private let session: LiveDictationSession
    private let keys: @Sendable () -> ApiKeys
    private let active = ActiveDictation()
    private let relay: DictationPreviewRelay
    private let preview = DictationPreviewOwner()
    private let previewFactory: PreviewFactory

    init(engine: PennelEngine, keys: @escaping @Sendable () -> ApiKeys = { Keychain.apiKeys() }, previewFactory: @escaping PreviewFactory = { AppleSpeechPreview(language: $0, vocabulary: $1, onText: $2) }) {
        let relay = DictationPreviewRelay()
        self.relay = relay
        session = engine.beginDictationSession(observer: relay, practice: false)
        self.keys = keys
        self.previewFactory = previewFactory
        active.set(session)
    }

    /// Runs off the main actor: the core blocks on network and on-device models.
    func run(samples: [Float]) async -> Result<DictationResult, PennelError> {
        stopPreview()
        let request = DictationRequest(samples: samples, sampleRate: Self.sampleRate, appBundleId: nil, keys: keys())
        let session = session
        let active = active
        return await Task.detached(priority: .userInitiated) {
            defer { active.clear(session) }
            do {
                return .success(try session.finish(request: request))
            } catch let error as PennelError {
                return .failure(error)
            } catch {
                return .failure(.InvalidResponse(message: error.localizedDescription))
            }
        }.value
    }

    func startPreview(onText: @escaping @Sendable (String) -> Void) {
        guard session.mode() == .system else { return }
        relay.listen(onText)
        let session = session
        preview.start(previewFactory(session.language(), session.vocabulary()) { session.updateSystemPreview(text: $0) })
    }

    func feed(samples: [Float]) { preview.feed(samples) }

    func stopPreview() {
        relay.stop()
        preview.stop()
        session.stopPreview()
    }

    func cancel() { stopPreview(); active.cancel() }
    func cleanupVerificationAvailability() -> CleanupVerificationAvailability? { session.cleanupVerificationAvailability() }
}

private final class DictationPreviewRelay: DictationPreviewObserver, @unchecked Sendable {
    private let lock = NSRecursiveLock()
    private var callback: (@Sendable (String) -> Void)?
    private var stopped = false

    func listen(_ callback: @escaping @Sendable (String) -> Void) {
        lock.withLock { if !stopped { self.callback = callback } }
    }

    func changed(update: DictationPreviewUpdate) { lock.withLock { callback?(update.text) } }
    func stop() { lock.withLock { stopped = true; callback = nil } }
}

private final class DictationPreviewOwner: @unchecked Sendable {
    private let lock = NSLock()
    private var current: (any DictationPreviewing)?
    private var stopped = false

    func start(_ preview: any DictationPreviewing) {
        lock.withLock {
            guard !stopped, current == nil else { preview.stop(); return }
            if preview.start() { current = preview }
            else { preview.stop() }
        }
    }

    func feed(_ samples: [Float]) { lock.withLock { current?.feed(samples) } }

    func stop() {
        lock.withLock { stopped = true; current?.stop(); current = nil }
    }
}

private final class ActiveDictation: @unchecked Sendable {
    private let lock = NSLock()
    private var current: LiveDictationSession?
    private var cancelled = false

    func set(_ session: LiveDictationSession) {
        lock.withLock {
            current = session
            if cancelled { session.cancel() }
        }
    }

    func clear(_ session: LiveDictationSession) {
        lock.withLock {
            if current === session { current = nil }
        }
    }

    func cancel() {
        lock.withLock {
            cancelled = true
            current?.cancel()
        }
    }
}
