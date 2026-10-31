import AVFoundation
import CoreMedia
import Foundation
import PennelMobileKit
import Speech

protocol DictationPreviewing: Sendable {
    func start() -> Bool
    func feed(_ samples: [Float])
    func stop()
}

/// Display-only recognition. The selected provider still owns the complete, authoritative result.
final class AppleSpeechPreview: DictationPreviewing, @unchecked Sendable {
    private let language: String?
    private let vocabulary: [String]
    private let buffer = SpeechPreviewBuffer(maxSamples: 16_000 * 5)
    private let gate: SpeechPreviewCallbackGate
    private let lock = NSLock()
    private var task: Task<Void, Never>?
    private var continuation: AsyncStream<Void>.Continuation?

    init(language: String?, vocabulary: [String], onText: @escaping @Sendable (String) -> Void) {
        self.language = language
        self.vocabulary = Array(vocabulary.prefix(100))
        gate = SpeechPreviewCallbackGate(onText)
    }

    deinit { stop() }

    func start() -> Bool {
        lock.withLock {
            guard buffer.start() else { return false }
            let input = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
            continuation = input.continuation
            let language = language
            let vocabulary = vocabulary
            let buffer = buffer
            let gate = gate
            task = Task.detached {
                await Self.run(input: input.stream, language: language, vocabulary: vocabulary, buffer: buffer, gate: gate)
            }
            return true
        }
    }

    func feed(_ samples: [Float]) {
        guard !samples.isEmpty else { return }
        lock.withLock {
            if buffer.append(samples) { continuation?.yield(()) }
        }
    }

    func stop() {
        gate.stop()
        lock.withLock {
            buffer.stop()
            continuation?.finish()
            continuation = nil
            task?.cancel()
            task = nil
        }
    }

    private static func run(
        input: AsyncStream<Void>, language: String?, vocabulary: [String], buffer: SpeechPreviewBuffer,
        gate: SpeechPreviewCallbackGate
    ) async {
        defer { buffer.stop(); gate.stop() }
        let wanted = Locale(identifier: language ?? Locale.preferredLanguages.first ?? "en")
        guard let locale = await DictationTranscriber.supportedLocale(equivalentTo: wanted),
              !Task.isCancelled, buffer.isActive else { return }
        let transcriber = DictationTranscriber(locale: locale, preset: .progressiveLongDictation)
        // Preview never starts a download; setup/final recognition owns missing-asset recovery.
        guard await AssetInventory.status(forModules: [transcriber]) == .installed,
              !Task.isCancelled, buffer.isActive else { return }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let audio = AsyncStream<AnalyzerInput>.makeStream(bufferingPolicy: .bufferingNewest(8))
        let results = Task {
            var caption = SpeechPreviewCaption()
            do {
                for try await result in transcriber.results {
                    guard !Task.isCancelled, buffer.isActive else { break }
                    if let text = caption.update(
                        start: result.range.start.seconds, end: result.range.end.seconds,
                        text: String(result.text.characters), isFinal: result.isFinal
                    ) { gate.emit(text) }
                }
            } catch {
                // Preview failure never changes the final recognition path.
            }
        }
        do {
            let context = AnalysisContext()
            context.contextualStrings[.general] = vocabulary
            try await analyzer.setContext(context)
            guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]),
                  !Task.isCancelled, buffer.isActive else { throw CancellationError() }
            let converter = try AppleSpeechPreviewAudioConverter(target: format)
            try await analyzer.prepareToAnalyze(in: format)
            try Task.checkCancellation()
            try await analyzer.start(inputSequence: audio.stream)
            for await _ in input {
                try Task.checkCancellation()
                guard buffer.isActive else { break }
                if let chunk = buffer.drain(), let converted = try converter.convert(chunk.samples) {
                    audio.continuation.yield(AnalyzerInput(
                        buffer: converted,
                        bufferStartTime: CMTime(value: chunk.startFrame, timescale: 16_000)
                    ))
                }
            }
        } catch {
            // The regular selected-provider path owns diagnostics and the final result.
        }
        buffer.stop()
        gate.stop()
        audio.continuation.finish()
        await analyzer.cancelAndFinishNow()
        results.cancel()
        await results.value
    }
}

private final class AppleSpeechPreviewAudioConverter {
    private let source: AVAudioFormat
    private let target: AVAudioFormat
    private let converter: AVAudioConverter

    init(target: AVAudioFormat) throws {
        guard let source = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1),
              let converter = AVAudioConverter(from: source, to: target) else { throw PreviewAudioError.format }
        self.source = source
        self.target = target
        self.converter = converter
        converter.primeMethod = .none
    }

    func convert(_ samples: [Float]) throws -> AVAudioPCMBuffer? {
        guard !samples.isEmpty else { return nil }
        guard let input = AVAudioPCMBuffer(pcmFormat: source, frameCapacity: AVAudioFrameCount(samples.count)),
              let channel = input.floatChannelData?[0] else { throw PreviewAudioError.format }
        input.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: samples.count) }
        let capacity = AVAudioFrameCount(ceil(Double(samples.count) * target.sampleRate / source.sampleRate)) + 256
        guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { throw PreviewAudioError.format }
        var consumed = false
        var error: NSError?
        converter.convert(to: output, error: &error) { _, status in
            guard !consumed else { status.pointee = .noDataNow; return nil }
            consumed = true
            status.pointee = .haveData
            return input
        }
        if let error { throw error }
        return output.frameLength > 0 ? output : nil
    }
}

private enum PreviewAudioError: Error { case format }
