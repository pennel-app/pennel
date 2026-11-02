@preconcurrency import AVFoundation
import CoreAudio

typealias AudioSink = @convention(c) (UnsafeMutableRawPointer?, UInt32, UnsafePointer<Float>?, Int, UInt64) -> Void

final class TrackWriter: @unchecked Sendable {
    private let track: UInt32
    private let sink: AudioSink
    private let context: UnsafeMutableRawPointer?
    private let started: TimeInterval
    private let queue: DispatchQueue
    private var file: AVAudioFile?
    private var pending: [Float] = []
    private var sent: UInt64 = 0
    private var pendingStart: UInt64 = 0
    private var archiveFailure: String?
    private let writeOverride: ((AVAudioPCMBuffer) throws -> Void)?
    private let healthLock = NSLock()
    private let enqueueLock = NSLock()
    private var accepting = true
    private var heard = false
    private let diagnostics: CaptureDiagnostics?
    private var trackName: String { track == 0 ? "microphone" : "system" }

    init(track: UInt32, sink: @escaping AudioSink, context: UnsafeMutableRawPointer?, started: TimeInterval, url: URL, diagnostics: CaptureDiagnostics? = nil, write: ((AVAudioPCMBuffer) throws -> Void)? = nil) throws {
        self.track = track
        self.diagnostics = diagnostics
        self.sink = sink
        self.context = context
        self.started = started
        self.writeOverride = write
        self.queue = DispatchQueue(label: "dev.pennel.native-track.\(track)")
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        file = try AVAudioFile(forWriting: url, settings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
            AVEncoderBitRateKey: 32_000,
        ], commonFormat: .pcmFormatFloat32, interleaved: false)
    }

    var heardSound: Bool { healthLock.withLock { heard } }
    var archiveError: String? { healthLock.withLock { archiveFailure } }

    func append(_ buffer: AVAudioPCMBuffer) {
        let offset = max(0, ProcessInfo.processInfo.systemUptime - started - Double(buffer.frameLength) / 16_000)
        append(buffer, at: offset)
    }

    func append(_ buffer: AVAudioPCMBuffer, capturedAt uptime: TimeInterval) {
        append(buffer, at: max(0, uptime - started))
    }

    func append(_ buffer: AVAudioPCMBuffer, at offset: TimeInterval) {
        let samples = buffer.monoSamples
        enqueueLock.withLock {
            guard accepting else { return }
            diagnostics?.count(track: trackName, key: "output_samples", value: UInt64(samples.count))
            if samples.contains(where: { $0 != 0 }) { healthLock.withLock { heard = true } }
            queue.async {
                let desired = UInt64(max(0, offset) * 16_000)
                let position = self.sent + UInt64(self.pending.count)
                // Preserve interruptions on the common session timeline, without expanding tiny callback jitter.
                if desired > position + 1_600 {
                    var remaining = desired - position
                    self.diagnostics?.count(track: self.trackName, key: "inserted_silence_samples", value: remaining)
                    self.diagnostics?.count(track: self.trackName, key: "gap_count", value: 1)
                    self.diagnostics?.event("gap", track: self.trackName, at: self.started + Double(position) / 16_000, fields: ["inserted_silence_samples": remaining, "start_sample": position])
                    while remaining > 0 {
                        let count = min(remaining, 32_000)
                        self.appendSamples(Array(repeating: 0, count: Int(count)))
                        remaining -= count
                    }
                }
                self.appendSamples(samples)
            }
        }
    }

    private func appendSamples(_ samples: [Float]) {
        guard !samples.isEmpty else { return }
        if let buffer = AVAudioPCMBuffer(pcmFormat: MonoResampler.format, frameCapacity: AVAudioFrameCount(samples.count)),
           let data = buffer.floatChannelData {
            buffer.frameLength = AVAudioFrameCount(samples.count)
            samples.withUnsafeBufferPointer { source in
                if let base = source.baseAddress { data[0].update(from: base, count: samples.count) }
            }
            do {
                if let writeOverride { try writeOverride(buffer) }
                else { try file?.write(from: buffer) }
            } catch {
                healthLock.withLock { archiveFailure = "The audio recording could not be saved. Check your free disk space." }
            }
        } else {
            healthLock.withLock { archiveFailure = "An audio block could not be prepared for recording." }
        }
        if pending.isEmpty { pendingStart = sent }
        pending.append(contentsOf: samples)
        if pending.count >= 16_000 { flush() }
    }

    private func flush() {
        guard !pending.isEmpty else { return }
        pending.withUnsafeBufferPointer { buffer in
            sink(context, track, buffer.baseAddress, buffer.count, pendingStart * 1_000 / 16_000)
        }
        sent += UInt64(pending.count)
        pending.removeAll(keepingCapacity: true)
    }

    func quiesce() {
        // A callback already in flight may retain this writer after the native handle is released.
        // Close admission before draining so it cannot call the released Rust context afterwards.
        enqueueLock.withLock { accepting = false }
    }

    func finish() -> String? {
        quiesce()
        return queue.sync {
            flush()
            file = nil
            return archiveError
        }
    }
}

