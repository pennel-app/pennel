import CoreMedia
import Foundation
import Testing
@testable internal import PennelMobile

struct AppleSpeechTimingTests {
    @Test func only_final_results_contribute_to_authoritative_text() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "provisional", range: timingRange(0, 900), runs: [], isFinal: false)
        builder.append(text: "café", range: timingRange(125, 875), runs: [], isFinal: true)
        let result = builder.finish(audioDurationMS: 1_000)
        #expect(result.text == "café")
        #expect(result.timingKind == .segment)
        #expect(result.spans.first?.startMs == 125)
        #expect(result.spans.first?.endMs == 875)
    }

    @Test func actual_attributed_runs_preserve_exact_utf8_and_relative_clock() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: " café 👩🏽‍💻. ", range: timingRange(100, 1_200), runs: [
            AppleSpeechTimedRun(text: " café ", range: timingRange(100, 500)),
            AppleSpeechTimedRun(text: "👩🏽‍💻. ", range: timingRange(600, 1_200)),
        ], isFinal: true)
        let result = builder.finish(audioDurationMS: 2_000)
        #expect(result.text == "café 👩🏽‍💻.")
        #expect(result.timingKind == .attributed)
        #expect(result.spans.count == 2)
        #expect(result.spans.first?.startByte == 0)
        #expect(result.spans.last?.endByte == UInt32(result.text.utf8.count))
        #expect(result.spans.first?.startMs == 100)
        #expect(result.spans.last?.endMs == 1_200)
    }

    @Test func unmappable_unicode_or_punctuation_keeps_the_intact_final_result() {
        for runText in ["cafe\u{301}.", "café"] {
            var builder = AppleSpeechResultBuilder()
            builder.append(text: "café.", range: timingRange(0, 1_000), runs: [
                AppleSpeechTimedRun(text: runText, range: timingRange(0, 1_000)),
            ], isFinal: true)
            let result = builder.finish(audioDurationMS: 1_000)
            #expect(Array(result.text.utf8) == Array("café.".utf8))
            #expect(result.timingKind == .segment)
            #expect(result.spans.count == 1)
        }
    }

    @Test func zero_or_out_of_audio_range_timing_never_invents_an_utterance() {
        for range in [timingRange(0, 0), timingRange(0, 2_000)] {
            var builder = AppleSpeechResultBuilder()
            builder.append(text: "Nog niet beslist.", range: range, runs: [], isFinal: true)
            let result = builder.finish(audioDurationMS: 1_000)
            #expect(result.text == "Nog niet beslist.")
            #expect(result.timingKind == .untimed)
            #expect(result.spans.isEmpty)
        }
    }

    @Test func malformed_finer_run_keeps_both_final_results_with_real_coarse_timing() {
        var builder = AppleSpeechResultBuilder()
        builder.append(text: "Not yet. ", range: timingRange(100, 700), runs: [], isFinal: true)
        builder.append(text: "Nog niet.", range: timingRange(800, 1_400), runs: [
            AppleSpeechTimedRun(text: "Nog niet.", range: timingRange(1_600, 1_800)),
        ], isFinal: true)
        let result = builder.finish(audioDurationMS: 2_000)
        #expect(result.text == "Not yet. Nog niet.")
        #expect(result.timingKind == .segment)
        #expect(result.spans.count == 2)
        #expect(result.spans.first?.startMs == 100)
        #expect(result.spans.last?.endMs == 1_400)
    }

    @Test func timed_capability_is_additive_to_existing_text_host() {
        let host = AppleSpeechTranscriber()
        let _: any SystemTranscriber = host
        let _: any TimedSystemTranscriber = host
    }
}

private func timingRange(_ startMS: Int64, _ endMS: Int64) -> CMTimeRange {
    let start = CMTime(value: startMS, timescale: 1_000)
    let end = CMTime(value: endMS, timescale: 1_000)
    return CMTimeRange(start: start, duration: end - start)
}
