import AVFoundation
import BackgroundTasks
import Observation
import os
import PennelMobileKit
import UIKit

/// How a recording ended. A meeting with a gap, a capture that stopped before the user did, or a failed save is
/// never presented as a complete recording.
struct MeetingFinish: Equatable, Sendable {
    var id: String
    var gaps: [MeetingGap]
    /// Why the recording stopped before **Stop opname**, e.g. the microphone did not come back after a call.
    var stoppedEarly: String?
    /// The transcript or audio could not be saved completely.
    var error: String?

    var isComplete: Bool { gaps.isEmpty && stoppedEarly == nil && error == nil }
}

/// Retains interruption notices for existing recordings and the native comparison UI. New recordings also keep
/// their capture diagnostics and completeness in the meeting folder.
final class MeetingInterruptionLog: @unchecked Sendable {
    struct Record: Codable, Equatable {
        var gaps: [MeetingGap]
        var stoppedEarly: String?
        var stoppedEarlyCode: String? = nil
    }

    private static let key = "meetingInterruptions"
    private let defaults: UserDefaults
    private let lock = NSLock()

    init(defaults: UserDefaults = AppPaths.defaults) {
        self.defaults = defaults
    }

    func record(for id: String) -> Record? {
        lock.withLock { load()[id] }
    }

    func save(_ record: Record, for id: String) {
        change { $0[id] = record }
    }

    func rename(from old: String, to new: String) {
        change { records in
            if let record = records.removeValue(forKey: old) { records[new] = record }
        }
    }

    func remove(_ id: String) {
        change { $0[id] = nil }
    }

    private func change(_ body: (inout [String: Record]) -> Void) {
        lock.withLock {
            var records = load()
            body(&records)
            defaults.set(try? JSONEncoder().encode(records), forKey: Self.key)
        }
    }

    private func load() -> [String: Record] {
        guard let data = defaults.data(forKey: Self.key) else { return [:] }
        return (try? JSONDecoder().decode([String: Record].self, from: data)) ?? [:]
    }
}

/// Keeps the app running while a stopped meeting is finished in the background (`BGContinuedProcessingTask`).
@MainActor
protocol FinishTask: AnyObject {
    func update(pending: UInt32)
    func end(success: Bool)
}

@MainActor
protocol FinishTaskScheduling: AnyObject {
    func begin() -> FinishTask?
}

/// The one recording meeting of the app: microphone → `MeetingRecorder` → core, the Live Activity, interruptions,
/// and finishing (last chunks, rename, summary) after **Stop opname**.
@MainActor
@Observable
final class MeetingRuntime: DictationMeetingHook {
    typealias Timers = DictationSession.Timers

    enum Phase { case idle, recording, interrupted, finishing }

    enum StartError: LocalizedError {
        case busy, microphone, syncBusy

        var errorDescription: String? {
            switch self {
            case .busy: "A meeting is already being recorded or finalized."
            case .microphone: "No microphone access. Allow microphone access in Settings."
            case .syncBusy: "Pennel is still syncing. Try again in a few seconds."
            }
        }
    }

    enum Message {
        static let interrupted = "Recording paused by a call. Pennel resumes after the call ends."
        static let couldNotResume = "The recording stopped after an interruption because the microphone did not return."
        static let microphoneLost = "Pennel cannot hear the microphone. Recording resumes when it returns."
        static let terminated = "The recording stopped because iOS closed Pennel."
        static let defaultTitle = "Meeting"

        static func code(for message: String?) -> String? {
            switch message {
            case interrupted, "Opname gepauzeerd door een oproep. Pennel neemt verder op zodra de oproep voorbij is.": "recording_paused"
            case couldNotResume, "De opname is na een onderbreking gestopt omdat de microfoon niet terugkwam.": "recording_resume_failed"
            case microphoneLost, "Pennel hoort de microfoon niet meer. De opname gaat verder zodra hij terug is.": "recording_microphone_lost"
            case terminated, "De opname is gestopt omdat iOS Pennel heeft afgesloten.": "recording_terminated"
            case "The audio recording could not be saved. Check available storage.", "De audio-opname kon niet worden bewaard. Controleer de vrije opslagruimte.": "storage"
            default: nil
            }
        }
    }

