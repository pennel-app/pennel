import AVFoundation
import Foundation
import PennelMobileKit
import Testing
import UIKit
@testable internal import PennelMobile

private final class CountingCapture: AudioCapturing, @unchecked Sendable {
    private let lock = NSLock()
    private var onSamples: (@Sendable ([Float]) -> Void)?
    private var alive = false
    private(set) var starts = 0
    private(set) var stops = 0

    var isRunning: Bool { lock.withLock { alive } }

    func die() {
        lock.withLock { alive = false }
    }

    func start(onSamples: @escaping @Sendable ([Float]) -> Void) throws {
        lock.withLock {
            starts += 1
            alive = true
            self.onSamples = onSamples
        }
    }

    func stop() {
        lock.withLock {
            stops += 1
            alive = false
            onSamples = nil
        }
    }

    func feed(_ samples: [Float]) {
        lock.withLock { onSamples }?(samples)
    }
}

private final class Received: @unchecked Sendable {
    private let lock = NSLock()
    private var total = 0
    var count: Int { lock.withLock { total } }
    func add(_ samples: [Float]) { lock.withLock { total += samples.count } }
}

private final class FakeSessionBackend: AudioSessionBackend, @unchecked Sendable {
    private let lock = NSLock()
    private var log: [String] = []
    var calls: [String] { lock.withLock { log } }

    func configure(_ purpose: SystemAudioSession.Purpose, speaker: Bool) throws {
        lock.withLock { log.append("configure \(purpose)\(speaker ? " speaker" : "")") }
    }

    func setActive() throws { lock.withLock { log.append("active") } }
    func deactivate() { lock.withLock { log.append("inactive") } }
}

@MainActor
struct MeetingRuntimeTests {
    @Test func stop_handoff_includes_latest_notes_after_an_owned_inflight_autosave() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Final notes")
        let id = try #require(h.runtime.activeMeetingId)
        try h.runtime.saveLiveNotes("Acknowledged baseline")
        // An owned autosave reached native before JS received its acknowledgement.
        try h.runtime.saveLiveNotes("Earlier pending autosave")
        h.speak(seconds: 3)
        h.runtime.requestStop(notes: MeetingStopNotes(text: "Latest notes at Stop", expected: "Acknowledged baseline", pending: "Earlier pending autosave"))
        #expect(!h.capture.running)
        #expect(h.runtime.pendingNotes == "Latest notes at Stop")
        #expect(throws: (any Error).self) { try h.runtime.saveLiveNotes("Late old autosave") }

