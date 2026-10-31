import AVFoundation
import Foundation
import PennelMobileKit
@testable internal import PennelMobile

final class ManualClock: @unchecked Sendable {
    private let lock = NSLock()
    private var value: TimeInterval = 1_000

    var now: TimeInterval { lock.withLock { value } }

    func advance(_ seconds: TimeInterval) {
        lock.withLock { value += seconds }
    }
}

final class MeetingFakeCapture: AudioCapturing, @unchecked Sendable {
    private let lock = NSLock()
    private var onSamples: (@Sendable ([Float]) -> Void)?
    private var failing = false
    private(set) var starts = 0
    private(set) var stops = 0

    var running: Bool { lock.withLock { onSamples != nil } }
    var inputFormat: MicrophoneInputFormat? {
        running ? MicrophoneInputFormat(sampleRate: 24_000, channels: 1, interleaved: false) : nil
    }

    func failNextStarts() {
        lock.withLock { failing = true }
    }

    func start(onSamples: @escaping @Sendable ([Float]) -> Void) throws {
        try lock.withLock {
            starts += 1
            if failing { throw MicCapture.Failure.unavailable }
            self.onSamples = onSamples
        }
    }

    func stop() {
        lock.withLock {
            stops += 1
            onSamples = nil
        }
    }

    func feed(_ samples: [Float]) {
        lock.withLock { onSamples }?(samples)
    }
}

final class MeetingFakeAudioSession: AudioSessionControlling, @unchecked Sendable {
    private(set) var activations = 0
    private(set) var deactivations = 0
    var active: Bool { activations > deactivations }

    func activate() throws { activations += 1 }
    func deactivate() { deactivations += 1 }
}

final class MeetingFakeTranscriber: SystemTranscriber, @unchecked Sendable {
    private let lock = NSLock()
    private var counts: [Int] = []
    var calls: [Int] { lock.withLock { counts } }

    func transcribe(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> String {
        lock.withLock { counts.append(samples.count) }
        return "hallo wereld"
    }
}

@MainActor
final class FakeRecordingActivity: RecordingActivityControlling {
    private(set) var started: [RecordingActivityState] = []
    private(set) var updates: [RecordingActivityState] = []
    private(set) var ended = 0

    func start(_ state: RecordingActivityState) { started.append(state) }
    func update(_ state: RecordingActivityState) { updates.append(state) }
    func end() { ended += 1 }
}

@MainActor
final class FakeFinishTask: FinishTask {
    private(set) var progress: [UInt32] = []
    private(set) var ended: [Bool] = []

    func update(pending: UInt32) { progress.append(pending) }
    func end(success: Bool) { ended.append(success) }
}

@MainActor
final class FakeFinishScheduler: FinishTaskScheduling {
    private(set) var begun = 0
    let task = FakeFinishTask()

    func begin() -> FinishTask? {
        begun += 1
        return task
    }
}

func meetingTone(seconds: Double) -> [Float] {
    (0..<Int(16_000 * seconds)).map { 0.4 * sin(Float($0) * 0.05) }
}

@MainActor
final class MeetingHarness {
    let engine: PennelEngine
    let transcriber = MeetingFakeTranscriber()
    let capture = MeetingFakeCapture()
    let audio = MeetingFakeAudioSession()
    let activity = FakeRecordingActivity()
    let scheduler = FakeFinishScheduler()
    let center = NotificationCenter()
    let clock = ManualClock()
    let interruptions: MeetingInterruptionLog
    let dataDir: URL
    private(set) var finished: [MeetingFinish] = []
    private(set) var runtime: MeetingRuntime!

    init() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-meeting-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        dataDir = dir
        engine = try PennelEngine.newPreview(dataDir: dir.path)
        engine.setSystemTranscriber(transcriber: transcriber)
        try FirstRunDefaults.apply(to: engine, appleIntelligenceAvailable: false, hasKey: { _ in false })
        interruptions = MeetingInterruptionLog(defaults: UserDefaults(suiteName: "pennel-test-\(UUID().uuidString)")!)
        let clock = clock
        runtime = MeetingRuntime(
            engine: engine, capture: capture, audioSession: audio, activity: activity, scheduler: scheduler,
            interruptions: interruptions, notifications: center, keys: { ApiKeys(byProvider: [:]) },
            clock: { clock.now },
            timers: { _, _ in {} }, hasMicrophonePermission: { true })
        runtime.onFinished = { [unowned self] in self.finished.append($0) }
    }

    /// Feeds tone in 0.1 s callbacks, with the clock moving as a real microphone would.
    func speak(seconds: Double) {
        for _ in 0..<Int((seconds * 10).rounded()) {
            clock.advance(0.1)
            capture.feed(meetingTone(seconds: 0.1))
        }
    }

    func silence(seconds: TimeInterval) {
        clock.advance(seconds)
    }

    func interruption(_ type: AVAudioSession.InterruptionType, shouldResume: Bool = false) {
        var info: [AnyHashable: Any] = [AVAudioSessionInterruptionTypeKey: type.rawValue]
        if type == .ended {
            info[AVAudioSessionInterruptionOptionKey] = shouldResume ? AVAudioSession.InterruptionOptions.shouldResume.rawValue : UInt(0)
        }
        center.post(name: AVAudioSession.interruptionNotification, object: nil, userInfo: info)
    }

    func transcript(_ id: String) -> [Segment] {
        engine.meetingTranscript(id: id)
    }
}