struct MeetingCaptureHealth {
    let echoRequested: Bool
    private(set) var microphoneActive = false
    private(set) var microphoneError: String?
    private(set) var systemAudioAvailable = false
    private(set) var systemAudioError: String?
    private(set) var echoActive = false

    var echoWarning: String? {
        echoRequested && microphoneActive && !echoActive
            ? "Echo cancellation is not active. Use headphones."
            : nil
    }

    mutating func microphoneStarting() { microphoneActive = false; echoActive = false }
    mutating func microphoneStarted(echoActive: Bool) {
        microphoneActive = true
        microphoneError = nil
        self.echoActive = echoActive
    }
    mutating func microphoneFailed() {
        microphoneStarting()
        microphoneError = "Microphone recording was interrupted. Check your microphone."
    }
    mutating func systemAudioStarting() { systemAudioAvailable = false }
    mutating func systemAudioStarted() { systemAudioAvailable = true; systemAudioError = nil }
    mutating func systemAudioFailed() {
        systemAudioAvailable = false
        systemAudioError = "System audio is unavailable. Only the microphone is being recorded."
    }

    func bridgeObject(heardSystemAudio: Bool, archiveError: String?) -> [String: Any] {
        [
            "microphone_active": microphoneActive,
            "microphone_error": microphoneError as Any? ?? NSNull(),
            "system_audio_available": systemAudioAvailable,
            "system_audio_error": systemAudioError as Any? ?? NSNull(),
            "heard_system_audio": heardSystemAudio,
            "echo_cancellation": echoActive,
            "archive_error": archiveError as Any? ?? NSNull(),
            "capture_warning": echoWarning as Any? ?? NSNull(),
            "terminal_error": (archiveError ?? microphoneError) as Any? ?? NSNull(),
        ]
    }
}

final class NativeMeetingRecorder {
    private let log = Diagnostics("meeting")
    private var engine = AVAudioEngine()
    private let tap = SystemAudioTap()
    private let systemRoutes = SystemAudioRouteObserver()
    private var me: TrackWriter?
    private var others: TrackWriter?
    private var startedAt: Date?
    private let microphoneConfiguration = MeetingMicrophoneConfigurationObserver()
    private var pendingRebuild: DispatchWorkItem?
    private var forceRebuild = false
    private var rebuildGeneration = CaptureGeneration()
    private var recordingGeneration: UInt64 = 0
    private var quiescent = false
    private var captureWarning: String?
    private var finalArchiveError: String?
    private var finalHeardSystemAudio = false
    private var diagnostics: CaptureDiagnostics?
    private let silence = SilenceProbe()
    private let micSilence = SilenceProbe(threshold: 5)
    private var health = MeetingCaptureHealth(echoRequested: true)
    private var tapProcesses: (() -> [AudioObjectID])?
    var onSystemAudioSilent: (() -> Void)?

    var heardSystemAudio: Bool { others?.heardSound ?? finalHeardSystemAudio }
    var systemAudioAvailable: Bool { health.systemAudioAvailable }
    var isEchoCancellationEnabled: Bool { health.echoActive }
    var healthSnapshot: [String: Any] {
        var object = health.bridgeObject(heardSystemAudio: heardSystemAudio, archiveError: me?.archiveError ?? others?.archiveError ?? finalArchiveError)
        let warnings = [health.echoWarning, captureWarning].compactMap { $0 }
        object["capture_warning"] = warnings.isEmpty ? NSNull() : warnings.joined(separator: " ") as Any
        object["capture_diagnostics"] = diagnostics?.snapshot() ?? [:]
        return object
    }

    var elapsed: TimeInterval { startedAt.map { Date().timeIntervalSince($0) } ?? 0 }

    /// `tapProcesses` is re-evaluated on every (re)start so helpers that appear later are included.
    private var echoCancellation = true

