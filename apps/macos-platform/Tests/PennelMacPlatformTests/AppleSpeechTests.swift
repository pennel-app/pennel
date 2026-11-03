import Foundation
import CoreMedia
import XCTest
@testable import PennelMacPlatform

final class AppleSpeechTests: XCTestCase {
    func testVocabularyBridgeRejectsMalformedDataWithoutRecognizingOrEchoingIt() {
        let receipt = SpeechReceipt()
        let context = Unmanaged.passUnretained(receipt).toOpaque()
        let samples: [Float] = [0.2]
        samples.withUnsafeBufferPointer { audio in
            #"{"private vocabulary":"not a word list"}"#.withCString { vocabulary in
                transcribe(audio.baseAddress, audio.count, 16_000, nil, vocabulary, speechReceipt, context)
            }
        }
        XCTAssertEqual(receipt.calls, 1)
        XCTAssertNil(receipt.text)
        XCTAssertEqual(receipt.error, "Invalid speech vocabulary")
    }

    func testEmptyAudioBridgeFailsBeforeAnyModelOrPermissionWork() {
        let receipt = SpeechReceipt()
        transcribe(nil, 0, 16_000, nil, nil, speechReceipt, Unmanaged.passUnretained(receipt).toOpaque())
        XCTAssertEqual(receipt.calls, 1)
        XCTAssertNil(receipt.text)
        XCTAssertEqual(receipt.error, "No usable audio")
    }

    func testTimedBuilderOnlyIncludesFinalResultsAndPreservesUTF8Offsets() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "old provisional", range: speechRange(0, 0.5), runs: [], isFinal: false)
        builder.append(text: " café 👩🏽‍💻. ", range: speechRange(0.1, 1.2), runs: [
            AppleSpeechTimedRun(text: " café ", range: speechRange(0.1, 0.5)),
            AppleSpeechTimedRun(text: "👩🏽‍💻. ", range: speechRange(0.6, 1.2)),
        ], isFinal: true)
        let transcript = builder.finish(audioDurationMS: 2_000)
        XCTAssertEqual(transcript.text, "café 👩🏽‍💻.")
        XCTAssertEqual(transcript.timingKind, "attributed")
        XCTAssertEqual(transcript.spans.count, 2)
        XCTAssertEqual(transcript.spans.first?.startByte, 0)
        XCTAssertEqual(transcript.spans.last?.endByte, UInt32(transcript.text.utf8.count))
        XCTAssertEqual(transcript.spans.first?.startMS, 100)
        XCTAssertEqual(transcript.spans.last?.endMS, 1_200)
    }

    func testUnmappableRunsUseWholeFinalResultWithoutInventedAlignment() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "Hallo, café.", range: speechRange(0, 1), runs: [
            AppleSpeechTimedRun(text: "Hallo café", range: speechRange(0, 1))
        ], isFinal: true)
        let transcript = builder.finish(audioDurationMS: 1_000)
        XCTAssertEqual(transcript.text, "Hallo, café.")
        XCTAssertEqual(transcript.timingKind, "segment")
        XCTAssertEqual(transcript.spans.count, 1)
        XCTAssertEqual(transcript.spans[0].endByte, UInt32(transcript.text.utf8.count))
    }

    func testMalformedTimesKeepRecognizedTextWithoutTiming() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "Een volledige zin.", range: speechRange(0, 20), runs: [], isFinal: true)
        let transcript = builder.finish(audioDurationMS: 1_000)
        XCTAssertEqual(transcript.text, "Een volledige zin.")
        XCTAssertEqual(transcript.timingKind, "untimed")
        XCTAssertTrue(transcript.spans.isEmpty)
    }

    func testZeroDurationTextCannotBecomeAnAttributedInstant() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "café", range: speechRange(0, 0), runs: [], isFinal: true)
        let transcript = builder.finish(audioDurationMS: 1_000)
        XCTAssertEqual(transcript.text, "café")
        XCTAssertEqual(transcript.timingKind, "untimed")
        XCTAssertTrue(transcript.spans.isEmpty)
    }

    func testRunsOutsideFinalResultRangeFallBackToTheActualFinalRange() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "café", range: speechRange(0, 1), runs: [
            AppleSpeechTimedRun(text: "café", range: speechRange(1.5, 1.9)),
        ], isFinal: true)
        let transcript = builder.finish(audioDurationMS: 2_000)
        XCTAssertEqual(transcript.text, "café")
        XCTAssertEqual(transcript.timingKind, "segment")
        XCTAssertEqual(transcript.spans[0].startMS, 0)
        XCTAssertEqual(transcript.spans[0].endMS, 1_000)
    }

    func testCanonicallyEquivalentButDifferentRunBytesUseIntactResult() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "café.", range: speechRange(0, 1), runs: [
            AppleSpeechTimedRun(text: "cafe\u{301}.", range: speechRange(0, 1)),
        ], isFinal: true)
        let transcript = builder.finish(audioDurationMS: 1_000)
        XCTAssertEqual(Array(transcript.text.utf8), Array("café.".utf8))
        XCTAssertEqual(transcript.timingKind, "segment")
        XCTAssertEqual(transcript.spans.count, 1)
    }
}

private func speechRange(_ start: Double, _ end: Double) -> CMTimeRange {
    let start = CMTime(value: Int64((start * 1_000).rounded()), timescale: 1_000)
    let end = CMTime(value: Int64((end * 1_000).rounded()), timescale: 1_000)
    return CMTimeRange(start: start, duration: end - start)
}

private final class SpeechReceipt {
    var calls = 0
    var text: String?
    var error: String?
}

private func speechReceipt(_ raw: UnsafeMutableRawPointer?, _ text: UnsafePointer<CChar>?, _ error: UnsafePointer<CChar>?) {
    guard let raw else { return }
    let receipt = Unmanaged<SpeechReceipt>.fromOpaque(raw).takeUnretainedValue()
    receipt.calls += 1
    receipt.text = text.map { String(cString: $0) }
    receipt.error = error.map { String(cString: $0) }
}
