// Port of apps/macos-platform/Sources/PennelMacPlatform/AppleSpeech.swift. Keep behavior and canonical English diagnostics in step with it.
import AVFoundation
import CoreMedia
import Foundation
import Speech

/// Apple's on-device dictation model (iOS 26+) as a Pennel speech provider: free, offline,
/// nothing to download beyond the language assets iOS manages itself.
final class AppleSpeechTranscriber: SystemTranscriber, TimedSystemTranscriber, @unchecked Sendable {
    func transcribe(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> String {
        try transcribeTimed(samples: samples, sampleRate: sampleRate, language: language, vocabulary: vocabulary).text
    }

    func transcribeTimed(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> SpeechTranscript {
        do {
            return try transcribeBlocking(samples: samples, sampleRate: sampleRate, language: language, vocabulary: vocabulary)
        } catch let error as PennelError {
            throw error
        } catch {
            throw PennelError.InvalidResponse(message: error.localizedDescription)
        }
    }

    private func transcribeBlocking(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> SpeechTranscript {
        guard sampleRate > 0 else { throw NativeSpeechError(message: "audio") }
        let box = ResultBox()
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            box.result = await Result { try await Self.run(samples: samples, sampleRate: Double(sampleRate), language: language, vocabulary: vocabulary) }
            done.signal()
        }
        done.wait()
        return try box.result!.get()
    }

    static func requestedLocale(language: String?, preferredLanguages: [String] = Locale.preferredLanguages) -> Locale {
        Locale(identifier: language ?? preferredLanguages.first ?? "en")
    }

    private static func locale(for language: String?) async throws -> Locale {
        let wanted = requestedLocale(language: language)
        guard let supported = await DictationTranscriber.supportedLocale(equivalentTo: wanted) else {
            throw NativeSpeechError(message: "Apple Dictation does not support \(wanted.identifier)")
        }
        return supported
    }

    private static func run(samples: [Float], sampleRate: Double, language: String?, vocabulary: [String]) async throws -> SpeechTranscript {
        var preset = DictationTranscriber.Preset.longDictation
        preset.attributeOptions.insert(.audioTimeRange)
        let transcriber = DictationTranscriber(locale: try await locale(for: language), preset: preset)
        if let install = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
            try await install.downloadAndInstall()
        }
        let file = try writeTemporaryFile(samples, sampleRate: sampleRate)
        defer { try? FileManager.default.removeItem(at: file.url) }

        async let builder = transcriber.results.reduce(into: AppleSpeechResultBuilder()) { builder, result in
            let text = result.text
            let runs = text.runs.map { run in
                AppleSpeechTimedRun(text: String(text[run.range].characters), range: run.audioTimeRange)
            }
            builder.append(text: String(text.characters), range: result.range, runs: runs, isFinal: result.isFinal)
        }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        try await analyzer.setContext(analysisContext(vocabulary: vocabulary))
        try await analyzer.start(inputAudioFile: file, finishAfterFile: true)
        return try await builder.finish(audioDurationMS: UInt64(ceil(Double(samples.count) * 1_000 / sampleRate)))
    }

    static func analysisContext(vocabulary: [String]) -> AnalysisContext {
        let context = AnalysisContext()
        context.contextualStrings[.general] = Array(vocabulary.prefix(100))
        return context
    }

    /// Capability probe only; a supported locale does not establish bilingual recognition quality.
    static func speechTranscriberExperimentAvailable(language: String?) async -> Bool {
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: requestedLocale(language: language)) else { return false }
        return await AssetInventory.status(forModules: [SpeechTranscriber(locale: locale, preset: .transcription)]) == .installed
    }

    private static func writeTemporaryFile(_ samples: [Float], sampleRate: Double) throws -> AVAudioFile {
        guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count))
        else { throw NativeSpeechError(message: "audio") }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { buffer.floatChannelData![0].update(from: $0.baseAddress!, count: samples.count) }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-\(UUID().uuidString).caf")
        let writer = try AVAudioFile(forWriting: url, settings: format.settings)
        try writer.write(from: buffer)
        return try AVAudioFile(forReading: url)
    }

    private final class ResultBox: @unchecked Sendable {
        var result: Result<SpeechTranscript, Error>?
    }
}

