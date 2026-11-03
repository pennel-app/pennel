import AVFoundation
import XCTest
@testable import PennelMacPlatform

final class LiveSpeechPreviewTests: XCTestCase {
    func testVolatileCorrectionReplacesItsEarlierRange() {
        var caption = LiveSpeechCaptionReducer()
        XCTAssertEqual(caption.update(start: 0, end: 1, text: "Een", isFinal: false), "Een")
        XCTAssertEqual(caption.update(start: 0, end: 2, text: "Een eerste zin", isFinal: false), "Een eerste zin")
        XCTAssertEqual(caption.update(start: 0, end: 2, text: "Een eerste zin.", isFinal: true), "Een eerste zin.")
        XCTAssertEqual(caption.text, "Een eerste zin.")
    }

    func testNewPhraseKeepsCommittedTextWithoutDuplicatingPartials() {
        var caption = LiveSpeechCaptionReducer()
        _ = caption.update(start: 0, end: 2, text: "Hallo wereld.", isFinal: true)
        _ = caption.update(start: 2, end: 3, text: "Dit", isFinal: false)
        XCTAssertEqual(caption.update(start: 2, end: 4, text: "Dit is Pennel.", isFinal: true), "Hallo wereld. Dit is Pennel.")
        XCTAssertNil(caption.update(start: 2, end: 4, text: "Dit is Pennel.", isFinal: true))
    }

    func testCommittedRangeCannotBeOverwrittenByLateVolatileText() {
        var caption = LiveSpeechCaptionReducer()
        _ = caption.update(start: 0, end: 2, text: "Een correcte zin.", isFinal: true)
        XCTAssertNil(caption.update(start: 0, end: 2, text: "Een oude gok", isFinal: false))
        XCTAssertEqual(caption.text, "Een correcte zin.")
    }

    func testFinalRangeReplacesOverlappingVolatileFragments() {
        var caption = LiveSpeechCaptionReducer()
        _ = caption.update(start: 0, end: 1, text: "Een", isFinal: false)
        _ = caption.update(start: 1, end: 2, text: "zin", isFinal: false)
        XCTAssertEqual(caption.update(start: 0, end: 2, text: "Een zin.", isFinal: true), "Een zin.")
        XCTAssertNil(caption.update(start: .nan, end: 3, text: "Ongeldig", isFinal: false))
    }

    func testCaptionTailPreservesUnicodeAndRemainsBounded() {
        var caption = LiveSpeechCaptionReducer(maxCharacters: 12)
        _ = caption.update(start: 0, end: 1, text: "Een lange tekst met 👩🏽‍💻 en café.", isFinal: true)
        XCTAssertLessThanOrEqual(caption.text.count, 12)
        XCTAssertTrue(caption.text.hasSuffix("en café."))
    }

    func testPreparationBufferKeepsOnlyBoundedNewestAudioAndItsOffset() {
        let buffer = LiveSpeechPreviewBuffer(maxSamples: 5)
        XCTAssertTrue(buffer.start())
        XCTAssertTrue(buffer.append([0, 1, 2]))
        XCTAssertTrue(buffer.append([3, 4, 5, 6, 7]))
        let chunk = buffer.drain()
        XCTAssertEqual(chunk?.samples, [3, 4, 5, 6, 7])
        XCTAssertEqual(chunk?.startFrame, 3)
        XCTAssertNil(buffer.drain())
    }

    func testTimedInputPreservesRecordingOffsetAndDoesNotBridgeMissingAudio() {
        let buffer = LiveSpeechPreviewBuffer(maxSamples: 5)
        XCTAssertTrue(buffer.start())
        XCTAssertTrue(buffer.append([1, 2, 3], startFrame: 16_000))
        XCTAssertTrue(buffer.append([4, 5], startFrame: 32_000))
        let chunk = buffer.drain()
        XCTAssertEqual(chunk?.samples, [4, 5])
        XCTAssertEqual(chunk?.startFrame, 32_000)
    }