    static let shared = MeetingRuntime.live()

    private(set) var phase = Phase.idle
    /// Set from start until the meeting is finalised: sync must not push it before then (spec §5.6).
    private(set) var activeMeetingId: String?
    private(set) var title = ""
    private(set) var startedAt: Date?
    private(set) var transcript: [Segment] = []
    /// The live notes, kept here so a reopened live view starts from them instead of an empty field.
    private(set) var notes = ""
    private(set) var pending: UInt32 = 0
    private(set) var warning: String?
    private(set) var lastFinish: MeetingFinish?
    /// A meeting whose final save failed; it blocks a new recording until **Opnieuw bewaren** succeeds.
    private(set) var retainedId: String?

    /// The recording one and the one whose final save failed: sync must leave both alone.
    var unfinalisedMeetingIds: [String] { [activeMeetingId, retainedId].compactMap { $0 } }

    @ObservationIgnored var onFinished: (@MainActor (MeetingFinish) -> Void)?
    /// After a later write to a finished meeting (the automatic summary).
    @ObservationIgnored var onUpdated: (@MainActor (String) -> Void)?
    @ObservationIgnored var postprocessHandler: (@MainActor (MeetingFinish) async -> Void)?
    @ObservationIgnored private(set) var finishing: Task<Void, Never>?

    @ObservationIgnored private let engine: PennelEngine
    @ObservationIgnored private let syncGate: SyncHolding?
    @ObservationIgnored private let syncHoldTimeout: Duration
    @ObservationIgnored private let capture: AudioCapturing
    @ObservationIgnored private let audioSession: AudioSessionControlling
    @ObservationIgnored private let activity: RecordingActivityControlling
    @ObservationIgnored private let scheduler: FinishTaskScheduling
    @ObservationIgnored private let interruptions: MeetingInterruptionLog
    @ObservationIgnored private let keys: @Sendable () -> ApiKeys
    @ObservationIgnored private let clock: @Sendable () -> TimeInterval
    @ObservationIgnored private let timers: Timers
    @ObservationIgnored private let hasMicrophonePermission: @MainActor () -> Bool
    @ObservationIgnored private let languageModelReady: @MainActor (Settings) -> Bool
    @ObservationIgnored private var session: MeetingSession?
    @ObservationIgnored private var retained: MeetingSession?
    @ObservationIgnored private var recorder: MeetingRecorder?
    @ObservationIgnored private var originalTitle = ""
    @ObservationIgnored private var cancelTick: (() -> Void)?
    @ObservationIgnored private var finishTask: FinishTask?
    @ObservationIgnored private var observers: [NSObjectProtocol] = []
    @ObservationIgnored private var captureSequence: UInt64 = 0
    @ObservationIgnored private var pendingCaptureEvents: [MeetingCaptureEvent] = []
    @ObservationIgnored private var metadataError: String?
    @ObservationIgnored private var lastInputFormat: MicrophoneInputFormat?
    @ObservationIgnored private var lastRecordingTransport = RecordingTransport.unknown
    @ObservationIgnored private var captureEventsDropped: UInt64 = 0
    @ObservationIgnored private var retainedRecordingFinish: (durationMs: UInt64, complete: Bool)?
    @ObservationIgnored private var archiveOpeningError: String?

    private static let log = Logger(subsystem: "app.pennel.ios", category: "meeting")

