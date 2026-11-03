import AVFoundation
import PennelMobileKit

protocol MeetingAudioSink: AnyObject, Sendable {
    func push(startMs: UInt64, samples: [Float]) throws
}

/// A physical meeting is one microphone track, written as the existing "Anderen" track (spec §4.1).
final class OthersTrack: MeetingAudioSink, @unchecked Sendable {
    private let session: MeetingSession

    init(session: MeetingSession) {
        self.session = session
    }

    func push(startMs: UInt64, samples: [Float]) throws {
        try session.pushAudioAt(speaker: .others, startMs: startMs, samples: samples)
    }
}

protocol MeetingArchiving: AnyObject {
    func write(_ samples: [Float]) throws
    func close()
}

/// AAC, 16 kHz mono. The shared core chooses the archive's physical source and path.
final class AACArchive: MeetingArchiving {
    private var file: AVAudioFile?

    init(url: URL) throws {
        file = try AVAudioFile(forWriting: url, settings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
            AVEncoderBitRateKey: 32_000,
        ], commonFormat: .pcmFormatFloat32, interleaved: false)
    }

    func write(_ samples: [Float]) throws {
        guard let file, !samples.isEmpty else { return }
        guard let buffer = AVAudioPCMBuffer(pcmFormat: MonoResampler.format, frameCapacity: AVAudioFrameCount(samples.count)),
              let data = buffer.floatChannelData
        else { throw MicCapture.Failure.unavailable }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { source in
            if let base = source.baseAddress { data[0].update(from: base, count: samples.count) }
        }
        try file.write(from: buffer)
    }

    func close() {
        file = nil
    }
}

extension PennelEngine {
    func meetingArchiveURL(id: String, speaker: Speaker, sourceID: String? = nil) -> URL? {
        guard let track = meetingAudioTracks(id: id).first(where: {
            guard $0.speaker == speaker && $0.available else { return false }
            guard let sourceID else { return true }
            let physical: String = switch $0.source {
            case .microphone: "microphone"
            case .systemAudio: "system_audio"
            case .roomMicrophone: "room_microphone"
            }
            return physical == sourceID || (physical == "system_audio" && sourceID == "system")
        }) else { return nil }
        let root = URL(fileURLWithPath: meetingDir(id: id)).resolvingSymlinksInPath()
        let url = root.appendingPathComponent(track.relativePath).resolvingSymlinksInPath()
        guard url.path.hasPrefix(root.path + "/"),
              (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
        else { return nil }
        return url
    }
}

/// A stretch of the meeting timeline without audio, in milliseconds since the start. `endMs` is nil when the
/// recording never resumed.
struct MeetingGap: Codable, Hashable, Sendable {
    var startMs: UInt64
    var endMs: UInt64?
}

struct MeetingRecorderMetrics: Equatable, Sendable {
    var outputSamples: UInt64
    var insertedSilenceSamples: UInt64
    var gapCount: UInt64
}

/// Places microphone samples on the meeting timeline and hands them to the core and the archive.
///
/// The position follows the samples while capture runs and jumps to the wall clock after a pause, so an
/// interruption leaves a gap (`push_audio_at`) instead of shifting later speech earlier. Blocks go to the core in
/// whole seconds that start on a millisecond, because the core rounds a start time down to the millisecond and
/// would otherwise drop the overlap.
final class MeetingRecorder: @unchecked Sendable {
    private static let rate = 16_000
    private static let samplesPerMs = 16
    /// Callback jitter below this stays contiguous; only real holes in the capture become gaps.
    private static let jitterSamples = 1_600
    private static let silenceBlock = 32_000
    /// A hole this long without a pause is a stall (the engine died), not jitter: it counts as an interruption.
    static let stallSeconds: TimeInterval = 1

    private let sink: MeetingAudioSink
    private let archive: MeetingArchiving?
    private let clock: @Sendable () -> TimeInterval
    private let queue = DispatchQueue(label: "app.pennel.ios.meeting-recorder")
    private let lock = NSLock()

    private var startedAt: TimeInterval = 0
    private var accepting = false
    private var dictating = false
    private var resumed = false
    private var lastSampleAt: TimeInterval = 0
    private var recordedGaps: [MeetingGap] = []
    private var failure: String?

    // Owned by `queue`.
    private var pending: [Float] = []
    private var pendingStart = 0
    private var archived = 0
    private var insertedSilence = 0
    private var paddingGaps: UInt64 = 0

    init(sink: MeetingAudioSink, archive: MeetingArchiving?, clock: @escaping @Sendable () -> TimeInterval) {
        self.sink = sink
        self.archive = archive
        self.clock = clock
    }

    func begin() {
        lock.withLock {
            startedAt = clock()
            lastSampleAt = startedAt
            accepting = true
        }
    }

    /// Milliseconds since the start: the timeline the core's transcript and `mark_dictation` use.
    var elapsedMs: UInt64 {
        lock.withLock { UInt64(max(0, clock() - startedAt) * 1_000) }
    }