    func testTimedInputRejectsRepeatedFramesAndKeepsOnlyUnseenSuffix() {
        let buffer = LiveSpeechPreviewBuffer(maxSamples: 5)
        XCTAssertTrue(buffer.start())
        XCTAssertTrue(buffer.append([1, 2, 3], startFrame: 100))
        XCTAssertFalse(buffer.append([1, 2], startFrame: 100))
        XCTAssertTrue(buffer.append([3, 4, 5], startFrame: 102))
        let chunk = buffer.drain()
        XCTAssertEqual(chunk?.samples, [1, 2, 3, 4, 5])
        XCTAssertEqual(chunk?.startFrame, 100)
    }

    func testTimedEventsRetainExactRangeAndFinalityWithoutBecomingFinalTranscript() throws {
        let event = LiveSpeechPreviewEvent.fragment(startMS: 1_250, endMS: 2_750, text: "café 👩🏽‍💻", isFinal: true)
        let data = try JSONEncoder().encode(event)
        let value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(value["kind"] as? String, "fragment")
        XCTAssertEqual(value["start_ms"] as? Int, 1_250)
        XCTAssertEqual(value["end_ms"] as? Int, 2_750)
        XCTAssertEqual(value["text"] as? String, "café 👩🏽‍💻")
        XCTAssertEqual(value["is_final"] as? Bool, true)
    }

    func testTimedEventsRejectInvalidRangesAndBoundDisplayText() {
        XCTAssertNil(LiveSpeechPreviewEvent.timedFragment(start: .nan, end: 2, text: "invalid", isFinal: false))
        XCTAssertNil(LiveSpeechPreviewEvent.timedFragment(start: 2, end: 1, text: "invalid", isFinal: false))
        XCTAssertNil(LiveSpeechPreviewEvent.timedFragment(start: -1, end: 1, text: "invalid", isFinal: false))
        XCTAssertNil(LiveSpeechPreviewEvent.timedFragment(start: 0, end: .infinity, text: "invalid", isFinal: false))
        XCTAssertNil(LiveSpeechPreviewEvent.timedFragment(start: 0, end: 1, text: " \n ", isFinal: false))
        let event = LiveSpeechPreviewEvent.timedFragment(start: 0.125, end: 1.625, text: String(repeating: "👩🏽‍💻", count: 2_000), isFinal: false)
        guard case let .fragment(start, end, text, isFinal) = event else { return XCTFail("missing fragment") }
        XCTAssertEqual(start, 125)
        XCTAssertEqual(end, 1_625)
        XCTAssertEqual(text.count, 1_024)
        XCTAssertFalse(isFinal)
    }

    func testTimedGateDeliversStartupFailureAndRejectsLateResults() {
        let values = LockedPreviewEvents()
        let gate = LiveSpeechEventGate { values.append($0) }
        gate.emit(.status(.warming, reason: nil))
        gate.emit(.status(.unavailable, reason: "assets_missing"))
        gate.stop()
        gate.emit(.fragment(startMS: 0, endMS: 1, text: "late", isFinal: false))
        XCTAssertEqual(values.values, [.status(.warming, reason: nil), .status(.unavailable, reason: "assets_missing")])
    }

    func testPreviewVocabularyMatchesBoundedAuthoritativeContext() {
        let vocabulary = (0..<120).map { "term\($0)" }
        XCTAssertEqual(LiveSpeechPreview.boundedVocabulary(vocabulary), Array(vocabulary.prefix(100)))
    }

    func testPhysicalAdmissionRemainsOccupiedUntilUnderlyingWorkReleasesIt() throws {
        let pool = LiveSpeechPreviewAdmission(maximum: 2)
        let first = try XCTUnwrap(pool.acquire())
        let second = try XCTUnwrap(pool.acquire())
        XCTAssertNil(pool.acquire())
        let gate = LiveSpeechEventGate { _ in }
        gate.stop()
        XCTAssertNil(pool.acquire())
        first.release()
        let replacement = try XCTUnwrap(pool.acquire())
        XCTAssertNil(pool.acquire())
        first.release()
        XCTAssertNil(pool.acquire())
        second.release()
        replacement.release()
        XCTAssertNotNil(pool.acquire())
    }

