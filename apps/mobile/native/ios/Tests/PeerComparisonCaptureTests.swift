#if PENNEL_PEER_TRANSPORT_QA
import AVFoundation
import Foundation
import Testing
import UIKit
@testable internal import PennelMobile

/// Real microphone timing is opt-in; fixtures never leave the isolated QA container.
@Suite(.serialized)
struct PeerComparisonCaptureTests {
    private enum Failure: Error { case configuration, permission, response, transfer, capture }

    private final class Samples: @unchecked Sendable {
        struct Snapshot: Sendable { let count: Int; let late: Bool; let callbacks: Int; let firstPCM: Double? }
        let lock = NSLock()
        var count = 0
        var stopped = false
        var late = false
        private var callbacks = 0
        private var firstPCM: Double?
        func append(_ samples: [Float]) {
            lock.withLock {
                callbacks += 1
                if !samples.isEmpty, firstPCM == nil { firstPCM = ProcessInfo.processInfo.systemUptime }
                count += samples.count
                if stopped { late = true }
            }
        }
        func snapshot() -> Snapshot { lock.withLock { Snapshot(count: count, late: late, callbacks: callbacks, firstPCM: firstPCM) } }
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

    /// Numeric event codes: 0 begin, 1 start call, 2 start return, 3 first PCM,
    /// 4 first-PCM deadline, 5 stop call, 6 stop return, 7 failure, 8 engine change,
    /// 9 route change, 10 interruption, 11 media lost, 12 media reset, 13 app state, 14 end.
    private final class CaptureDiagnostics: @unchecked Sendable {
        private let cycle: Int
        private let candidate: Int
        private let phase: Int
        private let seam: Int
        private let began = ProcessInfo.processInfo.systemUptime
        private let lock = NSLock()
        private let queue = DispatchQueue(label: "app.pennel.peer-transport.qa.capture-diagnostics")
        private var appState: Int
        private var sequence = 0
        private var closed = false
        private var observers: [NSObjectProtocol] = []