    /// When the recording began, for mapping wall-clock moments (dictation) onto the meeting timeline.
    var startDate: Date? { startedAt }

    func start(directory: URL, sink: @escaping AudioSink, context: UnsafeMutableRawPointer?, tapProcesses: (() -> [AudioObjectID])? = nil, echoCancellation: Bool = true, microphoneURL: URL? = nil, systemURL: URL? = nil) throws {
        quiescent = false
        self.tapProcesses = tapProcesses
        self.echoCancellation = echoCancellation
        health = MeetingCaptureHealth(echoRequested: echoCancellation)
        let started = ProcessInfo.processInfo.systemUptime
        startedAt = Date()
        recordingGeneration &+= 1
        captureWarning = nil
        finalArchiveError = nil
        finalHeardSystemAudio = false
        let diagnostics = CaptureDiagnostics(started: started)
        self.diagnostics = diagnostics
        me = try TrackWriter(track: 0, sink: sink, context: context, started: started, url: microphoneURL ?? directory.appendingPathComponent("ik.m4a"), diagnostics: diagnostics)
        others = try TrackWriter(track: 1, sink: sink, context: context, started: started, url: systemURL ?? directory.appendingPathComponent("anderen.m4a"), diagnostics: diagnostics)
        // Device setup can stop the engine before system-audio initialization returns.
        observeMicrophoneConfiguration()
        do { try startMicrophone() } catch { _ = stop(); throw error }
        let generation = recordingGeneration
        systemRoutes.observe { [weak self] in
            guard let self, self.recordingGeneration == generation else { return }
            self.scheduleSystemAudioRebuild(reason: "output_route_changed")
        }
        startSystemAudio()
    }

    func quiesce() {
        guard !quiescent else { return }
        quiescent = true
        microphoneConfiguration.stop()
        systemRoutes.stop()
        recordingGeneration &+= 1
        pendingRebuild?.cancel()
        rebuildGeneration.stop()
        pendingRebuild = nil
        forceRebuild = false
        tap.onConfigurationChange = nil
        tap.onInvalidBuffer = nil
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        disableVoiceProcessing()
        tap.stop()
        me?.quiesce()
        others?.quiesce()
        health.microphoneStarting()
        health.systemAudioStarting()
    }

    func stop() -> String? {
        quiesce()
        let meError = me?.finish()
        let othersError = others?.finish()
        return completeStop(meError: meError, othersError: othersError)
    }

    func finishingTracks() -> (TrackWriter?, TrackWriter?) {
        quiesce()
        return (me, others)
    }

    func completeStop(meError: String?, othersError: String?) -> String? {
        finalArchiveError = meError ?? othersError
        finalHeardSystemAudio = others?.heardSound ?? finalHeardSystemAudio
        if let error = meError ?? othersError { captureWarning = error }
        diagnostics?.event("stopped", track: "all", fields: ["archive_complete": meError == nil && othersError == nil])
        me = nil
        others = nil
        startedAt = nil
        tapProcesses = nil
        return meError ?? othersError
    }

    private func startMicrophone() throws {
        guard let me else { return }
        try MicrophoneAccess.ensureGranted()
        health.microphoneStarting()
        echoCancellation = try MeetingMicrophoneStartup.start(
            echoCancellation: echoCancellation,
            attempt: { try self.startMicrophone(echoCancellation: $0, writer: me) },
            reset: {
                self.log.notice("echo-enabled microphone start failed; retrying with a fresh engine without voice processing")
                self.replaceMicrophoneEngine()
            }
        )
        health.microphoneStarted(echoActive: engine.inputNode.isVoiceProcessingEnabled)
    }