        await h.runtime.stop()
        #expect(h.engine.meetingNotes(id: id) == "Latest notes at Stop")
        #expect(h.runtime.pendingNotes == nil)
        #expect(h.runtime.retainedId == nil)
        #expect(h.finished.first?.error == nil)
    }

    @Test func failed_stop_note_handoff_retains_native_draft_and_never_overwrites_external_notes() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Note conflict")
        let id = try #require(h.runtime.activeMeetingId)
        let session = try #require(h.runtime.transcriptSnapshotSession())
        try h.runtime.saveLiveNotes("Acknowledged baseline")
        try session.setNotes(text: "Unrelated external notes")
        h.speak(seconds: 3)
        h.runtime.requestStop(notes: MeetingStopNotes(text: "Latest native-owned draft", expected: "Acknowledged baseline", pending: nil))
        await h.runtime.stop()

        #expect(!h.capture.running)
        #expect(h.runtime.retainedId == id)
        #expect(h.runtime.notes == "Acknowledged baseline")
        #expect(h.runtime.pendingNotes == "Latest native-owned draft")
        #expect(h.engine.meetingNotes(id: id) == "Unrelated external notes")
        #expect(h.finished.first?.error != nil)
        await #expect(throws: (any Error).self) { try await h.runtime.retrySave() }
        #expect(h.runtime.pendingNotes == "Latest native-owned draft")
        #expect(h.engine.meetingNotes(id: id) == "Unrelated external notes")

        // Explicit user reconciliation makes the retained retry idempotent.
        try session.setNotes(text: "Latest native-owned draft")
        try await h.runtime.retrySave()
        #expect(h.runtime.retainedId == nil)
        #expect(h.runtime.pendingNotes == nil)
        #expect(h.runtime.lastFinish?.error == nil)
    }

    @Test func interactive_stop_quiesces_capture_before_saving_and_is_idempotent() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Immediate Stop")
        let id = try #require(h.runtime.activeMeetingId)
        h.speak(seconds: 3)

        h.runtime.requestStop()
        #expect(h.runtime.phase == .finishing)
        #expect(h.runtime.activeMeetingId == id)
        #expect(h.runtime.lastFinish == nil)
        #expect(!h.capture.running)
        #expect(!h.audio.active)
        #expect(h.scheduler.begun == 1)
        #expect(h.scheduler.task.ended.isEmpty)
        let stops = h.capture.stops
        h.runtime.requestStop()
        #expect(h.capture.stops == stops)

        await h.runtime.stop()
        #expect(h.runtime.phase == .idle)
        #expect(h.finished.map(\.id) == [id])
        #expect(h.transcript(id).count == 1)
    }

    @Test func recording_diagnostics_classify_transport_without_device_identifiers() {
        #expect(MeetingRuntime.recordingTransport(.builtInMic) == .builtIn)
        #expect(MeetingRuntime.recordingTransport(.bluetoothHFP) == .bluetooth)
        #expect(MeetingRuntime.recordingTransport(.bluetoothLE) == .bluetooth)
        #expect(MeetingRuntime.recordingTransport(.usbAudio) == .usb)
        #expect(MeetingRuntime.recordingTransport(.headsetMic) == .wired)
        #expect(MeetingRuntime.recordingTransport(nil) == .unknown)
    }
    /// iOS only grants continued processing to a request made from the user's action in the foreground, so the
    /// task is requested at Stop opname and keeps running if the user then leaves the app.
    @Test func stop_during_background_schedules_finish_task() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        h.speak(seconds: 5)
        await h.runtime.stop()

        #expect(h.scheduler.begun == 1)
        #expect(h.scheduler.task.ended == [true])
    }

    @Test func failed_capture_is_not_reported_complete() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        let id = try #require(h.runtime.activeMeetingId)
        h.speak(seconds: 16)

        h.interruption(.began)
        h.silence(seconds: 20)
        h.capture.failNextStarts()
        h.interruption(.ended, shouldResume: true)
        await h.runtime.stop()

        #expect(h.finished.count == 1)
        let finish = try #require(h.finished.first)
        #expect(finish.id == id)
        #expect(!finish.isComplete)
        #expect(finish.stoppedEarly == MeetingRuntime.Message.couldNotResume)
        #expect(finish.gaps == [MeetingGap(startMs: 16_000, endMs: nil)])
        #expect(h.interruptions.record(for: id)?.stoppedEarly == MeetingRuntime.Message.couldNotResume)
        #expect(h.runtime.phase == .idle)
        #expect(h.runtime.activeMeetingId == nil)
        #expect(!h.capture.running)
        #expect(!h.audio.active)
        #expect(h.activity.ended == 1)
        #expect(h.transcript(id).count == 1)
        #expect(h.engine.meetingInfo(id: id)?.hasSummary == false)
    }

    @Test func active_meeting_id_exposed_while_recording() async throws {
        let h = try MeetingHarness()
        #expect(h.runtime.activeMeetingId == nil)
        try await h.runtime.start(title: "Overleg")
        let id = try #require(h.runtime.activeMeetingId)
        #expect(h.engine.meetings().map(\.id) == [id])
        #expect(h.runtime.isActive)
        #expect(h.activity.started.count == 1)
        await #expect(throws: (any Error).self) { try await h.runtime.start(title: "Tweede") }

        h.speak(seconds: 3)
        await h.runtime.stop()

        #expect(h.runtime.activeMeetingId == nil)
        #expect(!h.runtime.isActive)
        #expect(h.finished.map(\.id) == [id])
        #expect(h.finished.first?.isComplete == true)
        #expect(h.interruptions.record(for: id) == nil)
    }

    @Test func meeting_whose_save_failed_stays_out_of_sync() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        let id = try #require(h.runtime.activeMeetingId)
        h.runtime.setNotes("Retained notes from before the failed final save")
        #expect(h.runtime.unfinalisedMeetingIds == [id])
        h.speak(seconds: 3)

        let folders = try #require(FileManager.default.enumerator(at: h.dataDir, includingPropertiesForKeys: [.isDirectoryKey]))
            .compactMap { $0 as? URL }
            .filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }
        let locked = folders + [h.dataDir]
        for folder in locked { try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: folder.path) }
        defer { for folder in locked { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: folder.path) } }
        await h.runtime.stop()

        let retained = try #require(h.runtime.retainedId)
        #expect(h.runtime.activeMeetingId == nil)
        #expect(h.runtime.unfinalisedMeetingIds == [retained])
        #expect(h.runtime.notes == "Retained notes from before the failed final save")
        #expect(h.runtime.title == "Overleg")
        #expect(h.runtime.elapsedMs == 3_000)
    }

    @Test func termination_mid_recording_marks_the_meeting_stopped_early() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        let id = try #require(h.runtime.activeMeetingId)
        h.speak(seconds: 3)

        h.center.post(name: UIApplication.willTerminateNotification, object: nil)

        #expect(h.interruptions.record(for: id)?.stoppedEarly == MeetingRuntime.Message.terminated)
    }

    @Test func live_notes_survive_a_reopened_view_and_stop() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        let id = try #require(h.runtime.activeMeetingId)
        h.runtime.setNotes("Budget beslist")

        // A reopened live view starts from the runtime's notes and sends them again on Stop opname.
        let reopened = h.runtime.notes
        #expect(reopened == "Budget beslist")
        h.runtime.setNotes(reopened)
        h.speak(seconds: 2)
        await h.runtime.stop()

        #expect(h.engine.meetingNotes(id: id) == "Budget beslist")
        #expect(h.runtime.notes.isEmpty)
    }

    @Test func stalled_capture_restarts_and_leaves_a_gap() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        h.speak(seconds: 5)
        h.silence(seconds: 3)
        h.runtime.tick()
        #expect(h.capture.starts == 2)
        #expect(h.runtime.warning == MeetingRuntime.Message.microphoneLost)
        #expect(h.runtime.phase == .recording)
        h.speak(seconds: 5)
        h.runtime.tick()
        #expect(h.runtime.warning == nil)
        await h.runtime.stop()

        let finish = try #require(h.finished.first)
        #expect(finish.gaps == [MeetingGap(startMs: 5_000, endMs: 8_000)])
        #expect(!finish.isComplete)
    }

    @Test func stall_until_stop_is_not_reported_complete() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        h.speak(seconds: 5)
        h.silence(seconds: 1.5)
        await h.runtime.stop()

        let finish = try #require(h.finished.first)
        #expect(finish.gaps == [MeetingGap(startMs: 5_000, endMs: nil)])
        #expect(!finish.isComplete)
        #expect(h.interruptions.record(for: finish.id) != nil)
    }

    @Test func failed_restart_after_a_stall_finishes_once_as_stopped_early() async throws {
        let h = try MeetingHarness()
        try await h.runtime.start(title: "Overleg")
        h.speak(seconds: 5)
        h.silence(seconds: 3)
        h.capture.failNextStarts()
        h.runtime.tick()
        h.runtime.tick()
        await h.runtime.stop()

        #expect(h.finished.count == 1)
        #expect(h.finished.first?.stoppedEarly == MeetingRuntime.Message.couldNotResume)
        #expect(h.finished.first?.isComplete == false)
        #expect(h.runtime.phase == .idle)
    }

    @Test func shared_microphone_restarts_a_dead_engine() throws {
        let capture = CountingCapture()
        let center = NotificationCenter()
        let microphone = SharedMicrophone(capture: capture, notifications: center)
        let meeting = microphone.client()
        let received = Received()
        try meeting.start { received.add($0) }

        center.post(name: .AVAudioEngineConfigurationChange, object: nil)
        #expect(capture.starts == 1)

        capture.die()
        #expect(!meeting.isRunning)
        center.post(name: AVAudioSession.routeChangeNotification, object: nil)
        #expect(capture.starts == 2)
        #expect(meeting.isRunning)

        // A media-services reset kills the engine; the next client must not trust the stale running flag.
        capture.die()
        let session = microphone.client()
        try session.start { _ in }
        #expect(capture.starts == 3)
        capture.feed([0.1])
        #expect(received.count == 1)
    }

    @Test func shared_microphone_restarts_a_single_client_and_ignores_late_events_after_stop() throws {
        let capture = CountingCapture()
        let center = NotificationCenter()
        let microphone = SharedMicrophone(capture: capture, notifications: center)
        let meeting = microphone.client()
        let received = Received()
        try meeting.start { received.add($0) }
        defer { meeting.stop() }
        capture.feed([0.1])
        #expect(received.count == 1)

        capture.die()
        #expect(!meeting.isRunning)
        center.post(name: .AVAudioEngineConfigurationChange, object: nil)
        #expect(capture.starts == 2)
        #expect(capture.stops == 1)
        #expect(meeting.isRunning)
        capture.feed([0.2, 0.3])
        #expect(received.count == 3)

        meeting.stop()
        #expect(!meeting.isRunning)
        #expect(capture.stops == 2)
        for event in [Notification.Name.AVAudioEngineConfigurationChange,
                      AVAudioSession.routeChangeNotification,
                      AVAudioSession.mediaServicesWereResetNotification] {
            center.post(name: event, object: nil)
            capture.feed([0.4])
            #expect(capture.starts == 2)
            #expect(capture.stops == 2)
            #expect(!meeting.isRunning)
            #expect(received.count == 3)
        }
    }

    @Test func recording_takes_the_category_from_playback() throws {
        let backend = FakeSessionBackend()
        let shared = SharedAudioSession(backend: backend)
        let playback = shared.client(.playback)
        let meeting = shared.client(.meeting)

        try playback.activate()
        try meeting.activate()
        playback.deactivate()
        meeting.deactivate()
        #expect(backend.calls == ["configure playback", "active", "configure meeting speaker", "active", "inactive"])
    }

    @Test func playback_during_a_recording_uses_the_loudspeaker() throws {
        let backend = FakeSessionBackend()
        let shared = SharedAudioSession(backend: backend)
        let session = shared.client(.keyboardSession)
        let playback = shared.client(.playback)

        try session.activate()
        try playback.activate()
        playback.deactivate()
        try playback.activate()
        #expect(backend.calls == ["configure keyboardSession", "active", "configure keyboardSession speaker", "active", "active"])

        playback.deactivate()
        session.deactivate()
        try SharedAudioSession(backend: backend).client(.dictation).activate()
        #expect(backend.calls.suffix(3) == ["inactive", "configure dictation", "active"])
    }

    @Test func a_finished_playback_no_longer_blocks_the_recording_category() throws {
        let backend = FakeSessionBackend()
        let shared = SharedAudioSession(backend: backend)
        let playback = shared.client(.playback)
        try playback.activate()
        playback.deactivate()
        try shared.client(.dictation).activate()
        #expect(backend.calls == ["configure playback", "active", "inactive", "configure dictation", "active"])
    }

    @Test func shared_microphone_runs_one_capture_for_meeting_and_session() throws {
        let capture = CountingCapture()
        let microphone = SharedMicrophone(capture: capture)
        let meeting = microphone.client()
        let session = microphone.client()
        let meetingSamples = Received()
        let sessionSamples = Received()

        try meeting.start { meetingSamples.add($0) }
        try session.start { sessionSamples.add($0) }
        capture.feed([0.1, 0.2])
        #expect(capture.starts == 1)
        #expect(meetingSamples.count == 2)
        #expect(sessionSamples.count == 2)

        session.stop()
        capture.feed([0.3])
        #expect(capture.stops == 0)
        #expect(meetingSamples.count == 3)
        #expect(sessionSamples.count == 2)

        meeting.stop()
        #expect(capture.stops == 1)
    }
}
