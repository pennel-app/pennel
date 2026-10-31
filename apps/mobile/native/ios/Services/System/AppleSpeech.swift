// Port of apps/macos-platform/Sources/PennelMacPlatform/AppleSpeech.swift. Keep behavior and canonical English diagnostics in step with it.
import AVFoundation
import Foundation
import Speech

/// Apple's on-device dictation model (iOS 26+) as a Pennel speech provider: free, offline,
/// nothing to download beyond the language assets iOS manages itself.
final class AppleSpeechTranscriber: SystemTranscriber, @unchecked Sendable {
    func transcribe(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> String {
        do {
            return try transcribeBlocking(samples: samples, sampleRate: sampleRate, language: language, vocabulary: vocabulary)
        } catch let error as PennelError {
            throw error
        } catch {
            throw PennelError.InvalidResponse(message: error.localizedDescription)
        }
    }

    private func transcribeBlocking(samples: [Float], sampleRate: UInt32, language: String?, vocabulary: [String]) throws -> String {
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

    private static func run(samples: [Float], sampleRate: Double, language: String?, vocabulary: [String]) async throws -> String {
        let transcriber = DictationTranscriber(locale: try await locale(for: language), preset: .longDictation)
        if let install = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
            try await install.downloadAndInstall()
        }
        let file = try writeTemporaryFile(samples, sampleRate: sampleRate)
        defer { try? FileManager.default.removeItem(at: file.url) }

        async let text = transcriber.results.reduce(into: "") { text, result in
            if result.isFinal { text += String(result.text.characters) }
        }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        try await analyzer.setContext(analysisContext(vocabulary: vocabulary))
        try await analyzer.start(inputAudioFile: file, finishAfterFile: true)
        return try await text.trimmingCharacters(in: .whitespacesAndNewlines)
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
        var result: Result<String, Error>?
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