extension Result where Failure == Error {
    init(catching body: () async throws -> Success) async {
        do { self = .success(try await body()) } catch { self = .failure(error) }
    }
}

struct NativeSpeechError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}


struct AppleSpeechTimedRun {
    let text: String
    let range: CMTimeRange?
}

struct AppleSpeechResultBuilder {
    private var text = ""
    private var spans: [SpeechTextSpan] = []
    private var reliable = true
    private var attributed = true

    mutating func append(text part: String, range: CMTimeRange, runs: [AppleSpeechTimedRun], isFinal: Bool) {
        guard isFinal, !part.isEmpty else { return }
        let base = text.utf8.count
        text += part
        guard let finalByte = UInt32(exactly: text.utf8.count) else { reliable = false; return }
        let parts: [AppleSpeechTimedRun]
        let bounds = Self.milliseconds(range)
        var previousStart: UInt64 = 0
        let validRunTimes = runs.allSatisfy { run in
            guard let bounds, let time = run.range.flatMap(Self.milliseconds),
                  time.0 >= bounds.0, time.1 <= bounds.1, time.0 >= previousStart else { return false }
            previousStart = time.0
            return true
        }
        if !runs.isEmpty, runs.map(\.text).joined().utf8.elementsEqual(part.utf8),
           validRunTimes, runs.allSatisfy({ !$0.text.isEmpty }) {
            parts = runs
        } else {
            parts = [AppleSpeechTimedRun(text: part, range: range)]
            attributed = false
        }
        var offset = base
        for part in parts {
            guard let range = part.range, let time = Self.milliseconds(range),
                  let start = UInt32(exactly: offset), let end = UInt32(exactly: offset + part.text.utf8.count)
            else { reliable = false; return }
            spans.append(SpeechTextSpan(startByte: start, endByte: end, startMs: time.0, endMs: time.1))
            offset += part.text.utf8.count
        }
        if offset != Int(finalByte) { reliable = false }
    }

    func finish(audioDurationMS: UInt64) -> SpeechTranscript {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let plain = SpeechTranscript(text: trimmed, spans: [], timingKind: .untimed)
        guard reliable, !trimmed.isEmpty, !spans.isEmpty,
              let selected = text.range(of: trimmed) else { return plain }
        var cursor: UInt32 = 0
        var previousStart: UInt64 = 0
        for span in spans {
            guard span.startByte == cursor, span.endByte > span.startByte,
                  span.startMs < span.endMs, span.endMs <= audioDurationMS,
                  span.startMs >= previousStart else { return plain }
            cursor = span.endByte
            previousStart = span.startMs
        }
        guard Int(cursor) == text.utf8.count else { return plain }
        let left = text[..<selected.lowerBound].utf8.count
        let right = left + trimmed.utf8.count
        let trimmedSpans = spans.compactMap { span -> SpeechTextSpan? in
            let start = max(Int(span.startByte), left)
            let end = min(Int(span.endByte), right)
            guard end > start else { return nil }
            return SpeechTextSpan(startByte: UInt32(start - left), endByte: UInt32(end - left), startMs: span.startMs, endMs: span.endMs)
        }
        return SpeechTranscript(text: trimmed, spans: trimmedSpans, timingKind: attributed ? .attributed : .segment)
    }

    private static func milliseconds(_ range: CMTimeRange) -> (UInt64, UInt64)? {
        let start = range.start.seconds
        let end = range.end.seconds
        guard start.isFinite, end.isFinite, start >= 0, end > start,
              end * 1_000 < Double(UInt64.max) else { return nil }
        return (UInt64((start * 1_000).rounded()), UInt64((end * 1_000).rounded()))
    }
}