    func testReplacementWaitsOffCallerUntilRetiringPhysicalWorkerExits() async throws {
        let pool = LiveSpeechPreviewAdmission(maximum: 1)
        let retiring = try XCTUnwrap(pool.acquire())
        let waiting = Task { await pool.acquireWhenAvailable(isActive: { true }) }
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertNil(pool.acquire())
        retiring.release()
        let admitted = await waiting.value
        let replacement = try XCTUnwrap(admitted)
        XCTAssertNil(pool.acquire())
        replacement.release()
        XCTAssertNotNil(pool.acquire())
    }

    func testCancelledReplacementCannotClaimOrReleaseRetiringWorkersSlot() async throws {
        let pool = LiveSpeechPreviewAdmission(maximum: 1)
        let retiring = try XCTUnwrap(pool.acquire())
        let waiting = Task { await pool.acquireWhenAvailable(isActive: { true }) }
        waiting.cancel()
        let admitted = await waiting.value
        XCTAssertNil(admitted)
        XCTAssertNil(pool.acquire())
        retiring.release()
        XCTAssertNotNil(pool.acquire())
    }

    func testStoppingBeforePreparationRejectsAudioAndCannotRestart() {
        let buffer = LiveSpeechPreviewBuffer(maxSamples: 5)
        XCTAssertTrue(buffer.start())
        _ = buffer.append([1, 2, 3])
        buffer.stop()
        XCTAssertFalse(buffer.append([4]))
        XCTAssertNil(buffer.drain())
        XCTAssertFalse(buffer.start())
    }

    func testStoppedPreviewCanBeAwaitedBeforeStartingWithoutRequestingAccess() async {
        let values = LockedPreviewValues()
        let preview = LiveSpeechPreview(language: "nl") { values.append($0) }
        preview.stop()
        await preview.stopAndWait()
        await preview.stopAndWait()
        preview.feed([0, 0, 0])
        XCTAssertTrue(values.values.isEmpty)
    }

    func testCallbackGateRejectsUpdatesAfterStop() {
        let received = LockedPreviewValues()
        let gate = LiveSpeechCallbackGate { received.append($0) }
        gate.emit("Voor stoppen")
        gate.stop()
        gate.emit("Na stoppen")
        XCTAssertEqual(received.values, ["Voor stoppen"])
    }

    func testStopDrainsAnExecutingCallbackBeforeReturning() {
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        let stopping = DispatchSemaphore(value: 0)
        let stopped = DispatchSemaphore(value: 0)
        let gate = LiveSpeechCallbackGate { _ in
            entered.signal()
            _ = release.wait(timeout: .now() + 2)
        }
        DispatchQueue.global().async { gate.emit("Een update") }
        XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
        DispatchQueue.global().async { stopping.signal(); gate.stop(); stopped.signal() }
        XCTAssertEqual(stopping.wait(timeout: .now() + 2), .success)
        XCTAssertEqual(stopped.wait(timeout: .now() + 0.02), .timedOut)
        release.signal()
        XCTAssertEqual(stopped.wait(timeout: .now() + 2), .success)
    }

    func testAudioConversionUsesTheChosenAnalyzerFormat() throws {
        let target = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 24_000, channels: 1))
        let converter = try LiveSpeechAudioConverter(target: target)
        let samples = (0..<1600).map { Float(sin(Double($0) / 20)) }
        let result = try XCTUnwrap(converter.convert(samples))
        XCTAssertEqual(result.format, target)
        XCTAssertGreaterThan(result.frameLength, 0)
        XCTAssertLessThanOrEqual(result.frameLength, 2500)
    }
}

private final class LockedPreviewEvents: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: [LiveSpeechPreviewEvent] = []
    var values: [LiveSpeechPreviewEvent] { lock.withLock { stored } }
    func append(_ value: LiveSpeechPreviewEvent) { lock.withLock { stored.append(value) } }
}

private final class LockedPreviewValues: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: [String] = []
    var values: [String] { lock.withLock { stored } }
    func append(_ value: String) { lock.withLock { stored.append(value) } }
}
