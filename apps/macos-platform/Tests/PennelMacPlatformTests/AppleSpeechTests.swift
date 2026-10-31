import Foundation
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