    init(
        engine: PennelEngine,
        capture: AudioCapturing,
        audioSession: AudioSessionControlling,
        activity: RecordingActivityControlling,
        scheduler: FinishTaskScheduling,
        interruptions: MeetingInterruptionLog = MeetingInterruptionLog(),
        notifications: NotificationCenter = .default,
        keys: @escaping @Sendable () -> ApiKeys = { Keychain.apiKeys() },
        clock: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
        timers: @escaping Timers = DictationSession.runLoopTimers,
        hasMicrophonePermission: @escaping @MainActor () -> Bool = { AVAudioApplication.shared.recordPermission == .granted },
        languageModelReady: @escaping @MainActor (Settings) -> Bool = MeetingRuntime.languageModelReady,
        syncGate: SyncHolding? = nil,
        syncHoldTimeout: Duration = .seconds(15)
    ) {
        self.syncGate = syncGate
        self.syncHoldTimeout = syncHoldTimeout
        self.engine = engine
        self.capture = capture
        self.audioSession = audioSession
        self.activity = activity
        self.scheduler = scheduler
        self.interruptions = interruptions
        self.keys = keys
        self.clock = clock
        self.timers = timers
        self.hasMicrophonePermission = hasMicrophonePermission
        self.languageModelReady = languageModelReady
        observeSystem(notifications)
    }

    private static func live() -> MeetingRuntime {
        MeetingRuntime(
            engine: AppModel.shared.engine,
            capture: SharedMicrophone.shared.client(),
            audioSession: SharedAudioSession.shared.client(.meeting),
            activity: SystemRecordingActivity(),
            scheduler: SystemFinishScheduler(),
            syncGate: CloudSyncRunner.shared)
    }

    var isRecording: Bool { phase == .recording || phase == .interrupted }

    func interruption(for id: String) -> MeetingInterruptionLog.Record? {
        interruptions.record(for: id)
    }

    /// The folder name is the meeting id, so a rename in the library moves what is remembered about it.
    func renamed(from old: String, to new: String) {
        interruptions.rename(from: old, to: new)
    }

    func deleted(_ id: String) {
        interruptions.remove(id)
    }

    // MARK: Start

    /// Foreground only, after the microphone permission was asked: iOS does not let an app start recording from
    /// the background.
    func start(title requested: String) async throws {
        guard phase == .idle, retained == nil else { throw StartError.busy }
        guard hasMicrophonePermission() else { throw StartError.microphone }
        if let syncGate {
            guard await syncGate.hold(maxWait: syncHoldTimeout) else { throw StartError.syncBusy }
        }
        defer { syncGate?.release() }
        guard phase == .idle, retained == nil else { throw StartError.busy }
        try startHoldingSync(title: requested)
    }

    /// Runs without suspending, so no sync pass can start between creating the folder and setting `activeMeetingId`.
    private func startHoldingSync(title requested: String) throws {
        let keys = keys()
        try engine.validateMeetingSetup(keys: keys)
        let trimmed = requested.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = trimmed.isEmpty ? Message.defaultTitle : trimmed
        let session = try engine.startMeetingWith(title: name, attendees: [], keys: keys)
        let id = session.id()
        let tracks: [MeetingAudioTrack]
        do {
            tracks = try engine.prepareMeetingRecording(id: id, environment: RecordingEnvironment(
                appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown",
                build: Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "unknown",
                platform: "ios", osVersion: UIDevice.current.systemVersion
            ), roomRecording: true)
        } catch {
            if (try? session.stop()) != nil { try? engine.deleteMeeting(id: id) }
            throw error
        }
        captureSequence = 0
        pendingCaptureEvents = []
        metadataError = nil
        lastInputFormat = nil
        lastRecordingTransport = .unknown
        captureEventsDropped = 0
        retainedRecordingFinish = nil
        archiveOpeningError = nil

        var archive: MeetingArchiving?
        do {
            guard let track = tracks.first(where: { $0.source == .roomMicrophone }) else { throw MicCapture.Failure.unavailable }
            archive = try AACArchive(url: URL(fileURLWithPath: session.dir()).appendingPathComponent(track.relativePath))
        } catch {
            Self.log.error("archive: \(ErrorText.code(error), privacy: .public)")
            archiveOpeningError = "The audio recording could not be saved. Check available storage."
            warning = archiveOpeningError
        }
        let recorder = MeetingRecorder(sink: OthersTrack(session: session), archive: archive, clock: clock)
        do {
            try audioSession.activate()
            recorder.begin()
            try capture.start { recorder.feed($0) }
        } catch {
            Self.log.error("meeting start: \(ErrorText.code(error), privacy: .public)")
            capture.stop()
            audioSession.deactivate()
            recorder.finish()
            if (try? session.stop()) != nil { try? engine.deleteMeeting(id: id) }
            warning = nil
            throw error
        }

        self.session = session
        self.recorder = recorder
        originalTitle = name
        title = name
        startedAt = Date()
        transcript = []
        notes = ""
        pending = 0
        lastFinish = nil
        finishing = nil
        activeMeetingId = id
        phase = .recording
        recordCapture(.started)
        if archive == nil { recordCapture(.captureError, errorCode: "archive_open_failed") }
        activity.start(activityState)
        cancelTick = timers(1) { [weak self] in self?.tick() }
        Self.log.info("meeting recording")
    }

