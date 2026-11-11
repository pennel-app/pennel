import AVFoundation
import XCTest
@testable import PennelMacPlatform

final class MeetingCaptureHealthTests: XCTestCase {
    func testLiveLevelsUseRawBuffersAndStopAtQuiescenceWithoutChangingArchivedAudio() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let probe = MeetingLevelProbe()
        let writer = try TrackWriter(track: 1, sink: { context, track, _, count, _ in
            let probe = Unmanaged<MeetingLevelProbe>.fromOpaque(context!).takeUnretainedValue()
            probe.track = track; probe.archivedSamples += count
        }, context: Unmanaged.passUnretained(probe).toOpaque(), started: 0,
        url: directory.appendingPathComponent("system.m4a"), level: { probe.levelSources.append($0); probe.levels.append($1) })
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: MonoResampler.format, frameCapacity: 800))
        buffer.frameLength = 800
        buffer.floatChannelData![0].initialize(repeating: 0.25, count: 800)
        writer.append(buffer, at: 0)
        XCTAssertEqual(probe.levelSources, [1], "raw levels retain their audio source")
        XCTAssertEqual(probe.levels.count, 1, "levels arrive before the one-second transcript batch")
        XCTAssertEqual(probe.levels[0], 0.25, accuracy: 0.001)
        writer.quiesce()
        writer.append(buffer, at: 0.05)
        XCTAssertEqual(probe.levels.count, 1, "quiescence rejects pending level callbacks before Rust releases them")
        XCTAssertNil(writer.finish())
        XCTAssertEqual(probe.archivedSamples, 800)
        XCTAssertEqual(probe.track, 1, "level reporting preserves system/microphone source identity")
    }

    func testQuiescenceClosesAdmissionWithoutWaitingForBlockedArchiveWrites() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let writing = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        let writer = try TrackWriter(
            track: 0, sink: { _, _, _, _, _ in }, context: nil, started: 0,
            url: directory.appendingPathComponent("ik.m4a"),
            write: { _ in writing.signal(); release.wait() }
        )
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: MonoResampler.format, frameCapacity: 16_000))
        buffer.frameLength = 16_000
        buffer.floatChannelData![0].initialize(repeating: 0.1, count: 16_000)
        writer.append(buffer, at: 0)
        XCTAssertEqual(writing.wait(timeout: .now() + 2), .success)
        let quiescent = DispatchSemaphore(value: 0)
        DispatchQueue.global().async { writer.quiesce(); quiescent.signal() }
        let closedBeforeWriteFinished = quiescent.wait(timeout: .now() + 1) == .success
        if closedBeforeWriteFinished { writer.append(buffer, at: 1) }
        release.signal()
        XCTAssertTrue(closedBeforeWriteFinished)
        XCTAssertNil(writer.finish())
        XCTAssertEqual(writing.wait(timeout: .now() + 0.1), .timedOut, "quiescence must reject later audio")
    }

    func testFailedMicrophoneRestartRemainsVisibleUntilCaptureActuallyStarts() {
        var health = MeetingCaptureHealth(echoRequested: true)
        health.microphoneStarted(echoActive: true)
        health.microphoneFailed()
        XCTAssertFalse(health.microphoneActive)
        XCTAssertNotNil(health.microphoneError)
        health.microphoneStarting()
        XCTAssertNotNil(health.microphoneError, "a restart attempt is not recovery")
        health.microphoneStarted(echoActive: false)
        XCTAssertTrue(health.microphoneActive)
        XCTAssertNil(health.microphoneError)
        XCTAssertFalse(health.echoActive)
        XCTAssertNotNil(health.echoWarning)
    }

    func testSystemAudioFailureClearsOnlyAfterASuccessfulRestart() {
        var health = MeetingCaptureHealth(echoRequested: false)
        health.systemAudioFailed()
        XCTAssertFalse(health.systemAudioAvailable)
        XCTAssertNotNil(health.systemAudioError)
        XCTAssertTrue(health.bridgeObject(heardSystemAudio: false, archiveError: nil)["terminal_error"] is NSNull)
        health.systemAudioStarting()
        XCTAssertNotNil(health.systemAudioError)
        health.systemAudioStarted()
        XCTAssertTrue(health.systemAudioAvailable)
        XCTAssertNil(health.systemAudioError)
    }

    func testEchoAvailabilityDescribesTheRunningEngineRatherThanThePreference() {
        var health = MeetingCaptureHealth(echoRequested: true)
        XCTAssertFalse(health.echoActive)
        health.microphoneStarted(echoActive: false)
        XCTAssertFalse(health.echoActive)
        XCTAssertNotNil(health.echoWarning)
        health.microphoneStarted(echoActive: true)
        XCTAssertTrue(health.echoActive)
        XCTAssertNil(health.echoWarning)
        health.microphoneFailed()
        XCTAssertFalse(health.echoActive)
    }

    func testHealthBridgeKeepsArchiveFailuresSeparateFromCaptureAvailability() throws {
        var health = MeetingCaptureHealth(echoRequested: false)
        health.microphoneStarted(echoActive: false)
        health.systemAudioStarted()
        let snapshot = health.bridgeObject(heardSystemAudio: true, archiveError: "Audio bewaren mislukt.")
        XCTAssertEqual(snapshot["microphone_active"] as? Bool, true)
        XCTAssertEqual(snapshot["system_audio_available"] as? Bool, true)
        XCTAssertEqual(snapshot["heard_system_audio"] as? Bool, true)
        XCTAssertEqual(snapshot["echo_cancellation"] as? Bool, false)
        XCTAssertEqual(snapshot["archive_error"] as? String, "Audio bewaren mislukt.")
        XCTAssertEqual(snapshot["terminal_error"] as? String, "Audio bewaren mislukt.")
        XCTAssertTrue(snapshot["microphone_error"] is NSNull)
        XCTAssertNoThrow(try JSONSerialization.data(withJSONObject: snapshot))
    }

    func testArchiveFailureIsImmediatelyReadableAndNeverErasesTranscriptAudio() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let received = CapturedSamples()
        let context = Unmanaged.passUnretained(received).toOpaque()
        let sink: AudioSink = { context, _, samples, count, _ in
            guard let context, let samples else { return }
            let received = Unmanaged<CapturedSamples>.fromOpaque(context).takeUnretainedValue()
            received.count += Array(UnsafeBufferPointer(start: samples, count: count)).count
        }
        let writer = try TrackWriter(
            track: 0, sink: sink, context: context, started: 0,
            url: directory.appendingPathComponent("ik.m4a"),
            write: { _ in throw CocoaError(.fileWriteOutOfSpace) }
        )
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: MonoResampler.format, frameCapacity: 16_000))
        buffer.frameLength = 16_000
        buffer.floatChannelData![0].initialize(repeating: 0.1, count: 16_000)
        writer.append(buffer, at: 0)
        XCTAssertNotNil(writer.finish())
        XCTAssertNotNil(writer.archiveError)
        XCTAssertEqual(received.count, 16_000)
    }

    func testLaterArchiveWritesDoNotHideAnEarlierMissingBlock() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        var attempts = 0
        let writer = try TrackWriter(
            track: 0, sink: { _, _, _, _, _ in }, context: nil, started: 0,
            url: directory.appendingPathComponent("ik.m4a"),
            write: { _ in
                attempts += 1
                if attempts == 1 { throw CocoaError(.fileWriteOutOfSpace) }
            }
        )
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: MonoResampler.format, frameCapacity: 16_000))
        buffer.frameLength = 16_000
        buffer.floatChannelData![0].initialize(repeating: 0.1, count: 16_000)
        writer.append(buffer, at: 0)
        writer.append(buffer, at: 1)
        XCTAssertNotNil(writer.finish())
        XCTAssertEqual(attempts, 2)
        XCTAssertNotNil(writer.archiveError)
    }
}

private final class CapturedSamples {
    var count = 0
}

private final class MeetingLevelProbe {
    var levels: [Float] = []
    var levelSources: [UInt32] = []
    var archivedSamples = 0
    var track: UInt32 = 0
}
