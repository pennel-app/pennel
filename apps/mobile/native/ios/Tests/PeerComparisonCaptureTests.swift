#if PENNEL_PEER_TRANSPORT_QA
import AVFoundation
import Foundation
import Testing
@testable internal import PennelMobile

/// Real microphone timing is opt-in; fixtures never leave the isolated QA container.
@Suite(.serialized)
struct PeerComparisonCaptureTests {
    private enum Failure: Error { case configuration, permission, response, transfer, capture }

    private final class Samples: @unchecked Sendable {
        let lock = NSLock()
        var count = 0
        var stopped = false
        var late = false
        func append(_ samples: [Float]) { lock.withLock { count += samples.count; if stopped { late = true } } }
    }
    private final class Job: @unchecked Sendable {
        let lock = NSLock()
        var stop = false
        var finished = false
        func shouldStop() -> Bool { lock.withLock { stop } }
        func end() { lock.withLock { finished = true } }
        func ongoing() -> Bool { lock.withLock { !finished } }
        func cancel() { lock.withLock { stop = true } }
    }

    @Test(.enabled(if: ProcessInfo.processInfo.environment["PENNEL_PEER_CAPTURE_CONFIG_FILE"] == "1"))
    func microphoneCaptureDuringActiveAndStalledTransfers() async throws {
        let configs = try PeerComparisonConfiguration.load("qa-capture.json")
        guard await MicCapture.requestPermission() else { throw Failure.permission }
        for config in configs {
            try await Self.run(config)
        }
    }

    private static func run(_ config: [String: String]) async throws {
        guard let urlString = config["coordinator_url"], let base = URL(string: urlString), base.scheme == "http",
              let token = config["token"], let candidate = config["candidate"], let route = config["route"],
              let phase = config["phase"], ["baseline", "active", "stalled"].contains(phase) else { throw Failure.configuration }
        if phase == "baseline" {
            let measurements = try await Task.detached { try captureTen() }.value
            try await save(base, token, Result(candidate: candidate, route: route, phase: phase, transport_ongoing: false, measurements: measurements))
            return
        }
        let directory = try PeerComparisonQualificationTests.directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = PeerComparisonIdentityKeychain()
        let account = "peer-probe-capture-\(UUID().uuidString)"
        defer { try? store.remove(account: account) }
        let identity = try await Task.detached { try PeerComparisonFactory.identity(store: store, account: account) }.value
        let preparation: Preparation = try await post(base, token, "prepare", ["peer_id": identity.identityId])
        let host = try PeerComparisonQualificationTests.localIPv4()
        let probe = try await Task.detached {
            try PeerComparisonQualificationTests.probe(candidate, store, account, preparation.relay_config_json, host)
        }.value
        defer { probe.close() }
        let fixture = try await fixture(base, token)
        guard fixture.candidate == candidate, fixture.route == route, fixture.source_identity == preparation.source_identity else { throw Failure.response }
        _ = try await Task.detached { try probe.approvePeer(id: fixture.source_identity) }.value
        let job = Job()
        let transfer = Task.detached {
            defer { job.end() }
            var generation = 0
            while !job.shouldStop() {
                let stage = directory.appendingPathComponent("stage-\(generation)")
                let target = directory.appendingPathComponent("target-\(generation)")
                let result = try probe.receiveFile(addressJson: fixture.source_address_json, staging: stage.path, target: target.path, maxBlocks: nil)
                guard result.complete else { throw Failure.transfer }
                try FileManager.default.removeItem(at: target)
                try FileManager.default.removeItem(at: stage)
                generation += 1
            }
        }
        defer { job.cancel(); probe.cancel(); transfer.cancel() }
        let firstStage = directory.appendingPathComponent("stage-0/manifest.json")
        var started = false
        for _ in 0..<100 {
            if FileManager.default.fileExists(atPath: firstStage.path) { started = true; break }
            guard job.ongoing() else { throw Failure.transfer }
            try await Task.sleep(for: .milliseconds(100))
        }
        guard started, job.ongoing() else { throw Failure.transfer }
        if phase == "stalled" {
            let paused: Pause = try await post(base, token, "pause", [String: String]())
            guard paused.paused else { throw Failure.response }
        }
        let measurements = try await Task.detached { try captureTen() }.value
        let ongoing = job.ongoing()
        if phase == "stalled" {
            let resumed: Pause = try await post(base, token, "resume", [String: String]())
            guard !resumed.paused else { throw Failure.response }
        }
        job.cancel(); probe.cancel(); transfer.cancel()
        do { try await transfer.value }
        catch PeerCompareError.Cancelled { }
        catch PeerCompareError.Closed { }
        catch PeerCompareError.Transport { }
        guard ongoing else { throw Failure.transfer }
        try await save(base, token, Result(candidate: candidate, route: route, phase: phase,
                                           transport_ongoing: ongoing, measurements: measurements))
    }

