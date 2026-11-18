@preconcurrency import AVFoundation
import CoreAudio

struct MeetingMicrophoneRoute: Equatable {
    let input: AudioDeviceID?
    let output: AudioDeviceID?

    func defaultChangeNeedsRestart(from installed: Self?, running: Bool) -> Bool {
        // An unknown default during negotiation is not proof that a healthy graph was lost.
        input != nil && output != nil && (!running || self != installed)
    }

    static var current: Self {
        Self(input: AudioInput.defaultDevice(kAudioHardwarePropertyDefaultInputDevice),
             output: AudioInput.defaultDevice(kAudioHardwarePropertyDefaultOutputDevice))
    }
}

/// A failed processing unit is retried on a real device change, never on its own configuration notifications.
struct MeetingMicrophoneEchoPolicy {
    let requested: Bool
    private var rawRoute: MeetingMicrophoneRoute?

    init(requested: Bool) { self.requested = requested }

    func shouldTry(on route: MeetingMicrophoneRoute) -> Bool {
        guard requested else { return false }
        guard let rawRoute else { return true }
        return route.input != nil && route.output != nil && route != rawRoute
    }

    mutating func didStart(processed: Bool, on route: MeetingMicrophoneRoute) {
        rawRoute = requested && !processed ? route : nil
    }
}

final class MeetingMicrophoneHealth: @unchecked Sendable {
    enum State { case inactive, running(echo: Bool), failed }
    private let lock = NSLock()
    private var state: State = .inactive

    func set(_ next: State) { lock.withLock { state = next } }

    func deactivate() {
        lock.withLock { if case .running = state { state = .inactive } }
    }

    var echoActive: Bool {
        if case .running(let echo) = lock.withLock({ state }) { return echo }
        return false
    }

    func apply(to health: inout MeetingCaptureHealth) {
        switch lock.withLock({ state }) {
        case .inactive: health.microphoneStarting()
        case .running(let echo): health.microphoneStarted(echoActive: echo)
        case .failed: health.microphoneFailed()
        }
    }
}

/// Disarm an old tap and wait out any admitted write before replacing its engine or releasing the foreign context.
final class MicrophoneTapAdmission: @unchecked Sendable {
    private let lock = NSLock()
    private var open = true

    func close() { lock.withLock { open = false } }
    func deliver(_ work: () -> Void) { lock.withLock { if open { work() } } }
}

@MeetingMicrophoneControl
final class MeetingMicrophoneCapture {
    private let log = Diagnostics("meeting")
    private lazy var engine = AVAudioEngine()
    private lazy var configuration = MeetingMicrophoneConfigurationObserver()
    private var generation = CaptureGeneration()
    private var starting = false
    private var configurationChangedDuringStart = false
    private var writer: TrackWriter?
    private var diagnostics: CaptureDiagnostics?
    private var admission: MicrophoneTapAdmission?
    private var installedRoute: MeetingMicrophoneRoute?
    private var installedFormat: AVAudioFormat?
    private var usingProcessedEngine = false
    private var echoPolicy = MeetingMicrophoneEchoPolicy(requested: true)
    nonisolated let health = MeetingMicrophoneHealth()

    nonisolated init() {}

    /// The synchronous C caller waits for the engine result. Main and the microphone actor stay available
    /// between hardware reads; the microphone actor never waits for main.
    nonisolated func start(writer: TrackWriter, diagnostics: CaptureDiagnostics, echo: Bool) throws {
        let ready = DispatchSemaphore(value: 0)
        var result: Result<Void, Error> = .failure(MicrophoneStartError.failed)
        Task { @MeetingMicrophoneControl in
            guard self.writer == nil else { ready.signal(); return }
            self.health.set(.inactive)
            self.writer = writer
            self.diagnostics = diagnostics
            self.echoPolicy = MeetingMicrophoneEchoPolicy(requested: echo)
            self.observeConfiguration()
            do {
                try await self.startMicrophone()
                result = .success(())
            } catch { result = .failure(error) }
            ready.signal()
        }
        ready.wait()
        try result.get()
    }

    nonisolated func stop() {
        let stopped = DispatchSemaphore(value: 0)
        Task { @MeetingMicrophoneControl in
            guard self.writer != nil else { stopped.signal(); return }
            self.generation.stop()
            self.starting = false
            self.configurationChangedDuringStart = false
            self.configuration.stop()
            self.removeTapAndStop()
            self.disableVoiceProcessing()
            self.writer = nil
            self.diagnostics = nil
            self.health.deactivate()
            stopped.signal()
        }
        stopped.wait()
    }