        init(cycle: Int, candidate: Int, phase: Int, seam: Int, appState: Int) {
            self.cycle = cycle; self.candidate = candidate; self.phase = phase; self.seam = seam; self.appState = appState
            let center = NotificationCenter.default
            observers.append(center.addObserver(forName: .AVAudioEngineConfigurationChange, object: nil, queue: nil) { [weak self] note in
                let running = (note.object as? AVAudioEngine).map { $0.isRunning ? 1 : 0 } ?? -1
                self?.emit(8, reason: running)
            })
            observers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: nil) { [weak self] note in
                self?.emit(9, reason: (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? NSNumber)?.intValue ?? -1)
            })
            observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: nil) { [weak self] note in
                self?.emit(10, reason: (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? NSNumber)?.intValue ?? -1,
                           auxiliary: (note.userInfo?[AVAudioSessionInterruptionOptionKey] as? NSNumber)?.intValue ?? -1)
            })
            for (name, event) in [(AVAudioSession.mediaServicesWereLostNotification, 11), (AVAudioSession.mediaServicesWereResetNotification, 12)] {
                observers.append(center.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in self?.emit(event) })
            }
            for (name, state) in [(UIApplication.willResignActiveNotification, 1), (UIApplication.didEnterBackgroundNotification, 2),
                                  (UIApplication.willEnterForegroundNotification, 1), (UIApplication.didBecomeActiveNotification, 0)] {
                observers.append(center.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in self?.setAppState(state) })
            }
        }

        func setAppState(_ value: Int) {
            lock.withLock { appState = value }
            emit(13)
        }

        func emit(_ event: Int, reason: Int = -1, auxiliary: Int = -1, running: Bool? = nil,
                  format: MicrophoneInputFormat? = nil, samples: Samples.Snapshot? = nil,
                  deadline: Double? = nil, error: Error? = nil) {
            let state: (Int, Int)? = lock.withLock {
                guard !closed else { return nil }
                sequence += 1
                return (appState, sequence)
            }
            guard let state else { return }
            var status = 0
            var errorKind = 0
            if let error {
                let value = error as NSError
                if value.domain == NSOSStatusErrorDomain || value.domain == "com.apple.coreaudio.avfaudio" {
                    errorKind = value.domain == NSOSStatusErrorDomain ? 1 : 2
                    if let code = Int32(exactly: value.code) { status = Int(code) }
                } else { errorKind = 3 }
            }
            let fields: [String: Double] = [
                "event": Double(event), "cycle": Double(cycle), "candidate": Double(candidate), "phase": Double(phase),
                "seam": Double(seam), "sequence": Double(state.1), "app_state": Double(state.0),
                "elapsed_ms": (ProcessInfo.processInfo.systemUptime - began) * 1000,
                "reason": Double(reason), "auxiliary": Double(auxiliary), "running": running.map { $0 ? 1 : 0 } ?? -1,
                "physical_rate": Double(format?.sampleRate ?? 0), "physical_channels": Double(format?.channels ?? 0),
                "samples": Double(samples?.count ?? 0), "callbacks": Double(samples?.callbacks ?? 0),
                "first_pcm_since_cycle_ms": samples?.firstPCM.map { ($0 - began) * 1000 } ?? -1,
                "first_deadline_since_cycle_ms": deadline.map { ($0 - began) * 1000 } ?? -1,
                "osstatus": Double(status), "error_kind": Double(errorKind),
            ]
            // No audio, paths, device names, port identifiers or NSError descriptions are logged.
            // In particular, the realtime PCM callback only updates counters; it never logs.
            queue.async {
                let session = AVAudioSession.sharedInstance()
                var values = fields
                values["session_rate"] = session.sampleRate.isFinite ? session.sampleRate : -1
                values["session_input_channels"] = Double(session.inputNumberOfChannels)
                values["input_available"] = session.isInputAvailable ? 1 : 0
                values["route_inputs"] = Double(session.currentRoute.inputs.count)
                values["route_outputs"] = Double(session.currentRoute.outputs.count)
                values["record_permission"] = Double(AVAudioApplication.shared.recordPermission.rawValue)
                guard let encoded = try? JSONSerialization.data(withJSONObject: values, options: [.sortedKeys]) else { return }
                FileHandle.standardError.write(Data("Peer QA capture numeric ".utf8))
                FileHandle.standardError.write(encoded)
                FileHandle.standardError.write(Data([10]))
            }
        }

        func close() {
            let tokens: [NSObjectProtocol] = lock.withLock {
                guard !closed else { return [] }
                closed = true
                let result = observers
                observers.removeAll()
                return result
            }
            tokens.forEach { NotificationCenter.default.removeObserver($0) }
            queue.sync { }
        }
        deinit { close() }
    }

    /// Each production-path cycle owns its audio-session reference count and microphone.
    /// This exercises the real route/configuration recovery without touching global clients.
    private final class CaptureCycle {
        let capture: any AudioCapturing
        private let session: (any AudioSessionControlling)?
        private let microphoneOwner: SharedMicrophone?
        private let sessionOwner: SharedAudioSession?
        init(rawControl: Bool) {
            if rawControl {
                capture = MicCapture(audioSession: SystemAudioSession(purpose: .meeting))
                session = nil; microphoneOwner = nil; sessionOwner = nil
            } else {
                let sharedSession = SharedAudioSession()
                let microphone = SharedMicrophone(capture: MicCapture(audioSession: nil))
                capture = microphone.client()
                session = sharedSession.client(.meeting)
                microphoneOwner = microphone; sessionOwner = sharedSession
            }
        }
        func start(_ append: @escaping @Sendable ([Float]) -> Void) throws {
            try session?.activate()
            try capture.start(onSamples: append)
        }
        func stop() { capture.stop(); session?.deactivate() }
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
            let measurements = try await captureTen(candidate: candidate, phase: phase)
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
        var relayPaused = false
        do {
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
                relayPaused = true
            }
            let measurements = try await captureTen(candidate: candidate, phase: phase)
            let ongoing = job.ongoing()
            if relayPaused {
                let resumed: Pause = try await post(base, token, "resume", [String: String]())
                guard !resumed.paused else { throw Failure.response }
                relayPaused = false
            }
            try await drain(transfer, job: job, probe: probe)
            guard ongoing else { throw Failure.transfer }
            try await save(base, token, Result(candidate: candidate, route: route, phase: phase,
                                               transport_ongoing: ongoing, measurements: measurements))
        } catch {
            let failure = error
            // Retire the real owned worker before scoped files and keychain entries are removed.
            try? await drain(transfer, job: job, probe: probe)
            if relayPaused { let _: Pause? = try? await post(base, token, "resume", [String: String]()) }
            throw failure
        }
    }

    private static func drain(_ transfer: Task<Void, Error>, job: Job, probe: PeerComparisonProbe) async throws {
        job.cancel(); probe.cancel(); transfer.cancel()
        do { try await transfer.value }
        catch PeerCompareError.Cancelled { }
        catch PeerCompareError.Closed { }
        catch PeerCompareError.Transport { }
    }

    private static func captureTen(candidate: String, phase: String) async throws -> [Measurement] {
        let selector = ProcessInfo.processInfo.environment["PENNEL_PEER_CAPTURE_RAW_CONTROL"] ?? "0"
        guard ["0", "1"].contains(selector) else { throw Failure.configuration }
        let rawControl = selector == "1"
        // Constant labels identify the tested seam; they contain no user or device data.
        FileHandle.standardError.write(Data(rawControl ? "Peer QA capture seam=raw-control\n".utf8 : "Peer QA capture seam=isolated-production\n".utf8))
        let candidateCode = candidate == "quinn" ? 0 : 1
        let phaseCode = phase == "baseline" ? 0 : (phase == "active" ? 1 : 2)
        var results: [Measurement] = []
        for index in 0..<10 {
            let before = await MainActor.run { UIApplication.shared.applicationState.rawValue }
            let diagnostics = CaptureDiagnostics(cycle: index + 1, candidate: candidateCode, phase: phaseCode,
                                                 seam: rawControl ? 0 : 1, appState: before)
            defer { diagnostics.close() }
            diagnostics.emit(0)
            do {
                let measurement = try await Task.detached { try captureOne(rawControl: rawControl, diagnostics: diagnostics) }.value
                let after = await MainActor.run { UIApplication.shared.applicationState.rawValue }
                diagnostics.setAppState(after)
                diagnostics.emit(14)
                results.append(measurement)
            } catch {
                let after = await MainActor.run { UIApplication.shared.applicationState.rawValue }
                diagnostics.setAppState(after)
                diagnostics.emit(7, error: error)
                throw error
            }
        }
        return results
    }

    private static func captureOne(rawControl: Bool, diagnostics: CaptureDiagnostics) throws -> Measurement {
        let samples = Samples()
        let cycle = CaptureCycle(rawControl: rawControl)
        defer { cycle.stop() }
        let start = ProcessInfo.processInfo.systemUptime
        diagnostics.emit(1)
        do { try cycle.start { samples.append($0) } }
        catch { diagnostics.emit(7, running: cycle.capture.isRunning, format: cycle.capture.inputFormat, error: error); throw error }
        diagnostics.emit(2, running: cycle.capture.isRunning, format: cycle.capture.inputFormat)
        let firstSampleDeadline = ProcessInfo.processInfo.systemUptime + 2
        while samples.snapshot().count == 0 && ProcessInfo.processInfo.systemUptime < firstSampleDeadline {
            Thread.sleep(forTimeInterval: 0.01)
        }
        let ready = samples.snapshot()
        diagnostics.emit(ready.count > 0 ? 3 : 4, running: cycle.capture.isRunning,
                         format: cycle.capture.inputFormat, samples: ready, deadline: firstSampleDeadline)
        let began = ProcessInfo.processInfo.systemUptime
        Thread.sleep(forTimeInterval: 0.3)
        let held = samples.snapshot()
        let alive = cycle.capture.isRunning
        diagnostics.emit(5, running: alive, format: cycle.capture.inputFormat, samples: held)
        let stop = ProcessInfo.processInfo.systemUptime
        cycle.stop()
        let ended = ProcessInfo.processInfo.systemUptime
        samples.lock.withLock { samples.stopped = true }
        Thread.sleep(forTimeInterval: 0.02)
        let final = samples.snapshot()
        diagnostics.emit(6, running: cycle.capture.isRunning, format: cycle.capture.inputFormat, samples: final)
        let measurement = Measurement(start_ms: (began - start) * 1000, stop_ms: (ended - stop) * 1000,
                                      samples: final.count, callback_after_stop: final.late)
        guard alive, ready.count > 0, held.count > ready.count, !measurement.callback_after_stop else {
            diagnostics.emit(7, reason: ready.count, auxiliary: held.count, running: alive, samples: final)
            throw Failure.capture
        }
        return measurement
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