    private static func captureTen() throws -> [Measurement] {
        var results: [Measurement] = []
        for _ in 0..<10 {
            let samples = Samples()
            let capture = MicCapture(audioSession: SystemAudioSession(purpose: .meeting))
            let start = ProcessInfo.processInfo.systemUptime
            try capture.start { samples.append($0) }
            let firstSampleDeadline = ProcessInfo.processInfo.systemUptime + 2
            while samples.lock.withLock({ samples.count == 0 }) && ProcessInfo.processInfo.systemUptime < firstSampleDeadline {
                Thread.sleep(forTimeInterval: 0.01)
            }
            let began = ProcessInfo.processInfo.systemUptime
            Thread.sleep(forTimeInterval: 0.3)
            let alive = capture.isRunning
            let stop = ProcessInfo.processInfo.systemUptime
            capture.stop()
            let ended = ProcessInfo.processInfo.systemUptime
            samples.lock.withLock { samples.stopped = true }
            Thread.sleep(forTimeInterval: 0.02)
            let measurement = samples.lock.withLock {
                Measurement(start_ms: (began - start) * 1000, stop_ms: (ended - stop) * 1000,
                            samples: samples.count, callback_after_stop: samples.late)
            }
            guard alive, measurement.samples > 0, !measurement.callback_after_stop else {
                FileHandle.standardError.write(Data("Peer QA capture failed: running=\(alive) samples=\(measurement.samples) callback-after-stop=\(measurement.callback_after_stop).\n".utf8))
                throw Failure.capture
            }
            results.append(measurement)
        }
        return results
    }

    private static func fixture(_ base: URL, _ token: String) async throws -> Fixture {
        for _ in 0..<100 {
            var request = URLRequest(url: base.appendingPathComponent("fixture"))
            request.timeoutInterval = 5
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            let (data, response) = try await URLSession.shared.data(for: request)
            if (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 32 * 1024 {
                return try JSONDecoder().decode(Fixture.self, from: data)
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw Failure.response
    }
    private static func post<T: Decodable, B: Encodable>(_ base: URL, _ token: String, _ path: String, _ body: B) async throws -> T {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = "POST"; request.timeoutInterval = 30
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200, data.count <= 32 * 1024 else { throw Failure.response }
        return try JSONDecoder().decode(T.self, from: data)
    }
    private static func save(_ base: URL, _ token: String, _ result: Result) async throws {
        let saved: Saved = try await post(base, token, "capture-result", result)
        #expect(saved.saved)
    }
    private struct Preparation: Decodable, Sendable { let source_identity: String; let relay_config_json: String? }
    private struct Fixture: Decodable, Sendable { let source_identity: String; let source_address_json: String; let candidate: String; let route: String }
    private struct Saved: Decodable { let saved: Bool }
    private struct Pause: Decodable { let paused: Bool }
    private struct Measurement: Encodable, Sendable { let start_ms: Double; let stop_ms: Double; let samples: Int; let callback_after_stop: Bool }
    private struct Result: Encodable { let candidate: String; let route: String; let phase: String; let transport_ongoing: Bool; let measurements: [Measurement] }
}
#endif