    private func startMicrophone(echoCancellation: Bool, writer: TrackWriter) throws {
        let input = engine.inputNode
        // Voice-processing reconfiguration can temporarily clear the hardware-side format.
        let hardwareFormat = input.inputFormat(forBus: 0)
        guard AudioInput.isUsable(sampleRate: hardwareFormat.sampleRate, channels: hardwareFormat.channelCount) else {
            throw NoInputDevice()
        }
        if echoCancellation {
            // Apple's echo canceller removes what the speakers play from the mic; keep other audio at full volume.
            do {
                try input.setVoiceProcessingEnabled(true)
                input.voiceProcessingOtherAudioDuckingConfiguration =
                    AVAudioVoiceProcessingOtherAudioDuckingConfiguration(enableAdvancedDucking: false, duckingLevel: .min)
                // Voice processing is a duplex unit: without a running output side the mic delivers only zeros.
                engine.mainMixerNode.outputVolume = 0
            } catch {
                log.error("voice processing unavailable: \(error.localizedDescription)")
            }
        } else {
            disableVoiceProcessing()
        }
        log.notice("microphone tap starting: hardware=\(hardwareFormat.sampleRate) Hz, channels=\(hardwareFormat.channelCount), echo=\(input.isVoiceProcessingEnabled)")
        let resampler = MonoResampler(from: hardwareFormat)
        let diagnostics = diagnostics
        diagnostics?.format(track: "microphone", format: hardwareFormat, transport: CaptureDeviceTransport.input(input.audioUnit))
        let micSilence = micSilence
        micSilence.reset()
        input.removeTap(onBus: 0)
        // After a Bluetooth switch the cached client/output format can differ from the hardware.
        input.installTap(onBus: 0, bufferSize: 4096, format: hardwareFormat) { [weak self] buffer, time in
            guard let converted = resampler.convert(buffer) else { return }
            diagnostics?.count(track: "microphone", key: "captured_frames", value: UInt64(buffer.frameLength))
            let uptime = time.isHostTimeValid ? AVAudioTime.seconds(forHostTime: time.hostTime)
                : ProcessInfo.processInfo.systemUptime - Double(buffer.frameLength) / buffer.format.sampleRate
            writer.append(converted, capturedAt: uptime)
            if micSilence.feed(converted.monoSamples) {
                Task { @MainActor in self?.microphoneSilent() }
            }
        }
        engine.prepare()
        try MicrophoneAccess.start(engine)
        guard engine.isRunning else { throw NoInputDevice() }
    }

    private func observeMicrophoneConfiguration() {
        microphoneConfiguration.observe(engine) { [weak self] in
            MainActor.assumeIsolated { self?.restartMicrophone() }
        }
    }

    private func replaceMicrophoneEngine() {
        microphoneConfiguration.stop()
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        disableVoiceProcessing()
        // A failed voice-processing unit can leave the engine unusable even after it is disabled.
        engine = AVAudioEngine()
        observeMicrophoneConfiguration()
    }

    private func disableVoiceProcessing() {
        guard engine.inputNode.isVoiceProcessingEnabled else { return }
        do {
            try engine.inputNode.setVoiceProcessingEnabled(false)
        } catch {
            log.error("could not disable voice processing: \(error.localizedDescription)")
        }
    }

    /// Exact zeros from the mic mean the capture path is broken (seen with voice processing);
    /// recording without echo cancellation beats losing the user's side of the meeting.
    private func microphoneSilent() {
        guard echoCancellation, startedAt != nil else {
            log.error("microphone delivers only silence")
            if startedAt != nil { health.microphoneFailed() }
            return
        }
        log.error("microphone silent with voice processing; restarting without echo cancellation")
        echoCancellation = false
        replaceMicrophoneEngine()
        do {
            try startMicrophone()
        } catch {
            health.microphoneFailed()
            log.error("microphone restart failed: \(String(describing: error))")
        }
    }

    private func restartMicrophone() {
        guard startedAt != nil else { return }
        engine.stop()
        do {
            try startMicrophone()
            log.notice("microphone restarted after configuration change")
        } catch {
            health.microphoneFailed()
            log.error("microphone restart failed: \(String(describing: error))")
        }
    }