    /// The title can change while recording; the folder is renamed once, after the recording is saved.
    func rename(_ title: String) {
        guard isRecording else { return }
        self.title = title
    }

    /// Writes only a real change, so an untouched field can never overwrite what was typed earlier.
    func setNotes(_ text: String) {
        do {
            try saveLiveNotes(text)
        } catch {
            Self.log.error("live notes: \(ErrorText.code(error), privacy: .public)")
        }
    }

    func saveLiveNotes(_ text: String) throws {
        guard isRecording, let session else {
            throw PennelError.Cloud(code: "meeting_busy", message: "The recording is being finalized. Save your notes when finalization has finished.")
        }
        guard text != notes else { return }
        try session.setNotes(text: text)
        notes = text
    }

    // MARK: Stop

    /// **Stop opname**: finalises the meeting. Waits for the last chunks and the save, not for the summary.
    func stop() async {
        requestFinish(stoppedEarly: nil)
        await finishing?.value
    }

    /// One finish per recording: a second trigger (Stop opname after a failed resume, the Live Activity, a storage
    /// failure) waits for the first instead of replacing it.
    private func requestFinish(stoppedEarly: String?) {
        guard isRecording, finishing == nil else { return }
        phase = .finishing
        cancelTick?()
        cancelTick = nil
        finishing = Task { await finish(stoppedEarly: stoppedEarly) }
    }

    private func finish(stoppedEarly: String?) async {
        guard phase == .finishing, let session, let recorder else { return }
        capture.stop()
        audioSession.deactivate()
        let archiveError = recorder.finish()
        let durationMs = recorder.elapsedMs
        // Continued processing must be requested from the user's action while Pennel is in front; the request is
        // made at every stop and ended when finishing is done, so leaving the app keeps it going (spec §5.5).
        beginFinishTask()
        activity.update(activityState)

        let saved = await save(session)
        let health = session.health()
        var error = saved ?? health.terminalError ?? archiveError ?? archiveOpeningError
        var id = session.id()
        let captureComplete = recorder.gaps.isEmpty && stoppedEarly == nil && archiveError == nil && archiveOpeningError == nil
        if archiveError != nil { recordCapture(.captureError, errorCode: "archive_write_failed", elapsedMs: durationMs) }
        if stoppedEarly != nil { recordCapture(.captureError, errorCode: "capture_stopped_early", elapsedMs: durationMs) }
        recordCapture(.metrics, metrics: recorder.metrics, elapsedMs: durationMs)
        recordCapture(.stopped, elapsedMs: durationMs)
        do {
            try flushCaptureEvents(id: id)
            try engine.finishMeetingRecording(id: id, durationMs: durationMs, complete: captureComplete && error == nil)
        } catch let metadataFailure {
            Self.log.error("recording metadata: \(ErrorText.code(metadataFailure), privacy: .public)")
            self.metadataError = "The recording details could not be saved. Try saving again."
            if error == nil { error = self.metadataError }
        }
        if error == nil, title.trimmingCharacters(in: .whitespacesAndNewlines) != originalTitle,
           !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            do {
                id = try engine.renameMeeting(id: id, title: title)
            } catch {
                Self.log.error("rename after recording: \(ErrorText.code(error), privacy: .public)")
            }
        }
        if !session.isFinalized() || metadataError != nil {
            retained = session
            retainedId = id
            retainedRecordingFinish = (durationMs, captureComplete)
        }