    private func startMicrophone(forceRaw: Bool = false) async throws {
        guard let writer, !starting else { throw CancellationError() }
        let token = generation.begin()
        starting = true
        configurationChangedDuringStart = false
        health.deactivate()
        defer { if generation.accepts(token) { starting = false } }
        do {
            let attemptedEcho = !forceRaw && echoPolicy.shouldTry(on: .current)
            let processed = try await MeetingMicrophoneStartup.start(
                echoCancellation: attemptedEcho,
                checkCurrent: { try self.checkGeneration(token) },
                attempt: { echo in try await self.startEngine(echo: echo, writer: writer, token: token) },
                reset: {
                    self.diagnostics?.event("rebuild", track: "microphone", fields: ["reason": "voice_processing_start_failed"])
                    self.log.notice("echo-enabled microphone start failed; retrying with a fresh engine without voice processing")
                    self.replaceEngine()
                }
            )
            try checkGeneration(token)
            guard engine.isRunning else { throw MicrophoneStartError.failed }
            let input = engine.inputNode
            let active = processed && input.isVoiceProcessingEnabled && !input.isVoiceProcessingBypassed
            usingProcessedEngine = processed
            if let installedRoute { echoPolicy.didStart(processed: processed, on: installedRoute) }
            health.set(.running(echo: active))
            if configurationChangedDuringStart {
                // Coalesce setup-generated notifications. A stopped unit or a real route change
                // between tap installation and start still gets one guarded recheck.
                let observed = engine
                Task { @MeetingMicrophoneControl [weak self] in
                    guard let self, self.generation.accepts(token) else { return }
                    await self.checkConfiguration(of: observed)
                }
            }
        } catch {
            if generation.accepts(token) {
                removeTapAndStop()
                health.set(.failed)
            }
            throw error
        }
    }

    private func checkGeneration(_ token: UInt64) throws {
        guard generation.accepts(token), writer != nil else { throw CancellationError() }
    }

    private func startEngine(echo: Bool, writer: TrackWriter, token: UInt64) async throws {
        try checkGeneration(token)
        let before = engine.inputNode.inputFormat(forBus: 0)
        log.notice("microphone setup: hardware=\(before.sampleRate) Hz/\(before.channelCount) ch, echo_requested=\(echo)")
        var reads = 0
        let format = try await MeetingMicrophoneFormatReadiness.resolve(
            attempts: 41,
            configure: {
                if echo {
                    do { try self.engine.inputNode.setVoiceProcessingEnabled(true) }
                    catch {
                        self.log.error("voice processing unavailable: \(error.localizedDescription)")
                        throw MicrophoneStartError.forEngineFailure(access: MicrophoneAccess.state)
                    }
                    let input = self.engine.inputNode
                    input.isVoiceProcessingBypassed = false
                    input.voiceProcessingOtherAudioDuckingConfiguration =
                        AVAudioVoiceProcessingOtherAudioDuckingConfiguration(enableAdvancedDucking: false, duckingLevel: .min)
                    self.engine.mainMixerNode.outputVolume = 0
                } else { self.disableVoiceProcessing() }
            },
            read: {
                reads += 1
                let input = self.engine.inputNode
                let hardware = input.inputFormat(forBus: 0)
                if reads == 1 || reads == 41 {
                    let output = input.outputFormat(forBus: 0)
                    self.log.notice("microphone format after setup: input=\(hardware.sampleRate) Hz/\(hardware.channelCount) ch, output=\(output.sampleRate) Hz/\(output.channelCount) ch, echo=\(input.isVoiceProcessingEnabled), read=\(reads)")
                }
                return hardware
            },
            usable: { AudioInput.isUsable(sampleRate: $0.sampleRate, channels: $0.channelCount) },
            defaultInputAvailable: AudioInput.defaultDeviceAvailable,
            checkAccess: {
                try self.checkGeneration(token)
                switch MicrophoneAccess.state {
                case .granted: break
                case .denied: throw MicrophoneStartError.permissionDenied
                case .notDetermined: throw MicrophoneStartError.permissionPending
                }
            },
            pause: { try await Task.sleep(for: .milliseconds(50)) }
        )
        try checkGeneration(token)
        let input = engine.inputNode
        // Keep the historical guard against stale client formats through Bluetooth profile changes.
        let hardware = input.inputFormat(forBus: 0)
        guard AudioInput.isUsable(sampleRate: hardware.sampleRate, channels: hardware.channelCount) else {
            throw MicrophoneStartError.failed
        }
        if reads > 1 { log.notice("microphone hardware format became ready after \(reads) reads") }
        if !hardware.isEqual(format) { log.notice("microphone hardware format changed before tap installation") }
        let resampler = MonoResampler(from: hardware)
        let diagnostics = diagnostics
        diagnostics?.format(track: "microphone", format: hardware, transport: CaptureDeviceTransport.input(input.audioUnit))
        let silence = SilenceProbe(threshold: 5)
        let gate = MicrophoneTapAdmission()
        admission = gate
        input.removeTap(onBus: 0)
        let tapRoute = MeetingMicrophoneRoute.current
        input.installTap(onBus: 0, bufferSize: 4096, format: hardware) { @Sendable [weak self] buffer, time in
            gate.deliver {
                guard let converted = resampler.convert(buffer) else { return }
                diagnostics?.count(track: "microphone", key: "captured_frames", value: UInt64(buffer.frameLength))
                let uptime = time.isHostTimeValid ? AVAudioTime.seconds(forHostTime: time.hostTime)
                    : ProcessInfo.processInfo.systemUptime - Double(buffer.frameLength) / buffer.format.sampleRate
                writer.append(converted, capturedAt: uptime)
                if silence.feed(converted.monoSamples) {
                    Task { @MeetingMicrophoneControl [weak self] in await self?.microphoneSilent(token) }
                }
            }
        }
        engine.prepare()
        try MicrophoneAccess.start(engine)
        guard engine.isRunning else { throw NoInputDevice() }
        try MeetingMicrophoneStartup.verifyProcessing(requested: echo, enabled: input.isVoiceProcessingEnabled,
                                                    bypassed: input.isVoiceProcessingBypassed)
        installedRoute = tapRoute
        installedFormat = hardware
        log.notice("microphone tap started: hardware=\(hardware.sampleRate) Hz/\(hardware.channelCount) ch, echo=\(input.isVoiceProcessingEnabled), bypassed=\(input.isVoiceProcessingBypassed)")
    }

