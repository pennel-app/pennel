import Foundation

public struct SpeechPreviewCaption: Sendable {
    private struct Fragment: Sendable {
        let start: TimeInterval
        let end: TimeInterval
        let text: String
        let isFinal: Bool
    }
    private let maxCharacters: Int
    private var fragments: [Fragment] = []
    private var previous = ""

    public init(maxCharacters: Int = 240) { self.maxCharacters = max(1, maxCharacters) }

    public mutating func update(start: TimeInterval, end: TimeInterval, text: String, isFinal: Bool) -> String? {
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
        var joined = ""
        for fragment in fragments {
            let part = fragment.text.trimmingCharacters(in: .whitespacesAndNewlines)
            if !joined.isEmpty, let first = part.first, !".,!?;:)]}".contains(first) { joined += " " }
            joined += part
        }
        let visible = String(joined.suffix(maxCharacters))
        guard visible != previous else { return nil }
        previous = visible
        return visible
    }
}

public struct SpeechPreviewPCMChunk: Sendable {
    public let samples: [Float]
    public let startFrame: Int64
}

public final class SpeechPreviewBuffer: @unchecked Sendable {
    private enum Phase { case waiting, running, stopped }
    private let lock = NSLock()
    private let maxSamples: Int
    private var phase = Phase.waiting
    private var pending: [Float] = []
    private var frames: Int64 = 0

    public init(maxSamples: Int) { self.maxSamples = max(1, maxSamples) }
    public var isActive: Bool { lock.withLock { phase == .running } }

    public func start() -> Bool {
        lock.withLock {
            guard phase == .waiting else { return false }
            phase = .running
            return true
        }
    }

    public func append(_ samples: [Float]) -> Bool {
        lock.withLock {
            guard phase == .running, !samples.isEmpty else { return false }
            frames += Int64(samples.count)
            if samples.count >= maxSamples { pending = Array(samples.suffix(maxSamples)) }
            else {
                let excess = max(0, pending.count + samples.count - maxSamples)
                if excess > 0 { pending.removeFirst(excess) }
                pending.append(contentsOf: samples)
            }
            return true
        }
    }

    public func drain() -> SpeechPreviewPCMChunk? {
        lock.withLock {
            guard phase == .running, !pending.isEmpty else { return nil }
            let chunk = SpeechPreviewPCMChunk(samples: pending, startFrame: frames - Int64(pending.count))
            pending.removeAll(keepingCapacity: true)
            return chunk
        }
    }

    public func stop() { lock.withLock { phase = .stopped; pending.removeAll(keepingCapacity: false) } }
}

public final class SpeechPreviewCallbackGate: @unchecked Sendable {
    private let lock = NSRecursiveLock()
    private var callback: (@Sendable (String) -> Void)?

    public init(_ callback: @escaping @Sendable (String) -> Void) { self.callback = callback }
    public func emit(_ text: String) { lock.withLock { callback?(text) } }
    public func stop() { lock.withLock { callback = nil } }
}