        let finish = MeetingFinish(id: id, gaps: recorder.gaps, stoppedEarly: stoppedEarly, error: error)
        if !finish.gaps.isEmpty || stoppedEarly != nil {
            interruptions.save(.init(gaps: finish.gaps, stoppedEarly: stoppedEarly, stoppedEarlyCode: Message.code(for: stoppedEarly)), for: id)
        }
        self.session = nil
        self.recorder = nil
        transcript = []
        notes = ""
        pending = 0
        warning = nil
        startedAt = nil
        activeMeetingId = nil
        phase = .idle
        lastFinish = finish
        activity.end()
        Self.log.info("meeting finished: complete \(finish.isComplete, privacy: .public), gaps \(finish.gaps.count, privacy: .public)")
        onFinished?(finish)
        postprocess(finish)
    }

    /// `MeetingSession::stop` joins the transcription workers and can take a while; the pending count is
    /// published meanwhile for the live view and the background task.
    private func save(_ session: MeetingSession) async -> String? {
        let done = Flag()
        let stopping = Task.detached { () -> String? in
            defer { done.set() }
            do {
                try session.stop()
                return nil
            } catch {
                return ErrorText.message(error)
            }
        }
        while !done.isSet {
            pending = session.pending()
            finishTask?.update(pending: pending)
            try? await Task.sleep(for: .milliseconds(200))
        }
        return await stopping.value
    }

    /// Like the desktop: summarise when a language model is ready, never after a recording that stopped early or
    /// was not saved. Speaker recognition is not run on iPhone (one track, spec §4.1).
    private func postprocess(_ finish: MeetingFinish) {
        if let postprocessHandler {
            Task {
                await postprocessHandler(finish)
                endFinishTask(success: finish.error == nil)
            }
            return
        }
        guard finish.error == nil, finish.stoppedEarly == nil, languageModelReady(engine.settings()) else {
            endFinishTask(success: finish.error == nil)
            return
        }
        let engine = engine
        let keys = keys()
        let template = engine.settings().defaultTemplate
        let id = finish.id
        Task {
            let summarised = await Task.detached { (try? engine.summarizeMeeting(id: id, templateId: template, keys: keys)) != nil }.value
            if !summarised { Self.log.error("automatic summary failed") }
            onUpdated?(id)
            endFinishTask(success: true)
        }
    }

    /// Saves what the failed stop kept, without recording or transcribing again.
    func retrySave() async throws {
        guard let retained, let id = retainedId else { return }
        if !retained.isFinalized() { try await Task.detached { try retained.retryPersistence() }.value }
        try flushCaptureEvents(id: id)
        if let recording = retainedRecordingFinish {
            try engine.finishMeetingRecording(id: id, durationMs: recording.durationMs, complete: recording.complete)
        }
        self.retained = nil
        retainedId = nil
        retainedRecordingFinish = nil
        metadataError = nil
        let record = interruptions.record(for: id)
        let finish = MeetingFinish(id: id, gaps: record?.gaps ?? [], stoppedEarly: record?.stoppedEarly, error: nil)
        lastFinish = finish
        onFinished?(finish)
        postprocess(finish)
    }

    // MARK: Background finishing

    private func beginFinishTask() {
        guard finishTask == nil else { return }
        finishTask = scheduler.begin()
        finishTask?.update(pending: pending)
    }

    private func endFinishTask(success: Bool) {
        finishTask?.end(success: success)
        finishTask = nil
    }

    // MARK: Local recording diagnostics

    private func recordCapture(
        _ kind: CaptureEventKind, errorCode: String? = nil,
        metrics: MeetingRecorderMetrics? = nil, elapsedMs: UInt64? = nil
    ) {
        guard let id = activeMeetingId else { return }
        if let format = capture.inputFormat { lastInputFormat = format }
        if let port = AVAudioSession.sharedInstance().currentRoute.inputs.first?.portType {
            lastRecordingTransport = Self.recordingTransport(port)
        }
        if pendingCaptureEvents.count >= 256 {
            pendingCaptureEvents.removeFirst()
            captureEventsDropped += 1
        }
        captureSequence += 1
        pendingCaptureEvents.append(MeetingCaptureEvent(
            sequence: captureSequence, elapsedMs: elapsedMs ?? recorder?.elapsedMs ?? 0,
            kind: kind, source: .roomMicrophone,
            inputSampleRate: lastInputFormat?.sampleRate, tapSampleRate: nil, aggregateSampleRate: nil,
            inputChannels: lastInputFormat?.channels,
            inputInterleaved: lastInputFormat?.interleaved,
            transport: lastRecordingTransport,
            capturedFrames: nil, outputSamples: metrics?.outputSamples,
            insertedSilenceSamples: metrics?.insertedSilenceSamples, gapCount: metrics?.gapCount,
            rateMismatchCount: nil, droppedFrames: nil, droppedEvents: captureEventsDropped, errorCode: errorCode
        ))
        do {
            try flushCaptureEvents(id: id)
        } catch {
            metadataError = "The recording details could not be saved. Try saving again."
            warning = metadataError
            Self.log.error("capture metadata: \(ErrorText.code(error), privacy: .public)")
        }
    }

    private func flushCaptureEvents(id: String) throws {
        while let event = pendingCaptureEvents.first {
            try engine.recordMeetingCaptureEvent(id: id, event: event)
            pendingCaptureEvents.removeFirst()
        }
        metadataError = nil
    }

    static func recordingTransport(_ port: AVAudioSession.Port?) -> RecordingTransport {
        guard let port else { return .unknown }
        if [.builtInMic, .builtInReceiver, .builtInSpeaker].contains(port) { return .builtIn }
        if [.bluetoothHFP, .bluetoothA2DP, .bluetoothLE].contains(port) { return .bluetooth }
        if port == .usbAudio { return .usb }
        if [.headsetMic, .headphones, .lineIn, .lineOut].contains(port) { return .wired }
        if port == .HDMI { return .display }
        return .unknown
    }

    // MARK: While recording

    func tick() {
        guard isRecording, let session else { return }
        transcript = session.transcript()
        pending = session.pending()
        let health = session.health()
        if let terminal = health.terminalError {
            warning = terminal
            Self.log.error("meeting stopped after a storage failure")
            requestFinish(stoppedEarly: terminal)
            return
        }
        guard phase == .recording, let recorder else { return }
        if let format = capture.inputFormat, format != lastInputFormat { recordCapture(.formatChanged) }
        if recorder.stalledFor >= Self.stallRestartSeconds {
            restartStalledCapture(recorder)
        } else {
            warning = recorder.error ?? archiveOpeningError ?? metadataError ?? health.transcriptionWarning
        }
    }

    /// Samples stopped without an interruption (the engine died on a route change). The capture is restarted; the
    /// recorder turns the silent stretch into a gap, so the meeting is not reported complete.
    private static let stallRestartSeconds: TimeInterval = 2

    private func restartStalledCapture(_ recorder: MeetingRecorder) {
        warning = Message.microphoneLost
        Self.log.error("meeting capture stalled")
        capture.stop()
        recordCapture(.interruption, errorCode: "microphone_stalled")
        do {
            try audioSession.activate()
            try capture.start { recorder.feed($0) }
            recordCapture(.restarted)
        } catch {
            Self.log.error("meeting capture restart: \(ErrorText.code(error), privacy: .public)")
            requestFinish(stoppedEarly: Message.couldNotResume)
        }
    }

    private func interrupted() {
        guard phase == .recording, let recorder else { return }
        recorder.pause()
        recordCapture(.interruption)
        capture.stop()
        audioSession.deactivate()
        phase = .interrupted
        warning = Message.interrupted
        activity.update(activityState)
        Self.log.info("meeting interrupted")
    }

    /// After the call. When the microphone does not come back, the meeting is finished with what was recorded and
    /// marked as stopped early; `finishOnFailure` is false for the opportunistic retry when the app comes forward.
    private func resume(finishOnFailure: Bool) {
        guard phase == .interrupted, let recorder else { return }
        do {
            try audioSession.activate()
            try capture.start { recorder.feed($0) }
            recorder.resume()
        } catch {
            Self.log.error("meeting resume: \(ErrorText.code(error), privacy: .public)")
            capture.stop()
            audioSession.deactivate()
            guard finishOnFailure else { return }
            requestFinish(stoppedEarly: Message.couldNotResume)
            return
        }
        phase = .recording
        recordCapture(.restarted)
        warning = nil
        activity.update(activityState)
        Self.log.info("meeting resumed")
    }

    private var activityState: RecordingActivityState {
        let phase: RecordingActivityState.Phase = switch self.phase {
        case .interrupted: .interrupted
        case .finishing: .finishing
        case .idle, .recording: .recording
        }
        return RecordingActivityState(phase: phase, startedAt: startedAt ?? Date(), message: phase == .interrupted ? Message.interrupted : nil)
    }

    // MARK: DictationMeetingHook

    var isActive: Bool { isRecording }

    var elapsedMs: UInt64 { recorder?.elapsedMs ?? 0 }

    func setDictating(_ dictating: Bool) {
        recorder?.setDictating(dictating)
        session?.setDictating(dictating: dictating)
    }

    func markDictation(startMs: UInt64, endMs: UInt64) {
        session?.markDictation(startMs: startMs, endMs: endMs)
    }

    // MARK: System events

    private func observeSystem(_ center: NotificationCenter) {
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: nil) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  let type = AVAudioSession.InterruptionType(rawValue: raw)
            else { return }
            Self.onMain {
                switch type {
                case .began: self?.interrupted()
                // A recorder tries to come back even without `shouldResume` (that flag is meant for playback);
                // when iOS refuses, the meeting stops cleanly and says so.
                case .ended: self?.resume(finishOnFailure: true)
                @unknown default: break
                }
            }
        })
        observers.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: nil) { [weak self] _ in
            Self.onMain {
                self?.interrupted()
                self?.resume(finishOnFailure: true)
            }
        })
        observers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: nil) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self, self.isRecording else { return }
                self.recordCapture(.deviceChanged)
            }
        })
        // The end of an interruption is not always delivered; coming back to the app is a second chance.
        observers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: nil) { [weak self] _ in
            Self.onMain { self?.resume(finishOnFailure: false) }
        })
        observers.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: nil) { [weak self] _ in
            Self.onMain {
                guard let self, self.phase == .finishing else { return }
                self.beginFinishTask()
            }
        })
        observers.append(center.addObserver(forName: UIApplication.willTerminateNotification, object: nil, queue: nil) { [weak self] _ in
            Self.onMain {
                guard let self, self.isRecording else { return }
                self.capture.stop()
                self.recorder?.finish()
                // No finish runs after this, so the record is what tells the user after a relaunch that it is partial.
                if let id = self.activeMeetingId {
                    self.interruptions.save(.init(gaps: self.recorder?.gaps ?? [], stoppedEarly: Message.terminated, stoppedEarlyCode: "recording_terminated"), for: id)
                    self.recordCapture(.captureError, errorCode: "application_terminated")
                    self.recordCapture(.metrics, metrics: self.recorder?.metrics)
                    try? self.engine.finishMeetingRecording(id: id, durationMs: self.recorder?.elapsedMs ?? 0, complete: false)
                }
            }
        })
    }

    private nonisolated static func onMain(_ body: @escaping @MainActor @Sendable () -> Void) {
        if Thread.isMainThread {
            MainActor.assumeIsolated(body)
        } else {
            Task { @MainActor in body() }
        }
    }

    // MARK: Summary readiness

    /// The desktop's `summary_configuration_ready`.
    static func languageModelReady(_ settings: Settings) -> Bool {
        guard let account = settings.providers.first(where: { $0.id == settings.llm.providerId }),
              ProviderKinds.unavailableReason(account.kind) == nil,
              supports(kind: account.kind, task: .llm),
              !settings.llm.model.trimmingCharacters(in: .whitespaces).isEmpty
        else { return false }
        if requiresKey(kind: account.kind), !Keychain.hasKey(for: account.id) { return false }
        return account.kind != .appleIntelligence || FoundationLanguageModel.isAvailable
    }
}