    nonisolated func defaultRouteDidChange() {
        Task { @MeetingMicrophoneControl in await self.checkConfiguration(of: nil) }
    }

    private func observeConfiguration() {
        let observed = engine
        configuration.observe(observed) { [weak self, weak observed] in
            guard let self, let observed else { return }
            await self.checkConfiguration(of: observed)
        }
    }

    private func checkConfiguration(of observed: AVAudioEngine?) async {
        guard writer != nil else { return }
        if let observed, observed !== engine { return }
        if starting { configurationChangedDuringStart = true; return }
        let route = MeetingMicrophoneRoute.current
        if observed == nil {
            guard route.defaultChangeNeedsRestart(from: installedRoute, running: engine.isRunning) else { return }
        } else if engine.isRunning, route == installedRoute {
            let input = engine.inputNode
            let processingUnchanged = !usingProcessedEngine || (input.isVoiceProcessingEnabled && !input.isVoiceProcessingBypassed)
            if processingUnchanged, installedFormat?.isEqual(input.inputFormat(forBus: 0)) == true { return }
        }
        removeTapAndStop()
        await recover()
    }

    private func recover(forceRaw: Bool = false) async {
        do { try await startMicrophone(forceRaw: forceRaw) }
        catch is CancellationError {}
        catch { log.error("microphone restart failed: \(error.localizedDescription)") }
    }

    private func removeTapAndStop() {
        admission?.close()
        admission = nil
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        installedFormat = nil
    }

    private func replaceEngine() {
        configuration.stop()
        removeTapAndStop()
        disableVoiceProcessing()
        engine = AVAudioEngine()
        observeConfiguration()
    }

    private func disableVoiceProcessing() {
        guard engine.inputNode.isVoiceProcessingEnabled else { return }
        do { try engine.inputNode.setVoiceProcessingEnabled(false) }
        catch { log.error("could not disable voice processing: \(error.localizedDescription)") }
    }

    private func microphoneSilent(_ token: UInt64) async {
        guard generation.accepts(token), !starting else { return }
        guard usingProcessedEngine else {
            health.set(.failed)
            log.error("microphone delivers only silence")
            return
        }
        log.error("microphone silent with voice processing; restarting without echo cancellation")
        replaceEngine()
        await recover(forceRaw: true)
    }
}