    private func startSystemAudio() {
        guard let others else { return }
        health.systemAudioStarting()
        silence.reset()
        let generation = recordingGeneration
        do {
            let format = try tap.prepare(processes: tapProcesses?())
            let resampler = MonoResampler(from: format)
            let timing = CaptureTimingProbe()
            let diagnostics = diagnostics
            diagnostics?.format(track: "system", format: format, transport: tap.transport,
                                tapRate: tap.tapSampleRate, aggregateRate: tap.aggregateSampleRate)
            tap.onConfigurationChange = { [weak self] in
                Task { @MainActor in
                    guard let self, self.recordingGeneration == generation else { return }
                    self.scheduleSystemAudioRebuild(reason: "configuration_changed")
                }
            }
            tap.onInvalidBuffer = { [weak self] in
                guard timing.fail() else { return }
                diagnostics?.event("error", track: "system", fields: ["error_code": "invalid_buffer_layout"])
                Task { @MainActor in
                    guard let self, self.recordingGeneration == generation else { return }
                    self.systemAudioProblem(reason: "invalid_buffer_layout")
                }
            }
            let silence = silence
            try tap.start { [weak self] buffer, uptime in
                guard !timing.blocked else { return }
                guard let converted = resampler.convert(buffer) else {
                    guard timing.fail() else { return }
                    diagnostics?.event("error", track: "system", fields: ["error_code": "resampling_failed"])
                    Task { @MainActor in
                        guard let self, self.recordingGeneration == generation else { return }
                        self.systemAudioProblem(reason: "resampling_failed")
                    }
                    return
                }
                diagnostics?.count(track: "system", key: "captured_frames", value: UInt64(buffer.frameLength))
                if timing.observe(start: uptime, inputDuration: Double(buffer.frameLength) / buffer.format.sampleRate,
                                  outputSamples: Int(converted.frameLength)) {
                    diagnostics?.count(track: "system", key: "rate_mismatch_count", value: 1)
                    diagnostics?.event("error", track: "system", at: uptime, fields: ["error_code": "capture_rate_mismatch"])
                    Task { @MainActor in
                        guard let self, self.recordingGeneration == generation else { return }
                        self.systemAudioProblem(reason: "capture_rate_mismatch")
                    }
                    return
                }
                others.append(converted, capturedAt: uptime)
                if silence.feed(converted.monoSamples) {
                    Task { @MainActor in
                        guard let self, self.recordingGeneration == generation else { return }
                        self.systemAudioSilent()
                    }
                }
            }
            health.systemAudioStarted()
        } catch {
            diagnostics?.event("error", track: "system", fields: ["error_code": "system_capture_start_failed"])
            log.error("system audio tap failed: \(String(describing: error))")
            health.systemAudioFailed()
        }
    }

    private func systemAudioProblem(reason: String) {
        health.systemAudioFailed()
        captureWarning = "System audio was interrupted by a timing problem. Check the recording."
        scheduleSystemAudioRebuild(reason: reason, force: true)
    }

    private func scheduleSystemAudioRebuild(reason: String, force: Bool = false) {
        guard startedAt != nil else { return }
        forceRebuild = forceRebuild || force
        if force || !tap.configurationUnchangedAndFlowing() { tap.suspendDelivery() }
        pendingRebuild?.cancel()
        let rebuildToken = rebuildGeneration.begin()
        let generation = recordingGeneration
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.startedAt != nil, self.recordingGeneration == generation,
                  self.rebuildGeneration.accepts(rebuildToken) else { return }
            self.pendingRebuild = nil
            let forced = self.forceRebuild
            self.forceRebuild = false
            if !forced && self.tap.configurationUnchangedAndFlowing() { return }
            self.diagnostics?.event("rebuild", track: "system", fields: ["reason": reason])
            self.restartSystemAudio()
        }
        pendingRebuild = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5, execute: work)
    }

    /// Some apps play call audio from another process (FaceTime via avconferenced), so an app-only
    /// tap can stay silent; fall back to all system audio before telling the user.
    private func systemAudioSilent() {
        if tapProcesses != nil {
            log.notice("app tap silent; falling back to global system audio")
            tapProcesses = nil
            restartSystemAudio()
        } else {
            onSystemAudioSilent?()
        }
    }

    private func restartSystemAudio() {
        guard startedAt != nil else { return }
        tap.stop()
        startSystemAudio()
        log.notice("system audio restarted after output change")
    }
}

/// Thread-safe wrapper so the IOProc can feed the silence watch.
private final class SilenceProbe: @unchecked Sendable {
    private let lock = NSLock()
    private let threshold: TimeInterval
    private var watch: SilenceWatch
    private var samples = 0

    init(threshold: TimeInterval = 20) {
        self.threshold = threshold
        watch = SilenceWatch(threshold: threshold)
    }

    func reset() {
        lock.withLock {
            watch = SilenceWatch(threshold: threshold)
            samples = 0
        }
    }

    func feed(_ chunk: [Float]) -> Bool {
        lock.withLock {
            samples += chunk.count
            return watch.feed(chunk, at: Double(samples) / 16_000)
        }
    }
}

private final class CaptureTimingProbe: @unchecked Sendable {
    private var monitor = CaptureRateMonitor()
    private(set) var blocked = false
    func fail() -> Bool {
        guard !blocked else { return false }
        blocked = true
        return true
    }
    func observe(start: TimeInterval, inputDuration: TimeInterval, outputSamples: Int) -> Bool {
        if monitor.observe(start: start, inputDuration: inputDuration, outputSamples: outputSamples) { return fail() }
        return false
    }
}