private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false
    var isSet: Bool { lock.withLock { value } }
    func set() { lock.withLock { value = true } }
}

/// `BGContinuedProcessingTask` `app.pennel.ios.finish`: iOS shows the progress and keeps Pennel running while the
/// last chunks are transcribed and the summary is written. When iOS declines, finishing continues for as long as the
/// app runs, and otherwise **Opnieuw proberen** recovers failed chunks later.
@MainActor
final class SystemFinishScheduler: FinishTaskScheduling {
    static var identifier: String { (Bundle.main.bundleIdentifier ?? "app.pennel.ios") + ".finish" }
    private static let log = Logger(subsystem: "app.pennel.ios", category: "meeting")
    private static var current: SystemFinishTask?

    /// Must run before the app finishes launching.
    static func register() {
        guard !AppPaths.isQAProfile else { return }
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { task in
            guard let task = task as? BGContinuedProcessingTask else {
                task.setTaskCompleted(success: false)
                return
            }
            MainActor.assumeIsolated {
                guard let current else {
                    task.setTaskCompleted(success: true)
                    return
                }
                current.attach(task)
            }
        }
    }

    func begin() -> FinishTask? {
        guard !AppPaths.isQAProfile else { return nil }
        let request = BGContinuedProcessingTaskRequest(
            identifier: Self.identifier, title: ActivityCopy.current.text(.finishingRecording), subtitle: MeetingRuntime.Message.defaultTitle)
        request.strategy = .fail
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            Self.log.error("finish task: \(ErrorText.code(error), privacy: .public)")
            return nil
        }
        let task = SystemFinishTask()
        Self.current = task
        return task
    }

    fileprivate static func clear(_ task: SystemFinishTask) {
        if current === task { current = nil }
    }
}

