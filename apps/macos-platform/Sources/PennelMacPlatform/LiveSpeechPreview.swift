import AVFoundation
import CoreMedia
import Foundation
import Speech

enum LiveSpeechPreviewStatus: String, Codable, Sendable {
    case warming, ready, unavailable, failed
}

enum LiveSpeechPreviewEvent: Encodable, Equatable, Sendable {
    case status(LiveSpeechPreviewStatus, reason: String?)
    case fragment(startMS: UInt64, endMS: UInt64, text: String, isFinal: Bool)

    private enum CodingKeys: String, CodingKey {
        case kind, status, reason, text
        case startMS = "start_ms", endMS = "end_ms", isFinal = "is_final"
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .status(status, reason):
            try value.encode("status", forKey: .kind)
            try value.encode(status, forKey: .status)
            try value.encodeIfPresent(reason, forKey: .reason)
        case let .fragment(start, end, text, isFinal):
            try value.encode("fragment", forKey: .kind)
            try value.encode(start, forKey: .startMS)
            try value.encode(end, forKey: .endMS)
            try value.encode(text, forKey: .text)
            try value.encode(isFinal, forKey: .isFinal)
        }
    }

    static func timedFragment(start: TimeInterval, end: TimeInterval, text: String, isFinal: Bool) -> Self? {
        guard start.isFinite, end.isFinite, start >= 0, end >= start,
              end * 1_000 < Double(UInt64.max),
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return .fragment(startMS: UInt64(start * 1_000), endMS: UInt64((end * 1_000).rounded(.up)),
                         text: String(text.suffix(1_024)), isFinal: isFinal)
    }
}

final class LiveSpeechPreview: @unchecked Sendable {
    private static let admission = LiveSpeechPreviewAdmission(maximum: 2)
    private let language: String?
    private let vocabulary: [String]
    private let buffer = LiveSpeechPreviewBuffer(maxSamples: 16_000 * 5)
    private let gate: LiveSpeechEventGate
    private let lock = NSLock()
    private var task: Task<Void, Never>?
    private var continuation: AsyncStream<Void>.Continuation?

    convenience init(language: String?, onText: @escaping @Sendable (String) -> Void) {
        let projection = LiveSpeechLegacyProjection(onText: onText)
        self.init(language: language, vocabulary: [], onEvent: { projection.receive($0) })
    }

    init(language: String?, vocabulary: [String], onEvent: @escaping @Sendable (LiveSpeechPreviewEvent) -> Void) {
        self.language = language
        self.vocabulary = Self.boundedVocabulary(vocabulary)
        gate = LiveSpeechEventGate(onEvent)
    }

    static func boundedVocabulary(_ vocabulary: [String]) -> [String] { Array(vocabulary.prefix(100)) }

    deinit { stop() }