    var gaps: [MeetingGap] { lock.withLock { recordedGaps } }
    var metrics: MeetingRecorderMetrics {
        queue.sync {
            MeetingRecorderMetrics(
                outputSamples: UInt64(pendingStart + pending.count),
                insertedSilenceSamples: UInt64(insertedSilence),
                gapCount: paddingGaps
            )
        }
    }

    /// Seconds since the last sample while recording; zero while paused.
    var stalledFor: TimeInterval {
        lock.withLock { accepting ? max(0, clock() - lastSampleAt) : 0 }
    }
    var error: String? { lock.withLock { failure } }

    /// While the user dictates, the shared microphone hears the dictation; the meeting keeps the time but not the
    /// words, because the core only silences the "Ik" track itself.
    func setDictating(_ value: Bool) {
        lock.withLock { dictating = value }
    }

    func pause() {
        let at = elapsedMs
        lock.withLock {
            guard accepting else { return }
            accepting = false
            recordedGaps.append(MeetingGap(startMs: at, endMs: nil))
        }
        queue.async { self.flush(all: true) }
    }

    func resume() {
        lock.withLock {
            guard !accepting else { return }
            accepting = true
            resumed = true
            lastSampleAt = clock()
        }
    }

    /// Runs on the realtime audio thread; the work happens on a serial queue.
    func feed(_ samples: [Float]) {
        guard !samples.isEmpty else { return }
        let accepted: (block: [Float], startedAt: TimeInterval)? = lock.withLock {
            guard accepting else { return nil }
            lastSampleAt = clock()
            return (dictating ? [Float](repeating: 0, count: samples.count) : samples, startedAt)
        }
        guard let accepted else { return }
        let block = accepted.block
        let offset = clock() - accepted.startedAt - Double(block.count) / Double(Self.rate)
        let desired = Int(max(0, offset) * Double(Self.rate))
        queue.async { self.place(block, desired: desired) }
    }

    /// Drains the queue, sends what is left and closes the archive. Returns the first failure, if any.
    @discardableResult
    func finish() -> String? {
        let stalled = stalledFor >= Self.stallSeconds
        let at = elapsedMs
        lock.withLock { accepting = false }
        queue.sync {
            flush(all: true)
            archive?.close()
            // Capture that died before Stop opname leaves an open gap up to the end, never a complete recording.
            if stalled {
                let position = UInt64(pendingStart / Self.samplesPerMs)
                lock.withLock {
                    if recordedGaps.last?.endMs != nil || recordedGaps.isEmpty, at > position {
                        recordedGaps.append(MeetingGap(startMs: position, endMs: nil))
                    }
                }
            }
        }
        return error
    }

    private func place(_ samples: [Float], desired: Int) {
        let position = pendingStart + pending.count
        if desired > position + Self.jitterSamples {
            flush(all: true)
            let start = (desired + Self.samplesPerMs - 1) / Self.samplesPerMs * Self.samplesPerMs
            closeGap(from: position, to: start)
            paddingGaps += 1
            archiveSilence(start - position)
            pendingStart = start
        } else if pending.isEmpty, pendingStart % Self.samplesPerMs != 0 {
            let start = (pendingStart + Self.samplesPerMs - 1) / Self.samplesPerMs * Self.samplesPerMs
            archiveSilence(start - pendingStart)
            pendingStart = start
        }
        archiveWrite(samples)
        pending.append(contentsOf: samples)
        flush(all: false)
    }

    /// Closes the gap a pause opened, or records a stall that happened without one.
    private func closeGap(from position: Int, to start: Int) {
        lock.withLock {
            let end = UInt64(start / Self.samplesPerMs)
            if resumed, let last = recordedGaps.indices.last, recordedGaps[last].endMs == nil {
                resumed = false
                recordedGaps[last].endMs = end
            } else if start - position >= Int(Self.stallSeconds * Double(Self.rate)) {
                recordedGaps.append(MeetingGap(startMs: UInt64(position / Self.samplesPerMs), endMs: end))
            }
        }
    }

    private func flush(all: Bool) {
        while pending.count >= Self.rate || (all && !pending.isEmpty) {
            let count = min(pending.count, Self.rate)
            let block = Array(pending.prefix(count))
            pending.removeFirst(count)
            do {
                try sink.push(startMs: UInt64(pendingStart / Self.samplesPerMs), samples: block)
            } catch {
                fail(ErrorText.message(error))
            }
            pendingStart += count
        }
    }

    private func archiveWrite(_ samples: [Float]) {
        guard let archive else { return }
        do {
            try archive.write(samples)
        } catch {
            fail("The audio recording could not be saved. Check available storage.")
        }
        archived += samples.count
    }

    /// Keeps the archive on the transcript's timeline, so "Afspelen vanaf dit punt" lands on the right moment.
    private func archiveSilence(_ count: Int) {
        insertedSilence += count
        var remaining = count
        while remaining > 0 {
            let block = min(remaining, Self.silenceBlock)
            archiveWrite([Float](repeating: 0, count: block))
            remaining -= block
        }
    }

    private func fail(_ message: String) {
        lock.withLock { if failure == nil { failure = message } }
    }
}