@MainActor
private final class SystemFinishTask: FinishTask {
    private var task: BGContinuedProcessingTask?
    private var highest: UInt32 = 1
    private var pending: UInt32 = 0
    private var result: Bool?

    func attach(_ task: BGContinuedProcessingTask) {
        self.task = task
        task.expirationHandler = { [weak self] in
            Task { @MainActor in
                NotificationCenter.default.post(name: Notification.Name("PennelFinishTaskExpired"), object: nil)
                self?.end(success: false)
            }
        }
        task.progress.totalUnitCount = Int64(highest)
        report()
        if let result { complete(result) }
    }

    func update(pending: UInt32) {
        self.pending = pending
        highest = max(highest, pending)
        report()
    }

    func end(success: Bool) {
        guard result == nil else { return }
        result = success
        complete(success)
    }

    private func report() {
        guard let task else { return }
        task.progress.totalUnitCount = Int64(highest)
        task.progress.completedUnitCount = Int64(highest - min(pending, highest))
        let copy = ActivityCopy.current
        task.updateTitle(copy.text(.finishingRecording), subtitle: copy.finishing(pending: pending))
    }

    private func complete(_ success: Bool) {
        guard let task else { return }
        task.progress.completedUnitCount = task.progress.totalUnitCount
        task.setTaskCompleted(success: success)
        self.task = nil
        SystemFinishScheduler.clear(self)
    }
}
