import AVFoundation
import Observation
import os
import PennelMobileKit
import UIKit

/// The meeting that is recording right now, if any (Task 9 provides it). While a dictation runs during a meeting,
/// the meeting keeps that interval out of its transcript, like the desktop.
@MainActor
protocol DictationMeetingHook: AnyObject {
    var isActive: Bool { get }
    /// Milliseconds since the meeting started: the timeline `markDictation` is expressed in.
    var elapsedMs: UInt64 { get }
    func setDictating(_ dictating: Bool)
    func markDictation(startMs: UInt64, endMs: UInt64)
}

/// The short background session that records for the keyboard: `off → ready → listening → processing → ready`.
/// While `ready` the microphone runs (iOS cannot start it from the background later) but every sample is dropped.
@MainActor
@Observable
final class DictationSession {
    typealias Timers = @MainActor (TimeInterval, @escaping @MainActor @Sendable () -> Void) -> (() -> Void)

    enum StopReason: String { case user, expired, interrupted, mediaReset, terminated }

    static let shared = DictationSession.live()

    private(set) var phase = SessionPhase.off
    private(set) var expiresAt: Date?
    private(set) var startError: String?
    private(set) var startErrorCode: String?
    var cleanupVerificationAvailability: CleanupVerificationAvailability? { processingRunner?.cleanupVerificationAvailability() }

    @ObservationIgnored private let capture: AudioCapturing
    @ObservationIgnored private let audioSession: AudioSessionControlling
    @ObservationIgnored private let runner: @MainActor () -> DictationRunning
    @ObservationIgnored private let server: HandoffServer
    @ObservationIgnored private let activity: SessionActivityControlling
    @ObservationIgnored private let activityCopy: @MainActor () -> ActivityCopy
    @ObservationIgnored private let duration: @MainActor () -> TimeInterval
    @ObservationIgnored private let now: @MainActor () -> Date
    @ObservationIgnored private let timers: Timers
    @ObservationIgnored private let meeting: @MainActor () -> DictationMeetingHook?
    @ObservationIgnored private let requestPermission: @MainActor () async -> Bool
    @ObservationIgnored private let gate = SampleGate()
    @ObservationIgnored private var clock: SessionClock?
    @ObservationIgnored private var cancelHeartbeat: (() -> Void)?
    @ObservationIgnored private var cancelLevel: (() -> Void)?
    @ObservationIgnored private var meetingDictationStartMs: UInt64?
    /// Bumped on every stop, so a dictation that finishes after the session ended cannot revive it.
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var processingRunner: DictationRunning?
    @ObservationIgnored private var previewRequestID: UUID?
    @ObservationIgnored private var partialText: String?
    @ObservationIgnored private var pendingStart: UUID?
    @ObservationIgnored private var observers: [NSObjectProtocol] = []

    private static let log = Logger(subsystem: "app.pennel.ios", category: "session")

    init(
        capture: AudioCapturing,
        audioSession: AudioSessionControlling,
        runner: @escaping @MainActor () -> DictationRunning,
        store: HandoffStore,
        activity: SessionActivityControlling,
        activityCopy: @escaping @MainActor () -> ActivityCopy = { .current },
        notifications: NotificationCenter = .default,
        duration: TimeInterval = SessionClock.Duration.fiveMinutes.rawValue,
        durationSource: (@MainActor () -> TimeInterval)? = nil,
        now: @escaping @MainActor () -> Date = { Date() },
        timers: @escaping Timers = DictationSession.runLoopTimers,
        meeting: @escaping @MainActor () -> DictationMeetingHook? = { nil },
        observeCommands: Bool = true,
        requestPermission: @escaping @MainActor () async -> Bool = { await MicCapture.requestPermission() }
    ) {
        self.capture = capture
        self.audioSession = audioSession
        self.runner = runner
        server = HandoffServer(store: store, observesSignals: observeCommands)
        self.activity = activity
        self.activityCopy = activityCopy
        self.duration = durationSource ?? { duration }
        self.now = now
        self.timers = timers
        self.meeting = meeting
        self.requestPermission = requestPermission
        observeSystem(notifications)
    }

    private static func live() -> DictationSession {
        DictationSession(
            capture: SharedMicrophone.shared.client(),
            audioSession: SharedAudioSession.shared.client(.keyboardSession),
            runner: { DictationRunner(engine: AppModel.shared.engine) },
            store: HandoffStore(directory: AppPaths.handoffDirectory),
            activity: SystemSessionActivity(),
            durationSource: { SessionClock.Duration.stored().rawValue },
            meeting: { MeetingRuntime.shared })
    }

    /// Nothing runs while the app is suspended, so a result that outlived its lifetime is removed at launch.
    static func purgeExpiredResult() {
        HandoffStore(directory: AppPaths.handoffDirectory)
            .purgeExpiredResult(now: Date(), lifetime: HandoffClient.resultLifetime)
    }

    static func isStartLink(_ url: URL) -> Bool {
        let scheme = Bundle.main.object(forInfoDictionaryKey: "PennelURLScheme") as? String ?? "pennel"
        return url.scheme == scheme && url.host() == "sessie" && url.path() == "/start"
    }

