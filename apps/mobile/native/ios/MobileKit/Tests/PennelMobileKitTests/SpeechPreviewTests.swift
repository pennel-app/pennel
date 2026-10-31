import Foundation
import Testing
@testable import PennelMobileKit

struct SpeechPreviewTests {
    @Test func provisional_revisions_replace_each_other_and_final_text_cannot_regress() {
        var caption = SpeechPreviewCaption(maxCharacters: 100)
        #expect(caption.update(start: 0, end: 1, text: "Ik wel", isFinal: false) == "Ik wel")
        #expect(caption.update(start: 0, end: 1, text: "Ik wil", isFinal: false) == "Ik wil")
        #expect(caption.update(start: 0, end: 1, text: "Ik wil", isFinal: true) == nil)
        #expect(caption.update(start: 0, end: 1, text: "Ik wel", isFinal: false) == nil)
        #expect(caption.update(start: 1, end: 2, text: "English erbij", isFinal: false) == "Ik wil English erbij")
        #expect(caption.update(start: .nan, end: 2, text: "invalid", isFinal: false) == nil)
    }

    @Test func preview_buffer_drops_idle_audio_bounds_backlog_and_stops_permanently() throws {
        let buffer = SpeechPreviewBuffer(maxSamples: 4)
        #expect(buffer.append([1, 2]) == false)
        #expect(buffer.start())
        #expect(buffer.append([1, 2, 3]))
        #expect(buffer.append([4, 5, 6]))
        let chunk = try #require(buffer.drain())
        #expect(chunk.samples == [3, 4, 5, 6])
        #expect(chunk.startFrame == 2)
        #expect(buffer.drain() == nil)
        buffer.stop()
        #expect(buffer.append([7]) == false)
        #expect(buffer.start() == false)
        #expect(buffer.drain() == nil)
    }

    @Test func preview_callback_is_silent_after_stop_even_with_a_late_result() {
        let values = PreviewValues()
        let gate = SpeechPreviewCallbackGate { values.append($0) }
        gate.emit("voorlopig")
        gate.stop()
        gate.emit("te laat")
        #expect(values.snapshot == ["voorlopig"])
    }
}

private final class PreviewValues: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String] = []
    var snapshot: [String] { lock.withLock { values } }
    func append(_ value: String) { lock.withLock { values.append(value) } }
}