    @discardableResult
    func start() -> Bool {
        guard #available(macOS 26, *) else {
            gate.emit(.status(.unavailable, reason: "unsupported_platform"))
            return false
        }
        guard SFSpeechRecognizer.authorizationStatus() == .authorized else {
            gate.emit(.status(.unavailable, reason: "permission_required"))
            return false
        }
        return lock.withLock {
            guard buffer.start() else { return false }
            let input = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
            continuation = input.continuation
            let language = language
            let vocabulary = vocabulary
            let buffer = buffer
            let gate = gate
            gate.emit(.status(.warming, reason: nil))
            task = Task.detached {
                await Self.run(input: input.stream, finishInput: { input.continuation.finish() },
                               language: language, vocabulary: vocabulary, buffer: buffer, gate: gate)
            }
            return true
        }
    }

    func feed(_ samples: [Float], startMS: UInt64? = nil) {
        guard !samples.isEmpty else { return }
        let startFrame: Int64?
        if let startMS {
            guard startMS <= UInt64(Int64.max / 16) else { return }
            startFrame = Int64(startMS) * 16
        } else { startFrame = nil }
        lock.withLock {
            if buffer.append(samples, startFrame: startFrame) { continuation?.yield(()) }
        }
    }

    func stop() {
        // Drain the foreign callback before Rust frees its context; analyzer teardown is asynchronous.
        gate.stop()
        lock.withLock {
            buffer.stop()
            continuation?.finish()
            continuation = nil
            task?.cancel()
        }
    }

    func stopAndWait() async {
        stop()
        let running = lock.withLock { task }
        await running?.value
        lock.withLock { task = nil }
    }

    @available(macOS 26, *)
    private static func run(
        input: AsyncStream<Void>, finishInput: @escaping @Sendable () -> Void,
        language: String?, vocabulary: [String], buffer: LiveSpeechPreviewBuffer,
        gate: LiveSpeechEventGate
    ) async {
        defer { buffer.stop(); gate.stop() }
        guard let lease = await admission.acquireWhenAvailable(isActive: { buffer.isActive }) else {
            if !Task.isCancelled, buffer.isActive { gate.emit(.status(.unavailable, reason: "busy")) }
            return
        }
        defer { lease.release() }
        guard !Task.isCancelled, buffer.isActive else { return }
        let wanted = Locale(identifier: language ?? Locale.preferredLanguages.first ?? "en")
        guard let locale = await DictationTranscriber.supportedLocale(equivalentTo: wanted) else {
            gate.emit(.status(.unavailable, reason: "unsupported_locale"))
            return
        }
        guard !Task.isCancelled, buffer.isActive else { return }
        let transcriber = DictationTranscriber(locale: locale, preset: .progressiveLongDictation)
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else {
            gate.emit(.status(.unavailable, reason: "assets_missing"))
            return
        }
        guard !Task.isCancelled, buffer.isActive else { return }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let audio = AsyncStream<AnalyzerInput>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let results = Task {
            do {
                for try await result in transcriber.results {
                    guard !Task.isCancelled, buffer.isActive else { break }
                    if let event = LiveSpeechPreviewEvent.timedFragment(
                        start: result.range.start.seconds, end: result.range.end.seconds,
                        text: String(result.text.characters), isFinal: result.isFinal
                    ) { gate.emit(event) }
                }
                if !Task.isCancelled, buffer.isActive {
                    gate.emit(.status(.failed, reason: "recognition_finished"))
                    buffer.stop()
                    finishInput()
                }
            } catch {
                if !Task.isCancelled, buffer.isActive {
                    gate.emit(.status(.failed, reason: "recognition_failed"))
                    buffer.stop()
                    finishInput()
                }
            }
        }
        var prepared = false
        do {
            guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]),
                  !Task.isCancelled, buffer.isActive else { throw CancellationError() }
            let converter = try LiveSpeechAudioConverter(target: format)
            let context = AnalysisContext()
            context.contextualStrings[.general] = vocabulary
            try await analyzer.setContext(context)
            try await analyzer.prepareToAnalyze(in: format)
            try Task.checkCancellation()
            try await analyzer.start(inputSequence: audio.stream)
            prepared = true
            gate.emit(.status(.ready, reason: nil))
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
            audio.continuation.finish()
            if !Task.isCancelled, buffer.isActive {
                try await analyzer.finalizeAndFinishThroughEndOfInput()
            }
        } catch {
            if !Task.isCancelled, buffer.isActive {
                gate.emit(.status(.failed, reason: prepared ? "analysis_failed" : "preparation_failed"))
            }
        }
        buffer.stop()
        gate.stop()
        audio.continuation.finish()
        await analyzer.cancelAndFinishNow()
        results.cancel()
        await results.value
    }
}

private final class LiveSpeechLegacyProjection: @unchecked Sendable {
    private let lock = NSLock()
    private var reducer = LiveSpeechCaptionReducer()
    private let onText: @Sendable (String) -> Void

    init(onText: @escaping @Sendable (String) -> Void) { self.onText = onText }

    func receive(_ event: LiveSpeechPreviewEvent) {
        guard case let .fragment(start, end, text, isFinal) = event else { return }
        lock.withLock {
            if let text = reducer.update(start: Double(start) / 1_000, end: Double(end) / 1_000, text: text, isFinal: isFinal) {
                onText(text)
            }
        }
    }
}

final class LiveSpeechEventGate: @unchecked Sendable {
    private let lock = NSRecursiveLock()
    private var callback: (@Sendable (LiveSpeechPreviewEvent) -> Void)?

    init(_ callback: @escaping @Sendable (LiveSpeechPreviewEvent) -> Void) { self.callback = callback }
    func emit(_ event: LiveSpeechPreviewEvent) { lock.withLock { callback?(event) } }
    func stop() { lock.withLock { callback = nil } }
}

final class LiveSpeechPreviewAdmission: @unchecked Sendable {
    private let lock = NSLock()
    private let maximum: Int
    private var active = 0

    init(maximum: Int) { self.maximum = max(1, maximum) }

    func acquire() -> LiveSpeechPreviewLease? {
        lock.withLock {
            guard active < maximum else { return nil }
            active += 1
            return LiveSpeechPreviewLease { [self] in lock.withLock { active -= 1 } }
        }
    }

    func acquireWhenAvailable(isActive: @escaping @Sendable () -> Bool) async -> LiveSpeechPreviewLease? {
        for attempt in 0...50 {
            guard !Task.isCancelled, isActive() else { return nil }
            if let lease = acquire() { return lease }
            if attempt < 50 {
                do { try await Task.sleep(for: .milliseconds(20)) } catch { return nil }
            }
        }
        return nil
    }
}

final class LiveSpeechPreviewLease: @unchecked Sendable {
    private let lock = NSLock()
    private var onRelease: (@Sendable () -> Void)?

    init(_ onRelease: @escaping @Sendable () -> Void) { self.onRelease = onRelease }
    func release() {
        let action = lock.withLock { let action = onRelease; onRelease = nil; return action }
        action?()
    }
    deinit { release() }
}