    // MARK: Session

    /// Foreground only: iOS does not let an app start recording from the background.
    @discardableResult
    func start() async -> Bool {
        guard phase == .off else { return true }
        guard pendingStart == nil else { return false }
        let token = UUID()
        pendingStart = token
        defer { if pendingStart == token { pendingStart = nil } }
        startError = nil
        startErrorCode = nil
        let allowed = await requestPermission()
        guard pendingStart == token else { return false }
        guard allowed else {
            startError = "No microphone access. Allow microphone access in Settings."
            startErrorCode = "microphone_denied"
            return false
        }
        guard phase == .off else { return true }
        do {
            try audioSession.activate()
            let gate = gate
            try capture.start { gate.feed($0) }
        } catch {
            Self.log.error("session start: \(ErrorText.code(error), privacy: .public)")
            capture.stop()
            audioSession.deactivate()
            startError = ErrorText.failed(error)
            startErrorCode = ErrorText.failureCode(error)
            return false
        }
        let now = now()
        clock = SessionClock(duration: duration(), now: now)
        phase = .ready
        server.ignorePendingCommand()
        server.startObserving { [weak self] in
            Task { await self?.receiveCommand() }
        }
        publish(at: now)
        activity.start(activityState)
        cancelHeartbeat = timers(1) { [weak self] in self?.tick() }
        Self.log.info("session ready, \(Int(self.clock?.duration ?? 0), privacy: .public)s")
        return true
    }

    /// Closes the microphone for real: capture stopped, audio session deactivated, keyboard told `off`.
    func stop() {
        stop(reason: .user)
    }

    func stop(reason: StopReason) {
        pendingStart = nil
        guard phase != .off else { return }
        processingRunner?.cancel()
        processingRunner = nil
        previewRequestID = nil
        partialText = nil
        _ = gate.close()
        endMeetingDictation()
        cancelLevel?()
        cancelLevel = nil
        cancelHeartbeat?()
        cancelHeartbeat = nil
        server.stopObserving()
        capture.stop()
        audioSession.deactivate()
        phase = .off
        clock = nil
        expiresAt = nil
        generation += 1
        server.publish(SessionState(phase: .off, expiresAt: nil, heartbeatAt: now(), level: 0))
        server.purgeExpiredResult(now: now())
        activity.end(message: reason == .interrupted ? activityCopy().text(.sessionStoppedByCall) : nil)
        Self.log.info("session off: \(reason.rawValue, privacy: .public)")
    }

    /// Runs every second: heartbeat for the keyboard and the expiry check. The session only expires while idle.
    func tick() {
        guard phase != .off, var clock else { return }
        let now = now()
        server.purgeExpiredResult(now: now)
        if phase == .ready, clock.isExpired(now: now) {
            stop(reason: .expired)
            return
        }
        if phase != .ready {
            clock.extend(now: now)
            self.clock = clock
        }
        publish(at: now)
    }

    // MARK: Commands

    func receiveCommand() async {
        guard phase != .off, let command = server.takeCommand() else { return }
        await handle(command)
    }

    private func handle(_ command: HandoffCommand) async {
        switch command.action {
        case .start:
            guard phase == .ready else { return }
            let processing = runner()
            processingRunner = processing
            previewRequestID = command.id
            partialText = nil
            // The meeting hears the same microphone: it goes silent before the dictation starts keeping samples.
            if let meeting = meeting(), meeting.isActive {
                meeting.setDictating(true)
                meetingDictationStartMs = meeting.elapsedMs
            }
            phase = .listening
            let requestID = command.id
            let generation = generation
            processing.startPreview { [weak self] text in
                Task { @MainActor in
                    guard let self, self.phase == .listening,
                          self.previewRequestID == requestID, self.generation == generation else { return }
                    self.partialText = String(text.suffix(240))
                    self.publish(at: self.now())
                }
            }
            gate.open { processing.feed(samples: $0) }
            extendClock()
            cancelLevel = timers(0.1) { [weak self] in self?.publishLevel() }
        case .cancel:
            guard phase == .listening || phase == .processing else { return }
            if phase == .processing {
                generation += 1
            }
            processingRunner?.cancel()
            processingRunner = nil
            previewRequestID = nil
            partialText = nil
            _ = gate.close()
            endMeetingDictation()
            stopLevel()
            phase = .ready
            extendClock()
        case .stop:
            guard phase == .listening else {
                if phase == .ready { answerMissedStart(command) }
                return
            }
            await finish(command)
            return
        }
        publish(at: now())
        activity.update(activityState)
    }

