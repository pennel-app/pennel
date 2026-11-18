#if PENNEL_PEER_TRANSPORT_QA
import Foundation

struct PeerComparisonFirstPCM {
    let deadline: TimeInterval
    let milliseconds: Double?

    init(start: () throws -> Void, firstPCM: () -> TimeInterval?,
         uptime: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
         wait: (TimeInterval) -> Void = Thread.sleep(forTimeInterval:)) rethrows {
        let began = uptime()
        deadline = began + 2
        try start()
        while firstPCM() == nil {
            let remaining = deadline - uptime()
            guard remaining > 0 else { break }
            wait(min(0.01, remaining))
        }
        // A callback can land between an empty poll and the clock read at expiry.
        if let first = firstPCM(), first >= began, first <= deadline {
            milliseconds = (first - began) * 1000
        } else {
            milliseconds = nil
        }
    }
}

final class PeerComparisonCaptureJob: @unchecked Sendable {
    private let lock = NSLock()
    private let directory: URL
    private var stopping = false
    private var finished = false
    private var generation = 0
    private var completedBlocks: UInt64 = 0

    init(directory: URL) { self.directory = directory }

    func shouldStop() -> Bool { lock.withLock { stopping } }
    func cancel() { lock.withLock { stopping = true } }
    func end() { lock.withLock { finished = true } }
    func ongoing() -> Bool { lock.withLock { !finished } }

    private var stage: URL { directory.appendingPathComponent("stage-\(generation)", isDirectory: true) }
    func paths() -> (stage: URL, target: URL) {
        lock.withLock { (stage, directory.appendingPathComponent("target-\(generation)")) }
    }

    // Switch to the next generation before removing any files from the completed one.
    func finishedTransfer(blocks: UInt32) {
        lock.withLock {
            completedBlocks += UInt64(blocks)
            generation += 1
        }
    }

    func verifiedBlockCount() throws -> UInt64 {
        try lock.withLock {
            let names: [String]
            do { names = try FileManager.default.contentsOfDirectory(atPath: stage.path) }
            catch let error as CocoaError where error.code == .fileReadNoSuchFile || error.code == .fileNoSuchFile {
                return completedBlocks
            }
            // PartialTransfer publishes these names only after it checks the block length and digest.
            let published = names.filter { $0.range(of: #"^block-[0-9]{8}\.part$"#, options: .regularExpression) != nil }
            return completedBlocks + UInt64(published.count)
        }
    }
}

final class PeerComparisonPayloadReceive: @unchecked Sendable {
    enum Phase { case waiting, pending, returned }
    private let lock = NSLock()
    private var current = Phase.waiting
    var phase: Phase { lock.withLock { current } }

    func observe<T>(_ receive: () throws -> T) rethrows -> T {
        lock.withLock { current = .pending }
        defer { lock.withLock { current = .returned } }
        return try receive()
    }
}
#endif
