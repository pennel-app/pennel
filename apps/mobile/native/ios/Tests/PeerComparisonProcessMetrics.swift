#if PENNEL_PEER_TRANSPORT_QA
import Darwin
import Foundation
@testable internal import PennelMobile

enum PeerComparisonConfiguration {
    enum Failure: Error { case invalid }
    static func load(_ name: String) throws -> [[String: String]] {
        guard AppPaths.qaProfile == "peer-transport", ["qa-roundtrip.json", "qa-capture.json"].contains(name),
              let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else { throw Failure.invalid }
        let url = documents.appendingPathComponent("QA/peer-transport", isDirectory: true).appendingPathComponent(name)
        let data = try Data(contentsOf: url)
        defer { try? FileManager.default.removeItem(at: url) }
        guard data.count <= 32 * 1024, let configs = try JSONSerialization.jsonObject(with: data) as? [[String: String]],
              !configs.isEmpty, configs.count <= 12 else { throw Failure.invalid }
        return configs
    }
}

/// Public SDK counters for this app process; this does not measure extensions or system services.
final class PeerComparisonProcessMetrics: @unchecked Sendable {
    struct Snapshot: Encodable, Sendable {
        let scope = "owned_app_process"
        let method = "mach_task_basic_info+getrusage"
        let idle_rss_bytes: UInt64
        let peak_rss_bytes: UInt64
        let cpu_seconds: Double
        let sample_count: Int
        let elapsed_seconds: Double
    }
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "app.pennel.peer-transport.qa.metrics")
    private let timer: DispatchSourceTimer
    private let began = ProcessInfo.processInfo.systemUptime
    private let initialCPU: Double
    private let initialRSS: UInt64
    private var peak: UInt64
    private var count = 0

    init() {
        initialCPU = Self.cpu()
        initialRSS = Self.rss()
        peak = initialRSS
        timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(50))
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            let value = Self.rss()
            self.lock.withLock { self.peak = max(self.peak, value); self.count += 1 }
        }
        timer.resume()
    }
    deinit { timer.cancel() }
    func snapshot() -> Snapshot {
        let current = Self.rss()
        return lock.withLock {
            Snapshot(idle_rss_bytes: initialRSS, peak_rss_bytes: max(peak, current),
                cpu_seconds: max(0, Self.cpu() - initialCPU), sample_count: max(1, count),
                elapsed_seconds: ProcessInfo.processInfo.systemUptime - began)
        }
    }
    private static func rss() -> UInt64 {
        var info = mach_task_basic_info_data_t()
        var size = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(size)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &size)
            }
        }
        return result == KERN_SUCCESS ? UInt64(info.resident_size) : 0
    }
    private static func cpu() -> Double {
        var usage = rusage()
        guard getrusage(RUSAGE_SELF, &usage) == 0 else { return 0 }
        return Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec)
            + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1_000_000
    }
}
#endif