    private func finish(_ command: HandoffCommand) async {
        guard let processing = processingRunner else {
            _ = gate.close()
            previewRequestID = nil
            partialText = nil
            endMeetingDictation()
            stopLevel()
            phase = .ready
            answerMissedStart(command)
            extendClock()
            publish(at: now())
            activity.update(activityState)
            return
        }
        let samples = gate.close()
        processing.stopPreview()
        previewRequestID = nil
        partialText = nil
        endMeetingDictation()
        stopLevel()
        phase = .processing
        publish(at: now())
        activity.update(activityState)

        let generation = generation
        let started = now()
        let outcome = await processing.run(samples: samples)
        guard generation == self.generation, phase == .processing else { return }
        processingRunner = nil
        let result: HandoffResult
        switch outcome {
        case .success(let dictation):
            result = HandoffResult(id: command.id, text: dictation.text, error: nil, at: now())
            let seconds = now().timeIntervalSince(started)
            Self.log.info("dictated \(samples.count, privacy: .public) samples, \(dictation.text.count, privacy: .public) chars, \(seconds, privacy: .public)s")
        case .failure(let error):
            result = HandoffResult(id: command.id, text: nil, error: ErrorText.message(error), errorCode: ErrorText.failureCode(error), at: now())
            Self.log.error("dictation failed: \(ErrorText.code(error), privacy: .public)")
        }
        server.publish(result)

        phase = .ready
        extendClock()
        publish(at: now())
        activity.update(activityState)
    }

    /// The keyboard waits for a result for its stop id; if its start was never seen, answer instead of leaving it hanging.
    private func answerMissedStart(_ command: HandoffCommand) {
        server.publish(HandoffResult(id: command.id, text: nil, error: ErrorText.message(PennelError.NoSpeech), errorCode: "no_speech", at: now()))
    }

    // MARK: Helpers

    private func extendClock() {
        clock?.extend(now: now())
    }

    private func stopLevel() {
        cancelLevel?()
        cancelLevel = nil
    }

    private func publishLevel() {
        guard phase == .listening else { return }
        publish(at: now())
    }

    private func endMeetingDictation() {
        guard let start = meetingDictationStartMs else { return }
        meetingDictationStartMs = nil
        guard let meeting = meeting(), meeting.isActive else { return }
        meeting.setDictating(false)
        meeting.markDictation(startMs: start, endMs: meeting.elapsedMs)
    }

    private func publish(at now: Date) {
        expiresAt = clock?.expiresAt
        server.publish(SessionState(
            phase: phase, expiresAt: clock?.expiresAt, heartbeatAt: now,
            level: phase == .listening ? gate.level : 0,
            partialText: phase == .listening ? partialText : nil,
            previewRequestID: phase == .listening ? previewRequestID : nil))
    }

    private var activityState: SessionActivityState {
        SessionActivityState(phase: phase, expiresAt: clock?.expiresAt ?? now(), level: 0, message: nil)
    }

    private func observeSystem(_ center: NotificationCenter) {
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: nil) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began
            else { return }
            Self.onMain { self?.stop(reason: .interrupted) }
        })
        observers.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: nil) { [weak self] _ in
            Self.onMain { self?.stop(reason: .mediaReset) }
        })
        observers.append(center.addObserver(forName: UIApplication.willTerminateNotification, object: nil, queue: nil) { [weak self] _ in
            Self.onMain { self?.stop(reason: .terminated) }
        })
    }

    /// Audio session notifications may arrive on any thread; on the main thread they are handled before `post` returns.
    private nonisolated static func onMain(_ body: @escaping @MainActor @Sendable () -> Void) {
        if Thread.isMainThread {
            MainActor.assumeIsolated(body)
        } else {
            Task { @MainActor in body() }
        }
    }

    static func runLoopTimers(_ interval: TimeInterval, _ action: @escaping @MainActor @Sendable () -> Void) -> (() -> Void) {
        let timer = Timer(timeInterval: interval, repeats: true) { _ in
            MainActor.assumeIsolated(action)
        }
        RunLoop.main.add(timer, forMode: .common)
        return { timer.invalidate() }
    }
}

/// The capture callback runs on the realtime audio thread. Opening, closing and appending share one lock, so a
/// chunk captured while `ready` (before a start or after a stop/cancel) can never land in a dictation.
private final class SampleGate: @unchecked Sendable {
    private let lock = NSLock()
    private let buffer = DictationBuffer()
    private var phase = SessionPhase.ready
    private var currentLevel: Float = 0
    private var onPreviewSamples: (@Sendable ([Float]) -> Void)?

    func feed(_ samples: [Float]) {
        let preview = lock.withLock { () -> (@Sendable ([Float]) -> Void)? in
            guard phase == .listening else { return nil }
            buffer.append(samples, phase: phase)
            currentLevel = Self.level(of: samples)
            return onPreviewSamples
        }
        preview?(samples)
    }

    func open(onPreviewSamples: @escaping @Sendable ([Float]) -> Void) {
        lock.withLock {
            _ = buffer.take()
            currentLevel = 0
            phase = .listening
            self.onPreviewSamples = onPreviewSamples
        }
    }

    func close() -> [Float] {
        lock.withLock {
            phase = .ready
            currentLevel = 0
            onPreviewSamples = nil
            return buffer.take()
        }
    }

    var level: Float { lock.withLock { currentLevel } }

    private static func level(of samples: [Float]) -> Float {
        guard !samples.isEmpty else { return 0 }
        let rms = (samples.reduce(0) { $0 + $1 * $1 } / Float(samples.count)).squareRoot()
        return min(1, rms * 10)
    }
}