struct LiveSpeechCaptionReducer {
    private struct Fragment {
        let start: TimeInterval
        let end: TimeInterval
        let text: String
        let isFinal: Bool
    }
    private let maxCharacters: Int
    private var fragments: [Fragment] = []
    private var previous = ""

    init(maxCharacters: Int = 1_024) { self.maxCharacters = max(1, maxCharacters) }

    var text: String {
        var joined = ""
        for fragment in fragments {
            let part = fragment.text.trimmingCharacters(in: .whitespacesAndNewlines)
            if !joined.isEmpty, let first = part.first, !".,!?;:)]}".contains(first) { joined += " " }
            joined += part
        }
        return String(joined.suffix(maxCharacters))
    }

    mutating func update(start: TimeInterval, end: TimeInterval, text: String, isFinal: Bool) -> String? {
        guard start.isFinite, end.isFinite, start >= 0, end >= start,
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        let incoming = Fragment(start: start, end: end, text: String(text.suffix(maxCharacters)), isFinal: isFinal)
        func overlaps(_ fragment: Fragment) -> Bool {
            abs(fragment.start - start) < 0.000_001 || (fragment.start < end && start < fragment.end)
        }
        if !isFinal, fragments.contains(where: { $0.isFinal && overlaps($0) }) { return nil }
        fragments.removeAll { overlaps($0) }
        fragments.append(incoming)
        fragments.sort { $0.start < $1.start }
        if fragments.count > 64 { fragments.removeFirst(fragments.count - 64) }
        let visible = self.text
        guard visible != previous else { return nil }
        previous = visible
        return visible
    }
}

struct LiveSpeechPCMChunk: Sendable {
    let samples: [Float]
    let startFrame: Int64
}

final class LiveSpeechPreviewBuffer: @unchecked Sendable {
    private enum Phase { case waiting, running, stopped }
    private let lock = NSLock()
    private let maxSamples: Int
    private var phase = Phase.waiting
    private var pending: [Float] = []
    private var frames: Int64 = 0

    init(maxSamples: Int) { self.maxSamples = max(1, maxSamples) }
    var isActive: Bool { lock.withLock { phase == .running } }

    func start() -> Bool {
        lock.withLock {
            guard phase == .waiting else { return false }
            phase = .running
            return true
        }
    }

    func append(_ samples: [Float], startFrame: Int64? = nil) -> Bool {
        lock.withLock {
            guard phase == .running, !samples.isEmpty else { return false }
            let start = startFrame ?? frames
            guard start >= 0, start <= Int64.max - Int64(samples.count) else { return false }
            let skipped = min(samples.count, Int(max(0, frames - start)))
            guard skipped < samples.count else { return false }
            if start > frames { pending.removeAll(keepingCapacity: true) }
            let samples = Array(samples.dropFirst(skipped))
            frames = start + Int64(skipped) + Int64(samples.count)
            if samples.count >= maxSamples { pending = Array(samples.suffix(maxSamples)) }
            else {
                let excess = max(0, pending.count + samples.count - maxSamples)
                if excess > 0 { pending.removeFirst(excess) }
                pending.append(contentsOf: samples)
            }
            return true
        }
    }

    func drain() -> LiveSpeechPCMChunk? {
        lock.withLock {
            guard phase == .running, !pending.isEmpty else { return nil }
            let chunk = LiveSpeechPCMChunk(samples: pending, startFrame: frames - Int64(pending.count))
            pending.removeAll(keepingCapacity: true)
            return chunk
        }
    }

    func stop() { lock.withLock { phase = .stopped; pending.removeAll(keepingCapacity: false) } }
}

final class LiveSpeechCallbackGate: @unchecked Sendable {
    private let lock = NSRecursiveLock()
    private var callback: (@Sendable (String) -> Void)?

    init(_ callback: @escaping @Sendable (String) -> Void) { self.callback = callback }
    func emit(_ text: String) { lock.withLock { callback?(text) } }
    func stop() { lock.withLock { callback = nil } }
}

final class LiveSpeechAudioConverter {
    private let source: AVAudioFormat
    private let target: AVAudioFormat
    private let converter: AVAudioConverter

    init(target: AVAudioFormat) throws {
        guard let source = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1),
              let converter = AVAudioConverter(from: source, to: target) else { throw LiveSpeechAudioError.format }
        self.source = source
        self.target = target
        self.converter = converter
        converter.primeMethod = .none
    }

    func convert(_ samples: [Float]) throws -> AVAudioPCMBuffer? {
        guard !samples.isEmpty else { return nil }
        guard let input = AVAudioPCMBuffer(pcmFormat: source, frameCapacity: AVAudioFrameCount(samples.count)),
              let channel = input.floatChannelData?[0] else { throw LiveSpeechAudioError.format }
        input.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: samples.count) }
        let capacity = AVAudioFrameCount(ceil(Double(samples.count) * target.sampleRate / source.sampleRate)) + 256
        guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { throw LiveSpeechAudioError.format }
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

private enum LiveSpeechAudioError: Error { case format }
