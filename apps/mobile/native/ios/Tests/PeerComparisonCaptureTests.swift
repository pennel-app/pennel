#if PENNEL_PEER_TRANSPORT_QA
import AVFoundation
import Foundation
import Testing
import UIKit
@testable internal import PennelMobile

/// Real microphone timing is opt-in; fixtures never leave the isolated QA container.
@Suite(.serialized)
struct PeerComparisonCaptureTests {
    private enum Failure: Error { case configuration, permission, response, transfer, capture, routeUnavailable, routeTransition }

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
    /// Route control: 20 available-input mask, 21 baseline PCM, 22 HFP request,
    /// 23 changed physical format, 24 fresh PCM, 25 held PCM, 26 cleanup, 27 unavailable, 28 restored.
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
        private var engineChanges = 0
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

        func engineConfigurationChanges() -> Int { lock.withLock { engineChanges } }

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
                if event == 8 { engineChanges += 1 }
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
        func activateSession() throws { try session?.activate() }
        func start(_ append: @escaping @Sendable ([Float]) -> Void) throws {
            try activateSession()
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

    /// This separate opt-in control must observe a real physical-format transition.
    /// No settings UI, synthetic notification, retry, or inferred route success is used.
    @Test(.enabled(if: ProcessInfo.processInfo.environment["PENNEL_PEER_CAPTURE_ROUTE_CONTROL"] == "1"))
    func microphoneRecoversAfterPhysicalInputRouteChange() async throws {
        guard AppPaths.qaProfile == "peer-transport" else { throw Failure.configuration }
        guard await MicCapture.requestPermission() else { throw Failure.permission }
        let before = await MainActor.run { UIApplication.shared.applicationState.rawValue }
        // Candidate=2/phase=3 identify this standalone route control; seam=1 is shared ownership.
        let diagnostics = CaptureDiagnostics(cycle: 1, candidate: 2, phase: 3, seam: 1, appState: before)
        defer { diagnostics.close() }
        diagnostics.emit(0)
        do {
            try await Task.detached { try Self.exercisePhysicalInputRoute(diagnostics) }.value
            let after = await MainActor.run { UIApplication.shared.applicationState.rawValue }
            diagnostics.setAppState(after)
            diagnostics.emit(14)
        } catch {
            let after = await MainActor.run { UIApplication.shared.applicationState.rawValue }
            diagnostics.setAppState(after)
            diagnostics.emit(7, error: error)
            throw error
        }
    }

    /// Preferred rates are requests: only an observed reconfiguration can pass.
    @Test(.enabled(if: ProcessInfo.processInfo.environment["PENNEL_PEER_CAPTURE_FORMAT_CONTROL"] == "1"))
    func microphoneRecoversAfterSessionFormatChange() async throws {
        guard AppPaths.qaProfile == "peer-transport" else { throw Failure.configuration }
        guard await MicCapture.requestPermission() else { throw Failure.permission }
        let state = await MainActor.run { UIApplication.shared.applicationState.rawValue }
        let diagnostics = CaptureDiagnostics(cycle: 1, candidate: 2, phase: 4, seam: 1, appState: state)
        defer { diagnostics.close() }
        diagnostics.emit(0)
        do {
            try await Task.detached { try Self.exerciseSessionFormat(diagnostics) }.value
            diagnostics.emit(14)
        } catch {
            diagnostics.emit(7, error: error)
            throw error
        }
    }

    private static func exerciseSessionFormat(_ diagnostics: CaptureDiagnostics) throws {
        let cycle = CaptureCycle(rawControl: false)
        let session = AVAudioSession.sharedInstance()
        let previous = session.preferredSampleRate
        var requested = false
        defer {
            cycle.capture.stop()
            if requested {
                do { try session.setPreferredSampleRate(previous) }
                catch {
                    diagnostics.emit(7, error: error)
                    Issue.record("Peer QA sample-rate preference restoration failed.")
                }
            }
            cycle.stop()
            diagnostics.emit(28)
        }
        let samples = Samples()
        try cycle.start { samples.append($0) }
        let initialDeadline = ProcessInfo.processInfo.systemUptime + 2
        while samples.snapshot().count == 0 && ProcessInfo.processInfo.systemUptime < initialDeadline {
            Thread.sleep(forTimeInterval: 0.01)
        }
        guard cycle.capture.isRunning, samples.snapshot().count > 0,
              let before = cycle.capture.inputFormat else { throw Failure.capture }
        let ready = samples.snapshot().count
        Thread.sleep(forTimeInterval: 0.3)
        guard cycle.capture.isRunning, samples.snapshot().count > ready else { throw Failure.capture }
        let target: Double = before.sampleRate == 48_000 ? 44_100 : 48_000
        let events = diagnostics.engineConfigurationChanges()
        let deadline = ProcessInfo.processInfo.systemUptime + 2
        diagnostics.emit(22, reason: Int(target), auxiliary: Int(previous), running: true,
                         format: before, samples: samples.snapshot(), deadline: deadline)
        requested = true
        try session.setPreferredSampleRate(target)
        var changed: MicrophoneInputFormat?
        var checkpoint: Int?
        var recovered = false
        while ProcessInfo.processInfo.systemUptime < deadline {
            if cycle.capture.isRunning, let format = cycle.capture.inputFormat,
               format.sampleRate != before.sampleRate,
               abs(Double(format.sampleRate) - session.sampleRate) < 1 {
                let count = samples.snapshot().count
                if checkpoint == nil {
                    changed = format
                    checkpoint = count
                    diagnostics.emit(23, running: true, format: format, samples: samples.snapshot())
                } else if let checkpoint, count > checkpoint {
                    recovered = true
                    break
                }
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        guard recovered, let changed, diagnostics.engineConfigurationChanges() > events else {
            diagnostics.emit(4, running: cycle.capture.isRunning, format: cycle.capture.inputFormat,
                             samples: samples.snapshot(), deadline: deadline)
            throw Failure.routeTransition
        }
        let count = samples.snapshot().count
        Thread.sleep(forTimeInterval: 0.3)
        guard cycle.capture.isRunning, samples.snapshot().count > count else { throw Failure.capture }
        diagnostics.emit(25, running: true, format: changed, samples: samples.snapshot())
    }

    private static func exercisePhysicalInputRoute(_ diagnostics: CaptureDiagnostics) throws {
        let cycle = CaptureCycle(rawControl: false)
        let session = AVAudioSession.sharedInstance()
        let previous = session.preferredInput
        var changedPreference = false
        defer {
            // Stop this client before restoration so restoration cannot start another owned engine.
            diagnostics.emit(26, reason: changedPreference ? 1 : 0, running: cycle.capture.isRunning, format: cycle.capture.inputFormat)
            cycle.capture.stop()
            if changedPreference {
                do { try session.setPreferredInput(previous) }
                catch {
                    diagnostics.emit(7, error: error)
                    Issue.record("Peer QA route preference restoration failed.")
                }
            }
            cycle.stop()
            diagnostics.emit(28)
        }
        try cycle.activateSession()
        // HFP may become available after activation. This bounded discovery does not
        // start/retry capture or extend either subsequent PCM/route-change deadline.
        let discoveryDeadline = ProcessInfo.processInfo.systemUptime + 2
        var inputs: [AVAudioSessionPortDescription] = []
        var builtIn: AVAudioSessionPortDescription?
        var bluetooth: AVAudioSessionPortDescription?
        while true {
            inputs = session.availableInputs ?? []
            builtIn = inputs.first { $0.portType == .builtInMic }
            bluetooth = inputs.first { $0.portType == .bluetoothHFP }
            if builtIn != nil, bluetooth != nil { break }
            let remaining = discoveryDeadline - ProcessInfo.processInfo.systemUptime
            guard remaining > 0 else { break }
            Thread.sleep(forTimeInterval: min(0.01, remaining))
        }
        let mask = (builtIn == nil ? 0 : 1) | (bluetooth == nil ? 0 : 2)
        diagnostics.emit(20, reason: mask, auxiliary: inputs.count, deadline: discoveryDeadline)
        guard let builtIn, let bluetooth else {
            diagnostics.emit(27, reason: mask, auxiliary: inputs.count)
            FileHandle.standardError.write(Data("Peer QA physical route control unavailable.\n".utf8))
            throw Failure.routeUnavailable
        }
        changedPreference = true
        try session.setPreferredInput(builtIn)
        let samples = Samples()
        diagnostics.emit(1, reason: 1)
        do { try cycle.start { samples.append($0) } }
        catch { diagnostics.emit(7, error: error); throw error }
        diagnostics.emit(2, running: cycle.capture.isRunning, format: cycle.capture.inputFormat)
        let initialDeadline = ProcessInfo.processInfo.systemUptime + 2
        var beforeFormat: MicrophoneInputFormat?
        while ProcessInfo.processInfo.systemUptime < initialDeadline {
            if cycle.capture.isRunning, samples.snapshot().count > 0,
               session.currentRoute.inputs.contains(where: { $0.portType == .builtInMic }),
               let format = cycle.capture.inputFormat, format.sampleRate > 0, format.channels > 0,
               abs(Double(format.sampleRate) - session.sampleRate) < 1 {
                beforeFormat = format
                break
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        guard let beforeFormat else {
            diagnostics.emit(4, running: cycle.capture.isRunning, format: cycle.capture.inputFormat,
                             samples: samples.snapshot(), deadline: initialDeadline)
            throw Failure.routeTransition
        }
        let beforeReady = samples.snapshot()
        Thread.sleep(forTimeInterval: 0.3)
        let beforeHeld = samples.snapshot()
        diagnostics.emit(21, reason: beforeReady.count, auxiliary: beforeHeld.count,
                         running: cycle.capture.isRunning, format: beforeFormat, samples: beforeHeld)
        guard cycle.capture.isRunning, beforeHeld.count > beforeReady.count else { throw Failure.capture }

        let engineChangesBefore = diagnostics.engineConfigurationChanges()
        let transition = ProcessInfo.processInfo.systemUptime
        let deadline = transition + 2
        diagnostics.emit(22, reason: 2, running: cycle.capture.isRunning, format: beforeFormat,
                         samples: beforeHeld, deadline: deadline)
        try session.setPreferredInput(bluetooth)
        var changedFormat: MicrophoneInputFormat?
        var countAtChangedFormat: Int?
        var freshPCM = false
        while ProcessInfo.processInfo.systemUptime < deadline {
            let now = samples.snapshot()
            if cycle.capture.isRunning,
               session.currentRoute.inputs.contains(where: { $0.portType == .bluetoothHFP }),
               let format = cycle.capture.inputFormat,
               format.sampleRate != beforeFormat.sampleRate || format.channels != beforeFormat.channels {
                if countAtChangedFormat == nil {
                    changedFormat = format
                    countAtChangedFormat = now.count
                    diagnostics.emit(23, reason: beforeHeld.count, auxiliary: now.count,
                                     running: true, format: format, samples: now, deadline: deadline)
                } else if let checkpoint = countAtChangedFormat, now.count > checkpoint {
                    freshPCM = true
                    break
                }
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        guard let changedFormat, freshPCM else {
            diagnostics.emit(4, reason: beforeHeld.count, auxiliary: samples.snapshot().count,
                             running: cycle.capture.isRunning, format: cycle.capture.inputFormat,
                             samples: samples.snapshot(), deadline: deadline)
            throw Failure.routeTransition
        }
        let afterReady = samples.snapshot()
        diagnostics.emit(24, reason: countAtChangedFormat ?? -1, auxiliary: afterReady.count,
                         running: cycle.capture.isRunning, format: changedFormat, samples: afterReady,
                         deadline: deadline)
        Thread.sleep(forTimeInterval: 0.3)
        let afterHeld = samples.snapshot()
        diagnostics.emit(25, reason: afterReady.count, auxiliary: afterHeld.count,
                         running: cycle.capture.isRunning, format: cycle.capture.inputFormat, samples: afterHeld)
        guard cycle.capture.isRunning, afterHeld.count > afterReady.count,
              diagnostics.engineConfigurationChanges() > engineChangesBefore else { throw Failure.capture }
        diagnostics.emit(5, reason: engineChangesBefore, auxiliary: diagnostics.engineConfigurationChanges(),
                         running: cycle.capture.isRunning, format: cycle.capture.inputFormat, samples: afterHeld)
        cycle.capture.stop()
        diagnostics.emit(6, running: cycle.capture.isRunning, samples: samples.snapshot())
        samples.lock.withLock { samples.stopped = true }
        Thread.sleep(forTimeInterval: 0.02)
        guard !samples.snapshot().late else { throw Failure.capture }
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
