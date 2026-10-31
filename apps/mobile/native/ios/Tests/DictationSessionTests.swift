import AVFoundation
import Foundation
import PennelMobileKit
import Testing
@testable internal import PennelMobile

private final class FakeCapture: AudioCapturing, @unchecked Sendable {
    private let lock = NSLock()
    private var onSamples: (@Sendable ([Float]) -> Void)?
    private(set) var starts = 0
    private(set) var stops = 0

    var running: Bool { lock.withLock { onSamples != nil } }

    func start(onSamples: @escaping @Sendable ([Float]) -> Void) throws {
        lock.withLock {
            starts += 1
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

private final class FakeAudioSession: AudioSessionControlling, @unchecked Sendable {
    private(set) var activations = 0
    private(set) var deactivations = 0
    var active: Bool { activations > deactivations }

    func activate() throws { activations += 1 }
    func deactivate() { deactivations += 1 }
}

private final class FakeRunner: DictationRunning, @unchecked Sendable {
    private let lock = NSLock()
    private var received: [[Float]] = []
    private var previews: [@Sendable (String) -> Void] = []
    private var fed: [[Float]] = []
    var calls: [[Float]] { lock.withLock { received } }
    var previewSamples: [[Float]] { lock.withLock { fed } }

    func startPreview(onText: @escaping @Sendable (String) -> Void) { lock.withLock { previews.append(onText) } }
    func feed(samples: [Float]) { lock.withLock { fed.append(samples) } }
    func emitPreview(_ text: String, index: Int) { lock.withLock { previews[index] }(text) }

    func run(samples: [Float]) async -> Result<DictationResult, PennelError> {
        lock.withLock { received.append(samples) }
        return .success(DictationResult(historyId: "h1", historySaved: true, text: "hallo daar", raw: "hallo daar", cleanup: .skipped))
    }
}

@MainActor
private final class FakeActivity: SessionActivityControlling {
    private(set) var started: [SessionActivityState] = []
    private(set) var updates: [SessionActivityState] = []
    private(set) var ended: [String?] = []

    func start(_ state: SessionActivityState) { started.append(state) }
    func update(_ state: SessionActivityState) { updates.append(state) }
    func end(message: String?) { ended.append(message) }
}

@MainActor
private final class FakeMeeting: DictationMeetingHook {
    var isActive = true
    var elapsedMs: UInt64 = 0
    private(set) var dictating: [Bool] = []
    private(set) var marks: [(UInt64, UInt64)] = []

    func setDictating(_ dictating: Bool) { self.dictating.append(dictating) }
    func markDictation(startMs: UInt64, endMs: UInt64) { marks.append((startMs, endMs)) }
}

@MainActor
private final class TestClock {
    var now = Date(timeIntervalSince1970: 1_800_000_000)
}

@MainActor
private final class Harness {
    let capture = FakeCapture()
    let audio = FakeAudioSession()
    let runner = FakeRunner()
    let activity = FakeActivity()
    let center = NotificationCenter()
    let store = HandoffStore(directory: FileManager.default.temporaryDirectory
        .appendingPathComponent("pennel-session-\(UUID().uuidString)"))
    let clock: TestClock
    let session: DictationSession

    var now: Date {
        get { clock.now }
        set { clock.now = newValue }
    }

    init(meeting: FakeMeeting? = nil, activityLanguage: KeyboardInterfaceLanguage = .english) {
        let clock = TestClock()
        self.clock = clock
        session = DictationSession(
            capture: capture, audioSession: audio, runner: { [runner] in runner }, store: store,
            activity: activity, activityCopy: { ActivityCopy(language: activityLanguage) }, notifications: center, duration: 300,
            now: { clock.now }, timers: { _, _ in {} }, meeting: { meeting },
            observeCommands: false, requestPermission: { true })
    }

    var client: HandoffClient { HandoffClient(store: store, pending: PendingHandoff()) }

    @discardableResult
    func send(_ action: HandoffCommand.Action, with client: HandoffClient) async throws -> UUID {
        let id = try client.send(action, now: now)
        await session.receiveCommand()
        return id
    }

    var state: SessionState? { store.read(SessionState.self, as: .session) }
    var result: HandoffResult? { store.read(HandoffResult.self, as: .result) }
}

@MainActor
struct DictationSessionTests {
    @Test func preview_is_scoped_to_the_start_and_cleared_before_final_insertion() async throws {
        let h = Harness()
        #expect(await h.session.start())
        let client = h.client
        let startID = try await h.send(.start, with: client)
        h.capture.feed([0.2, 0.3])
        #expect(h.runner.previewSamples == [[0.2, 0.3]])
        h.runner.emitPreview("voorlopig", index: 0)
        for _ in 0..<100 { if h.state?.partialText == "voorlopig" { break }; await Task.yield() }
        #expect(h.state?.partialText == "voorlopig")
        #expect(h.state?.previewRequestID == startID)
        #expect(h.result == nil)
        try await h.send(.cancel, with: client)
        #expect(h.state?.partialText == nil)
        #expect(h.state?.previewRequestID == nil)
        let nextID = try await h.send(.start, with: client)
        h.runner.emitPreview("oude poging", index: 0)
        h.runner.emitPreview("nieuwe poging", index: 1)
        for _ in 0..<100 { if h.state?.partialText == "nieuwe poging" { break }; await Task.yield() }
        #expect(h.state?.partialText == "nieuwe poging")
        #expect(h.state?.previewRequestID == nextID)
        let stopID = try await h.send(.stop, with: client)
        #expect(h.state?.partialText == nil)
        #expect(h.state?.previewRequestID == nil)
        #expect(h.result?.id == stopID)
        #expect(h.result?.text == "hallo daar")
        h.session.stop()
        h.runner.emitPreview("late callback", index: 1)
        await Task.yield()
        #expect(h.state?.partialText == nil)
        #expect(h.state?.phase == .off)
    }

    @Test func start_stop_writes_result_with_command_id() async throws {
        let h = Harness()
        #expect(await h.session.start())
        #expect(h.audio.active)
        #expect(h.capture.running)
        #expect(h.activity.started.count == 1)
        #expect(h.state?.phase == .ready)
        #expect(h.state?.expiresAt == h.now.addingTimeInterval(300))

        let client = h.client
        try await h.send(.start, with: client)
        #expect(h.session.phase == .listening)
        #expect(h.state?.phase == .listening)
        h.capture.feed([0.1, 0.2])

        h.now = h.now.addingTimeInterval(200)
        let stopId = try await h.send(.stop, with: client)

        let result = try #require(h.result)
        #expect(result.id == stopId)
        #expect(result.text == "hallo daar")
        #expect(result.error == nil)
        #expect(client.takeResult(now: h.now)?.text == "hallo daar")
        #expect(h.session.phase == .ready)
        #expect(h.state?.phase == .ready)
        #expect(h.state?.expiresAt == h.now.addingTimeInterval(300))
        #expect(h.audio.active)
    }

    @Test func expired_result_is_purged_by_tick_and_stop() async throws {
        let h = Harness()
        #expect(await h.session.start())
        let client = h.client
        try await h.send(.start, with: client)
        try await h.send(.stop, with: client)
        #expect(h.result?.text == "hallo daar")
        // Listening keeps the session alive past its idle expiry, so only the result's age matters here.
        try await h.send(.start, with: client)

        h.now = h.now.addingTimeInterval(HandoffClient.resultLifetime)
        h.session.tick()
        #expect(h.result != nil)

        h.now = h.now.addingTimeInterval(1)
        h.session.tick()
        #expect(h.result == nil)
        #expect(h.session.phase == .listening)

        try h.store.write(HandoffResult(id: UUID(), text: "oud", error: nil, at: h.now.addingTimeInterval(-HandoffClient.resultLifetime - 1)), as: .result)
        h.session.stop()
        #expect(h.result == nil)
    }

    @Test func cancel_drops_audio_and_writes_no_result() async throws {
        let h = Harness()
        #expect(await h.session.start())
        let client = h.client
        try await h.send(.start, with: client)
        h.capture.feed([0.5, 0.5, 0.5])
        try await h.send(.cancel, with: client)

        #expect(h.session.phase == .ready)
        #expect(h.state?.phase == .ready)
        #expect(h.result == nil)
        #expect(h.runner.calls.isEmpty)

        try await h.send(.start, with: client)
        h.capture.feed([0.9])
        try await h.send(.stop, with: client)
        #expect(h.runner.calls == [[0.9]])
    }

    @Test func expiry_stops_and_writes_off() async throws {
        let h = Harness()
        #expect(await h.session.start())
        let started = h.now

        h.now = started.addingTimeInterval(299)
        h.session.tick()
        #expect(h.session.phase == .ready)
        #expect(h.state?.heartbeatAt == h.now)
        #expect(h.audio.active)

        h.now = started.addingTimeInterval(300)
        h.session.tick()
        #expect(h.session.phase == .off)
        #expect(h.state?.phase == .off)
        #expect(h.capture.running == false)
        #expect(h.audio.deactivations == 1)
        #expect(h.audio.active == false)
        #expect(h.activity.ended.count == 1)
        #expect(h.client.session(now: h.now) == .off)
    }

    @Test(arguments: [KeyboardInterfaceLanguage.english, .dutch])
    func interruption_stops_session_in_the_selected_language(_ language: KeyboardInterfaceLanguage) async throws {
        let h = Harness(activityLanguage: language)
        #expect(await h.session.start())
        try await h.send(.start, with: h.client)
        h.capture.feed([0.3])

        h.center.post(
            name: AVAudioSession.interruptionNotification, object: nil,
            userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue])

        #expect(h.session.phase == .off)
        #expect(h.state?.phase == .off)
        #expect(h.capture.running == false)
        #expect(h.audio.active == false)
        let expected = language == .english ? "Session stopped by a call." : "Sessie gestopt door een oproep."
        #expect(h.activity.ended == [expected])
        #expect(h.runner.calls.isEmpty)
        #expect(h.result == nil)

        #expect(await h.session.start())
        h.center.post(name: AVAudioSession.mediaServicesWereResetNotification, object: nil)
        #expect(h.session.phase == .off)
        #expect(h.audio.active == false)
    }

    @Test func ready_audio_never_reaches_runner() async throws {
        let h = Harness()
        #expect(await h.session.start())
        let client = h.client

        h.capture.feed([1, 1, 1])
        try await h.send(.start, with: client)
        h.capture.feed([2, 2])
        try await h.send(.stop, with: client)

        h.capture.feed([3, 3, 3])
        try await h.send(.start, with: client)
        h.capture.feed([4])
        try await h.send(.cancel, with: client)
        h.capture.feed([5, 5])

        try await h.send(.start, with: client)
        h.capture.feed([6])
        try await h.send(.stop, with: client)

        #expect(h.runner.calls == [[2, 2], [6]])
    }

    @Test func dictation_during_meeting_marks_interval() async throws {
        let meeting = FakeMeeting()
        let h = Harness(meeting: meeting)
        #expect(await h.session.start())
        let client = h.client

        meeting.elapsedMs = 61_000
        try await h.send(.start, with: client)
        #expect(meeting.dictating == [true])

        meeting.elapsedMs = 64_500
        h.capture.feed([0.2])
        try await h.send(.stop, with: client)

        #expect(meeting.dictating == [true, false])
        #expect(meeting.marks.count == 1)
        #expect(meeting.marks.first?.0 == 61_000)
        #expect(meeting.marks.first?.1 == 64_500)

        meeting.isActive = false
        try await h.send(.start, with: client)
        try await h.send(.stop, with: client)
        #expect(meeting.dictating == [true, false])
        #expect(meeting.marks.count == 1)
    }

    @Test func leftover_cleanup_never_ends_an_activity_started_after_launch() async {
        var existing = ["killed-run"]
        let ended = EndedIds()
        let cleanup = SystemSessionActivity.endLeftovers(existing) { await ended.add($0) }
        existing.append("new-session")
        await cleanup.value
        #expect(await ended.ids == ["killed-run"])
    }
}

private actor EndedIds {
    private(set) var ids: [String] = []
    func add(_ id: String) { ids.append(id) }
}
