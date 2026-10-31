import AVFoundation
import ActivityKit
import Foundation
import PennelMobileKit
import Speech
import UIKit
import WidgetKit

@MainActor
public final class PennelNativeRuntime {
    public static let shared = PennelNativeRuntime()
    private var bootstrapped = false
    private var model: AppModel { AppModel.shared }
    private lazy var dictation = NativeDictationRecorder(engine: model.engine)
    private var revision: UInt64 = 0
    private var previousState = ""
    private var listeners: [UUID: @MainActor (String) -> Void] = [:]
    private var timer: Timer?
    private var observations: [NSObjectProtocol] = []
    private var cachedAccount: CloudAccount?
    private var pendingRecovery: NativeRecoveryDisplay?
    private var pendingLogin: CloudLoginStart?
    private var loginExpiry: Date?
    private let loginGeneration = NativeLoginGeneration()
    private let loginStarts = NativeLoginStartQueue()
    private var navigation: [String: Any]?
    private var navigationSequence: UInt64 = 0
    private var lastNavigationRequest = ""
    private var player: AVAudioPlayer?
    private var playbackSession: AudioSessionControlling?
    private let recognitionTransport = NativeRecognitionTransport()
    private var recognitionJobs: [String: SpeakerRecognitionJob] = [:]
    private var recognitionWatches: [String: Task<Void, Never>] = [:]
    private var finishBudgetGeneration: UInt64 = 0
    private var settingsTail: Task<Void, Never>?
    private lazy var inferenceRequests = NativeInferenceRequests(engine: model.engine)
    private var inferenceKeyRevisions: [String: UInt64] = [:]
    private var lastPublishedLanguage: String?
    private var keyboardWriting: KeyboardWritingServer?
    private var feedbackPreview: KeyboardFeedbackPreview?
    private weak var feedbackPreviewView: UIView?
    private static let automaticConsent = "pennel.mobile.cloudSpeakerAutomaticConsent"

    private init() {}

    @discardableResult
    public func bootstrap() throws -> String {
        if !bootstrapped {
            bootstrapped = true
            _ = model
            NativeRecognitionReader.discardStaleSnapshots()
            SystemSessionActivity.endLeftovers()
            SystemRecordingActivity.endLeftovers()
            DictationSession.purgeExpiredResult()
            SystemFinishScheduler.register()
            CloudSyncBackground.register()
            CloudSyncRunner.shared.attach(to: MeetingRuntime.shared)
            MeetingRuntime.shared.postprocessHandler = { [weak self] finish in await self?.postprocess(finish) }
            dictation.changed = { [weak self] in self?.publish() }
            observeLifecycle()
            publishKeyboardPreferences()
            keyboardWriting = KeyboardWritingServer(engine: model.engine, authorization: { [weak self] in
                guard let self else { return "" }
                let keys = self.inferenceKeyRevisions.sorted { $0.key < $1.key }.map { "\($0.key):\($0.value)" }.joined(separator: ";")
                return self.accountEpoch() + "|" + keys
            })
            keyboardWriting?.start()
            CloudSyncRunner.shared.enterForeground()
        }
        return try snapshot()
    }

    public func getSnapshot() throws -> String {
        if !bootstrapped { return try bootstrap() }
        return try snapshot()
    }

    func observe(_ listener: @escaping @MainActor (String) -> Void) -> UUID {
        let id = UUID()
        listeners[id] = listener
        if timer == nil {
            timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.publish() }
            }
        }
        if let value = try? getSnapshot() { listener(value) }
        return id
    }

    func removeObserver(_ id: UUID) {
        listeners[id] = nil
        if listeners.isEmpty { timer?.invalidate(); timer = nil }
    }

    public func startDictation(practice: Bool = false, navigate: Bool = true) async throws {
        _ = try bootstrap()
        if !practice && navigate { AppRouter.shared.requestDictation() }
        try await dictation.start(practice: practice)
        publish()
    }

    public func startSession() async throws {
        _ = try bootstrap()
        guard await DictationSession.shared.start() else {
            throw PennelBridgeError(DictationSession.shared.startErrorCode ?? "session_unavailable", DictationSession.shared.startError ?? "The dictation session could not start.")
        }
        AppRouter.shared.showsSessionReturn = true
        requestNavigation("dictation")
        publish()
    }

    public func stopSession() { DictationSession.shared.stop(); publish() }

    public func startMeeting(title: String, navigate: Bool = true) async throws {
        _ = try bootstrap()
        guard await MicCapture.requestPermission() else { throw PennelBridgeError("microphone_denied", "Allow microphone access in Settings.") }
        try await MeetingRuntime.shared.start(title: title)
        if navigate { AppRouter.shared.requestLiveMeeting() }
        publish()
    }

    public func stopMeeting() async {
        await MeetingRuntime.shared.stop()
        if let id = MeetingRuntime.shared.lastFinish?.id { requestNavigation("meeting", meetingId: id) }
        publish()
    }

    public func open(_ url: URL) -> Bool {
        if Self.acceptsSyncReturn(url, scheme: Bundle.main.object(forInfoDictionaryKey: "PennelURLScheme") as? String) {
            requestNavigation("sync")
            publish()
            return true
        }
        guard DictationSession.isStartLink(url) else { return false }
        Task { try? await startSession() }
        return true
    }

    nonisolated static func acceptsSyncReturn(_ url: URL, scheme: String?) -> Bool {
        guard let scheme, ["pennel", "pennel-preview"].contains(scheme) else { return false }
        return url.scheme == scheme && url.host == "sync" && url.path.isEmpty && url.query == nil
            && url.fragment == nil && url.user == nil && url.password == nil && url.port == nil
    }

    public func invoke(method: String, payload: String) async throws -> String {
        _ = try bootstrap()
        let input = BridgePayload(values: try BridgeJSON.object(payload))
        let engine = model.engine
        defer { publish() }
        switch method {
        case "runtime.capabilities": return try BridgeJSON.encode(capabilities())
        case "navigation.consume":
            guard let sequence = UInt64(exactly: input.number("sequence")) else { throw PennelBridgeError("invalid_request", "Invalid navigation sequence.") }
            if sequence == navigationSequence { navigation = nil }
            return "null"
        case "permissions.microphone": return try BridgeJSON.encode(["granted": await MicCapture.requestPermission()])
        case "system.openSettings":
            if let url = URL(string: UIApplication.openSettingsURLString) { await UIApplication.shared.open(url) }
            return "null"
        case "ui.feedback":
            guard let event = FeedbackEvent(rawValue: try input.required("event")), event != .keyPress else { throw PennelBridgeError("invalid_request", "Unknown feedback event.") }
            HapticFeedback.play(event)
            return "null"
        case "keyboard.feedbackPreview":
            guard let profile = KeyboardFeedbackPreviewProfile(rawValue: try input.required("profile")) else { throw PennelBridgeError("invalid_request", "Unknown feedback sample.") }
            guard let view = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
                .filter({ $0.activationState == .foregroundActive }).flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController?.view else {
                throw PennelBridgeError("unavailable", "Open Pennel to preview keyboard feedback.")
            }
            if feedbackPreviewView !== view {
                feedbackPreview?.stop()
                feedbackPreview = KeyboardFeedbackPreview(view: view)
                feedbackPreviewView = view
            }
            if input.flag("prepare_only") { feedbackPreview?.prepare(profile: profile) }
            else { feedbackPreview?.play(profile: profile) }
            return "null"
        case "keyboard.feedbackTrial":
            let profile: KeyboardFeedbackTrialProfile?
            if let value = input.values["profile"] as? String {
                guard let parsed = KeyboardFeedbackTrialProfile(rawValue: value), KeyboardPreferences.typingFeedbackEnabled(in: KeyboardLayout.groupDefaults) else {
                    throw PennelBridgeError("invalid_request", "Enable typing feedback before trying a sample.")
                }
                profile = parsed
            } else if input.values["profile"] is NSNull { profile = nil }
            else { throw PennelBridgeError("invalid_request", "Choose a feedback sample or end the trial.") }
            KeyboardPreferences.setFeedbackTrialProfile(profile, in: KeyboardLayout.groupDefaults)
            KeyboardPreferences.mirrorKeyboardFeedback(from: KeyboardLayout.groupDefaults, to: UserDefaults.standard)
            return try BridgeJSON.encode(keyboardSettings())
        case "keyboard.learning.remove":
            let word = try input.required("word").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !word.isEmpty, word.count <= 64 else { throw PennelBridgeError("invalid_request", "Enter a word of up to 64 characters.") }
            let defaults = KeyboardLayout.groupDefaults
            KeyboardPreferences.forgetLearnedWord(word, in: defaults)
            return try BridgeJSON.encode(keyboardSettings())
        case "dictation.start": try await startDictation(practice: input.flag("practice"), navigate: false); return try snapshot()
        case "dictation.stop":
            if let id = input.values["session_id"] as? String { try await dictation.stopPractice(sessionID: id) }
            else {
                guard !dictation.isPractice else { throw PennelBridgeError("dictation_owned", "Finish practice in its own screen.") }
                await dictation.stop()
            }
            return try snapshot()
        case "dictation.cancel":
            if let id = input.values["session_id"] as? String { try dictation.cancelPractice(sessionID: id) }
            else {
                guard !dictation.isPractice else { throw PennelBridgeError("dictation_owned", "Cancel practice in its own screen.") }
                dictation.cancel()
            }
            return try snapshot()
        case "dictation.verificationAvailability": return try BridgeJSON.encode(BridgeJSON.cleanupVerification(engine.cleanupVerificationAvailability()))
        case "dictation.copy":
            guard let text = input.values["text"] as? String else { throw PennelBridgeError("invalid_request", "Missing text.") }
            copy(text, html: input.values["html"] as? String)
            return "null"
        case "dictation.history":
            return try await background { try BridgeJSON.encode($0.history(limit: 200).map(BridgeJSON.history)) }
        case "dictation.retry":
            let id = try input.required("id")
            return try await background { engine in
                let entry = try engine.historyEntry(id: id)
                if entry.status == .unsaved { try engine.saveHistory(id: id); return try BridgeJSON.encode(["text": entry.text, "history_id": id]) }
                let result = try engine.retry(historyId: id, keys: Keychain.apiKeys())
                return try BridgeJSON.encode(["text": result.text, "history_id": result.historyId])
            }
        case "dictation.deleteHistory":
            let id = try input.required("id")
            return try await background { try $0.deleteHistory(id: id); return "null" }
        case "session.start": try await startSession(); return try snapshot()
        case "session.stop": stopSession(); return try snapshot()
        case "settings.read": return try BridgeJSON.encode(settingsProjection())
        case "settings.update": return try await settingsOperation { try await self.updateSettings(input) }
        case "settings.keyboard": return try BridgeJSON.encode(keyboardSettings())
        case "inference.protocols": return try BridgeJSON.encode(inferenceProtocols().map(BridgeJSON.inferenceProtocol))
        case "inference.availability":
            let capability = try BridgeJSON.inferenceCapability(input.required("capability"))
            return try BridgeJSON.encode(BridgeJSON.inferenceAvailability(engine.inferenceAvailability(capability: capability)))
        case "inference.decide", "inference.rerank":
            let capability: InferenceCapability = method == "inference.decide" ? .decision : .reranking
            let id = try input.required("request_id")
            guard let request = input.values["request"] as? [String: Any] else { throw PennelBridgeError("invalid_request", "Missing inference request.") }
            let requestJSON = try BridgeJSON.encode(request)
            return try await performInference(id: id, capability: capability) { engine, cancellation in
                if capability == .decision { return try engine.decideJson(requestJson: requestJSON, cancellation: cancellation, keys: Keychain.apiKeys()) }
                return try engine.rerankJson(requestJson: requestJSON, cancellation: cancellation, keys: Keychain.apiKeys())
            }
        case "inference.verify":
            let capability = try BridgeJSON.inferenceCapability(input.required("capability"))
            return try await performInference(id: input.required("request_id"), capability: capability) { engine, cancellation in
                try BridgeJSON.encode(BridgeJSON.inferenceVerification(engine.verifyInference(capability: capability, cancellation: cancellation, keys: Keychain.apiKeys())))
            }
        case "inference.cancel": return try BridgeJSON.encode(inferenceRequests.cancel(id: input.required("request_id")))
        case "providers.add":
            let kind = try BridgeJSON.kind(input.required("kind"))
            if let reason = ProviderKinds.unavailableReason(kind) { throw PennelBridgeError("unsupported", reason) }
            return try await inferenceSettingsOperation {
                let result = try await self.background { engine in
                    var providers = engine.settings().providers
                    if !ProviderKinds.isSingleton(kind) || !providers.contains(where: { $0.kind == kind }) {
                        providers.append(newAccount(kind: kind, existing: providers))
                    }
                    return try engine.patchMobileSettings(json: BridgeJSON.encode(["providers": providers.map(BridgeJSON.provider)]))
                }
                self.model.reloadSettings()
                return result
            }
        case "providers.remove":
            let id = try input.required("provider_id", alternative: "id")
            return try await inferenceSettingsOperation {
                let result = try await self.background { engine in
                    let providers = engine.settings().providers
                    guard providers.contains(where: { $0.id == id }) else { throw PennelBridgeError("not_found", "Provider not found.") }
                    guard providers.count > 1 else { throw PennelBridgeError("last_provider", "Keep at least one provider.") }
                    return try engine.patchMobileSettings(json: BridgeJSON.encode(["providers": providers.filter { $0.id != id }.map(BridgeJSON.provider)]))
                }
                self.model.reloadSettings()
                guard Keychain.set(key: nil, for: id) else { throw PennelBridgeError("credential_store", "The provider was removed, but its saved key could not be removed from Keychain.") }
                return result
            }
        case "providers.setKey":
            let id = try input.required("provider_id")
            guard model.settings.providers.contains(where: { $0.id == id }) else { throw PennelBridgeError("not_found", "Provider not found.") }
            guard let key = input.values["key"] as? String else { throw PennelBridgeError("invalid_request", "Missing key value.") }
            let saved = inferenceRequests.changingCredentials {
                guard Keychain.set(key: key.isEmpty ? nil : key, for: id) else { return false }
                inferenceKeyRevisions[id, default: 0] += 1
                return true
            }
            guard saved else { throw PennelBridgeError("credential_store", "The key could not be saved in Keychain.") }
            return "null"
        case "providers.test":
            let task = Self.providerTask(input.text("task"))
            let selected = task == .llm ? model.settings.llm : task == .meetingStt ? model.settings.meetingStt : model.settings.dictationStt
            let id = input.text("provider_id", default: selected.providerId)
            guard let provider = model.settings.providers.first(where: { $0.id == id }) else { throw PennelBridgeError("not_found", "Provider not found.") }
            if let reason = ProviderKinds.unavailableReason(provider.kind) { throw PennelBridgeError("unsupported", reason) }
            if input.flag("verify_task") {
                return try await settingsOperation {
                    let settings = self.model.engine.settings()
                    let choice = task == .llm ? settings.llm : task == .meetingStt ? settings.meetingStt : settings.dictationStt
                    guard choice.providerId == id else { throw PennelBridgeError("provider_not_selected", "Select this provider for the task before testing it.") }
                    if let expected = input.values["expected_model"] as? String, expected != choice.model {
                        throw PennelBridgeError("provider_changed", "The selected model changed. Run the test again with the current choice.")
                    }
                    return try await self.background { engine in
                        let result = try testProviderTask(engine: engine, task: task, keys: Keychain.apiKeys())
                        return try BridgeJSON.encode(["ok": result.verified, "message": result.message])
                    }
                }
            }
            let nl = uiLanguage() == "nl"
            if provider.kind == .appleSpeech {
                let locale = model.settings.language.map(Locale.init(identifier:)) ?? .current
                let supported = await DictationTranscriber.supportedLocale(equivalentTo: locale) != nil
                return try BridgeJSON.encode(["ok": supported, "message": supported ? (nl ? "Deze taal wordt ondersteund. Test bij Modellen of spraakherkenning met het audiovoorbeeld werkt." : "This language is supported. Test recognition with the audio sample in Models.") : (nl ? "Apple-dicteren ondersteunt deze taal niet op dit toestel." : "Apple dictation does not support this language on this device.")])
            }
            if provider.kind == .appleIntelligence {
                let availability = intelligenceAvailability()
                return try BridgeJSON.encode(["ok": availability.status == "available", "message": availability.reason ?? (nl ? "Apple Intelligence is beschikbaar. Test tekstverwerking bij Modellen." : "Apple Intelligence is available. Test text processing in Models.")])
            }
            if provider.kind == .pennelCloud {
                let generation = loginGeneration.current
                let account = try await Task.detached { try engine.cloudAccount() }.value
                if loginGeneration.isCurrent(generation) { cachedAccount = account }
                return try BridgeJSON.encode(["ok": true, "message": nl ? "Pennel Cloud-account gecontroleerd. Spraak en tekst zijn hiermee nog niet getest." : "Pennel Cloud account verified. Speech and text processing have not been tested."])
            }
            return try await background { _ in
                let models = try providerModels(baseUrl: provider.baseUrl, apiKey: Keychain.apiKeys().byProvider[id])
                return try BridgeJSON.encode(["ok": true, "message": nl ? "Verbinding getest · \(models.count) modellen gevonden. Spraak en tekst zijn hiermee nog niet getest." : "Connection tested · \(models.count) models found. Speech and text processing have not been tested."])
            }
        case "providers.models":
            let id = try input.required("provider_id")
            guard let provider = model.settings.providers.first(where: { $0.id == id }) else { throw PennelBridgeError("not_found", "Provider not found.") }
            if let reason = ProviderKinds.unavailableReason(provider.kind) { throw PennelBridgeError("unsupported", reason) }
            if [.appleSpeech, .appleIntelligence, .pennelCloud].contains(provider.kind) {
                let name = defaultModel(kind: provider.kind, task: Self.providerTask(input.text("task")))
                return try BridgeJSON.encode([["id": name, "name": name]])
            }
            return try await background { _ in
                try BridgeJSON.encode(providerModels(baseUrl: provider.baseUrl, apiKey: Keychain.apiKeys().byProvider[id]).map { ["id": $0.id, "name": $0.id] })
            }
        case "meetings.list":
            let query = input.text("query")
            return try await background { engine in try BridgeJSON.encode((query.isEmpty ? engine.meetings() : engine.searchMeetings(query: query)).map(BridgeJSON.meeting)) }
        case "meetings.read":
            let id = try input.required("id")
            let interruption = interruptionMessage(id: id)
            let interruptionCode = interruptionCode(id: id)
            return try await background { try Self.meetingJSON($0, id: id, interruption: interruption, interruptionCode: interruptionCode) }
        case "meetings.start": try await startMeeting(title: input.text("title"), navigate: false); return try snapshot()
        case "meetings.stop":
            if let id = input.values["id"] as? String {
                let runtime = MeetingRuntime.shared
                let current = runtime.activeMeetingId
                guard current == id || (current == nil && (runtime.retainedId == id || runtime.lastFinish?.id == id)) else {
                    throw PennelBridgeError("meeting_changed", "This recording changed. Refresh its current state before stopping it.")
                }
            }
            await stopMeeting()
            return try snapshot()
        case "meetings.retrySave":
            guard try input.required("id") == MeetingRuntime.shared.retainedId else {
                throw PennelBridgeError("meeting_changed", "This recording no longer needs recovery. Refresh its current state.")
            }
            try await MeetingRuntime.shared.retrySave()
            return try snapshot()
        case "meetings.notes":
            let id = try input.required("id")
            guard let notes = input.values["notes"] as? String ?? input.values["text"] as? String else { throw PennelBridgeError("invalid_request", "Missing notes.") }
            if MeetingRuntime.shared.activeMeetingId == id { try MeetingRuntime.shared.saveLiveNotes(notes); return "null" }
            guard let expected = input.values["expected_notes"] as? String else { throw PennelBridgeError("invalid_request", "Reload the current notes before saving.") }
            let output = try await background { engine in
                try engine.mobileSetMeetingNotes(id: id, text: notes, expected: expected)
                return "null"
            }
            CloudSyncRunner.shared.poke()
            return output
        case "meetings.rename":
            let id = try input.required("id")
            let title = try input.required("title")
            if MeetingRuntime.shared.activeMeetingId == id { MeetingRuntime.shared.rename(title); return try BridgeJSON.encode(id) }
            let renamed = try await Task.detached { try engine.renameMeeting(id: id, title: title) }.value
            CloudSyncRunner.shared.poke()
            return try BridgeJSON.encode(renamed)
        case "meetings.delete":
            let id = try input.required("id")
            guard !MeetingRuntime.shared.unfinalisedMeetingIds.contains(id) else { throw PennelBridgeError("meeting_busy", "Finish this recording first.") }
            let result = try await background { try $0.deleteMeeting(id: id); return "null" }
            CloudSyncRunner.shared.poke()
            return result
        case "meetings.summarize":
            let id = try input.required("id")
            let template = input.text("template", default: model.settings.defaultTemplate)
            let interruption = interruptionMessage(id: id)
            let interruptionCode = interruptionCode(id: id)
            let result = try await background { engine in
                _ = try engine.summarizeMeeting(id: id, templateId: template, keys: Keychain.apiKeys())
                return try Self.meetingJSON(engine, id: id, interruption: interruption, interruptionCode: interruptionCode)
            }
            CloudSyncRunner.shared.poke()
            return result
        case "meetings.templates": return try await background { try BridgeJSON.encode($0.meetingTemplates().map(BridgeJSON.template)) }
        case "meetings.ask":
            let question = try input.required("question")
            let turns = (input.values["history"] as? [[String: String]] ?? []).compactMap { value -> ChatTurn? in
                guard let question = value["question"], let answer = value["answer"] else { return nil }
                return ChatTurn(question: question, answer: answer)
            }
            let id = input.values["meeting_id"] as? String
            return try await background { try BridgeJSON.encode(BridgeJSON.answer($0.askMeetings(question: question, history: turns, meetingId: id, keys: Keychain.apiKeys()))) }
        case "meetings.search":
            let query = input.text("query")
            return try await background { try BridgeJSON.encode($0.searchPassages(query: query).map(BridgeJSON.passage)) }
        case "meetings.export":
            let id = try input.required("id")
            let include = input.flag("include_transcript", default: input.flag("transcript"))
            let language: InterfaceLanguage = uiLanguage() == "nl" ? .dutch : .english
            let value = try await Task.detached { try engine.exportMeetingLocalized(id: id, includeTranscript: include, language: language) }.value
            if input.flag("share") { MeetingShare.present(value) }
            if input.flag("copy") { MeetingShare.copy(value, formatted: input.flag("formatted")) }
            return try BridgeJSON.encode(["title": value.title, "markdown": value.markdown, "html": value.html])
        case "meetings.play":
            let id = try input.required("id")
            guard engine.meetingInfo(id: id) != nil else { throw PennelBridgeError("not_found", "Meeting not found.") }
            guard !MeetingRuntime.shared.unfinalisedMeetingIds.contains(id) else { throw PennelBridgeError("meeting_busy", "Finish this recording first.") }
            stopPlayback()
            let speaker: Speaker
            switch input.text("speaker", default: "others") {
            case "me": speaker = .me
            case "others": speaker = .others
            default: throw PennelBridgeError("invalid_request", "Unknown audio track.")
            }
            guard let url = engine.meetingArchiveURL(id: id, speaker: speaker) else {
                throw PennelBridgeError("audio_unavailable", "This meeting has no saved audio for that track.")
            }
            let audio = SharedAudioSession.shared.client(.playback)
            try audio.activate()
            do {
                let next = try AVAudioPlayer(contentsOf: url)
                next.currentTime = max(0, input.number("start_ms") / 1_000)
                guard next.play() else { throw PennelBridgeError("audio_unavailable", "This audio could not play.") }
                player = next
                playbackSession = audio
            } catch { audio.deactivate(); throw error }
            return "null"
        case "meetings.stopPlayback": stopPlayback(); return "null"
        case "meetings.retryTranscription":
            let id = try input.required("id")
            let result = try await background { try BridgeJSON.encode($0.retranscribeFailed(id: id, keys: Keychain.apiKeys())) }
            CloudSyncRunner.shared.poke()
            return result
        case "meetings.conflictDelete":
            let id = try input.required("id")
            let name = try input.required("name")
            let result = try await background { engine in
                guard engine.meetingInfo(id: id) != nil else { throw PennelBridgeError("not_found", "Meeting not found.") }
                let directory = URL(fileURLWithPath: engine.meetingDir(id: id))
                guard Self.conflictURLs(directory).contains(where: { $0.lastPathComponent == name }) else { throw PennelBridgeError("invalid_conflict", "Conflict file not found.") }
                try FileManager.default.removeItem(at: directory.appendingPathComponent(name))
                return "null"
            }
            CloudSyncRunner.shared.poke()
            return result
        default:
            if method.hasPrefix("cloud.") { return try await cloud(method, input) }
            if method.hasPrefix("recognition.") { return try await recognition(method, input) }
            throw PennelBridgeError("unsupported_operation", "This operation is not available on this device.")
        }
    }

    private func background(_ work: @escaping @Sendable (PennelEngine) throws -> String) async throws -> String {
        let engine = model.engine
        return try await Task.detached(priority: .userInitiated) { try work(engine) }.value
    }

    private func inferenceFingerprint(_ capability: InferenceCapability) throws -> String {
        let settings = try BridgeJSON.object(model.engine.mobileSettingsJson())
        let field = capability == .decision ? "decision" : "reranking"
        let selected = settings[field] as? [String: Any]
        let id = selected?["provider_id"] as? String ?? ""
        let account = (settings["providers"] as? [[String: Any]])?.first { $0["id"] as? String == id }
        return try BridgeJSON.encode(["configuration": selected ?? [:], "provider": account ?? [:], "key_revision": inferenceKeyRevisions[id] ?? 0])
    }

    private func performInference(id: String, capability: InferenceCapability, work: @escaping @Sendable (PennelEngine, InferenceCancellation) throws -> String) async throws -> String {
        let ticket = try inferenceRequests.begin(id: id, fingerprint: inferenceFingerprint(capability))
        do {
            let cancellation = ticket.cancellation
            let result = try await background { engine in try work(engine, cancellation) }
            guard inferenceRequests.finish(ticket, currentFingerprint: try inferenceFingerprint(capability)) else { throw PennelError.Cancelled }
            return result
        } catch {
            _ = inferenceRequests.finish(ticket, currentFingerprint: "")
            throw error
        }
    }

    private func settingsOperation(_ work: @escaping @MainActor () async throws -> String) async throws -> String {
        let previous = settingsTail
        let current = Task {
            if let previous { await previous.value }
            return try await work()
        }
        settingsTail = Task { _ = try? await current.value }
        return try await current.value
    }

    private func inferenceSettingsOperation(_ work: @escaping @MainActor () async throws -> String) async throws -> String {
        try await settingsOperation { try await self.inferenceRequests.changingConfiguration(work) }
    }

    private func stopPlayback() {
        player?.stop()
        player = nil
        playbackSession?.deactivate()
        playbackSession = nil
    }

    private func copy(_ value: String, html: String? = nil, secret: Bool = false) {
        var options: [UIPasteboard.OptionsKey: Any] = [.localOnly: true]
        if secret { options[.expirationDate] = Date().addingTimeInterval(120) }
        var item = ["public.utf8-plain-text": value]
        if let html { item["public.html"] = html }
        UIPasteboard.general.setItems([item], options: options)
    }

    private static func providerTask(_ value: String) -> ProviderTask {
        switch value { case "llm", "text": .llm; case "meeting_stt", "meeting": .meetingStt; default: .dictationStt }
    }

    nonisolated static func conflictURLs(_ directory: URL) -> [URL] {
        ((try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isRegularFileKey, .isSymbolicLinkKey])) ?? []).filter {
            let values = try? $0.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
            return values?.isRegularFile == true && values?.isSymbolicLink != true
                && ($0.lastPathComponent as NSString).deletingPathExtension.contains(" (conflict ")
        }.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    private nonisolated static func meetingJSON(_ engine: PennelEngine, id: String, interruption: String? = nil, interruptionCode: String? = nil) throws -> String {
        guard let info = engine.meetingInfo(id: id) else { throw PennelBridgeError("not_found", "Meeting not found.") }
        let directory = URL(fileURLWithPath: engine.meetingDir(id: id))
        let speakers = engine.meetingSpeakers(id: id).map { ["voice": $0.voice, "name": $0.name] as [String: Any] }
        let voices = engine.mobileMeetingVoices(id: id).map { ["voice": $0.voice, "name": BridgeJSON.optional($0.name), "remembered": $0.remembered, "can_remember": $0.canRemember] as [String: Any] }
        return try BridgeJSON.encode([
            "info": BridgeJSON.meeting(info), "notes": engine.meetingNotes(id: id),
            "summary": BridgeJSON.optional(engine.meetingMarkdown(id: id)),
            "markdown": BridgeJSON.optional(engine.meetingMarkdown(id: id)),
            "transcript": engine.meetingTranscript(id: id).map(BridgeJSON.segment),
            "speakers": speakers, "voices": voices, "attendees": engine.meetingAttendees(id: id),
            "audio_tracks": engine.meetingAudioTracks(id: id).map { track in
                ["speaker": track.speaker == .me ? "me" : "others", "available": track.available] as [String: Any]
            },
            "summary_outdated": engine.mobileMeetingSummaryOutdated(id: id),
            "interruption": BridgeJSON.optional(interruption),
            "interruption_code": BridgeJSON.optional(interruptionCode),
            "conflicts": conflictURLs(directory).map { ["name": $0.lastPathComponent, "text": (try? String(contentsOf: $0, encoding: .utf8)) ?? ""] },
        ])
    }

    private func interruptionMessage(id: String) -> String? {
        guard let value = MeetingRuntime.shared.interruption(for: id) else { return nil }
        if let reason = value.stoppedEarly { return reason }
        guard !value.gaps.isEmpty else { return nil }
        return "This recording contains interruptions. Timestamps preserve the gaps."
    }

    private func interruptionCode(id: String) -> String? {
        guard let value = MeetingRuntime.shared.interruption(for: id) else { return nil }
        if let code = value.stoppedEarlyCode ?? MeetingRuntime.Message.code(for: value.stoppedEarly) { return code }
        return value.gaps.isEmpty ? nil : "recording_gaps"
    }
}

extension PennelNativeRuntime {
    private func cloud(_ method: String, _ input: BridgePayload) async throws -> String {
        let engine = model.engine
        let identity = loginGeneration.current
        switch method {
        case "cloud.status":
            let generation = loginGeneration.current
            if engine.cloudSignedIn() {
                let account = try await Task.detached { try engine.cloudAccount() }.value
                if loginGeneration.isCurrent(generation) { cachedAccount = account }
            } else { cachedAccount = nil }
            return try BridgeJSON.encode(cloudState())
        case "cloud.setup":
            return try await background { try BridgeJSON.encode(NativeCloudViews.setup($0.cloudSyncSetup())) }
        case "cloud.prepareSync":
            let code = try await Task.detached { try engine.cloudSyncPrepare() }.value
            guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
            let display = try await recoveryDisplay(code, identity: identity)
            model.reloadSettings()
            return try BridgeJSON.encode(display)
        case "cloud.activateSync", "cloud.resumeSync":
            guard UserDefaults.standard.object(forKey: CloudAccountModel.confirmedKey) as? Bool != false else {
                throw PennelBridgeError("recovery_unconfirmed", "Save your recovery code before starting sync.")
            }
            try await Task.detached {
                if method == "cloud.resumeSync" { try engine.cloudSyncResume() }
                else { try engine.cloudSyncActivate() }
            }.value
            guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
            model.reloadSettings()
            CloudSyncRunner.shared.poke()
            return "null"
        case "cloud.pendingRecovery":
            guard let pending = pendingRecovery, pending.identity == identity, engine.cloudSignedIn() else {
                pendingRecovery = nil
                return try BridgeJSON.encode(NativeRecoveryDisplay.empty(epoch: accountEpoch()))
            }
            let context = try await validatedRecoveryContext(pending)
            guard loginGeneration.isCurrent(identity), pendingRecovery?.token == pending.token else { throw PennelError.Cancelled }
            guard context == pending.context else {
                pendingRecovery = nil
                return try BridgeJSON.encode(NativeRecoveryDisplay.empty(epoch: accountEpoch()))
            }
            return try BridgeJSON.encode(pending.payload(epoch: accountEpoch()))
        case "cloud.linkView": return try BridgeJSON.encode(NativeCloudViews.link(engine.cloudLinkView()))
        case "cloud.linkStart", "cloud.linkJoin", "cloud.linkPoll", "cloud.linkApprove", "cloud.linkCancel":
            let role = try NativeCloudViews.role(input.text("role", default: "new_device"))
            let name = input.text("device_name", default: UserDefaults.standard.string(forKey: "pennel.mobile.deviceName") ?? "iPhone")
            let code = method == "cloud.linkJoin" ? try input.required("code") : ""
            let value = try await Task.detached {
                switch method {
                case "cloud.linkStart": return try engine.cloudLinkStart(role: role, deviceName: name, platform: "ios")
                case "cloud.linkJoin": return try engine.cloudLinkJoin(code: code, role: role, deviceName: name, platform: "ios")
                case "cloud.linkApprove": return try engine.cloudLinkApprove()
                case "cloud.linkCancel": return try engine.cloudLinkCancel()
                default: return try engine.cloudLinkPoll()
                }
            }.value
            guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
            if value.phase == .complete {
                pendingRecovery = nil
                model.reloadSettings()
                let account = try? await Task.detached { try engine.cloudAccount() }.value
                guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
                cachedAccount = account
                UserDefaults.standard.set(true, forKey: CloudAccountModel.confirmedKey)
                CloudSyncRunner.shared.poke()
            }
            return try BridgeJSON.encode(NativeCloudViews.link(value))
        case "cloud.scanConnectionCode":
            let code = try await NativeConnectionScanner.scan(language: uiLanguage())
            return try BridgeJSON.encode(["code": code])
        case "cloud.loginStart":
            let generation = loginGeneration.begin()
            pendingLogin = nil
            loginExpiry = nil
            cachedAccount = nil
            pendingRecovery = nil
            let name = input.text("device_name", default: UserDefaults.standard.string(forKey: "pennel.mobile.deviceName") ?? "iPhone")
            let locale = uiLanguage()
            engine.cloudLoginCancel()
            let start = try await loginStarts.run(generation: generation, owner: loginGeneration) {
                try await Task.detached { try engine.cloudLoginStartWithLocale(deviceName: name, platform: "ios", locale: locale) }.value
            }
            guard loginGeneration.isCurrent(generation) else { throw PennelError.Cancelled }
            pendingLogin = start
            loginExpiry = Date().addingTimeInterval(15 * 60)
            UserDefaults.standard.set(name, forKey: "pennel.mobile.deviceName")
            if let url = URL(string: start.verificationUri), url.scheme == "https" { await UIApplication.shared.open(url) }
            guard loginGeneration.isCurrent(generation) else { throw PennelError.Cancelled }
            return try BridgeJSON.encode(loginJSON(start))
        case "cloud.loginPoll":
            guard let pendingLogin else { return try BridgeJSON.encode(["status": engine.cloudSignedIn() ? "done" : "expired"]) }
            let generation = loginGeneration.current
            guard loginExpiry.map({ $0 > Date() }) == true else {
                loginGeneration.cancel()
                engine.cloudLoginCancel()
                self.pendingLogin = nil
                loginExpiry = nil
                return try BridgeJSON.encode(["status": "expired"])
            }
            let result = try await Task.detached { try engine.cloudLoginPoll(deviceCode: pendingLogin.deviceCode) }.value
            guard loginGeneration.isCurrent(generation) else { throw PennelError.Cancelled }
            let status: String = switch result { case .pending: "pending"; case .slowDown: "slow_down"; case .done: "done"; case .expired: "expired" }
            if status == "done" || status == "expired" { self.pendingLogin = nil }
            if status == "expired" { loginGeneration.cancel(); engine.cloudLoginCancel(); loginExpiry = nil }
            if status == "done" {
                let account = try? await Task.detached { try engine.cloudAccount() }.value
                guard loginGeneration.isCurrent(generation) else { throw PennelError.Cancelled }
                cachedAccount = account
                model.reloadSettings()
                CloudSyncRunner.shared.poke()
            }
            return try BridgeJSON.encode(["status": status])
        case "cloud.loginCancel":
            loginGeneration.cancel()
            engine.cloudLoginCancel()
            pendingLogin = nil
            loginExpiry = nil
            return "null"
        case "cloud.logout":
            loginGeneration.cancel()
            engine.cloudLoginCancel()
            let value = try await background { try $0.cloudLogout(); return "null" }
            cachedAccount = nil
            pendingLogin = nil
            pendingRecovery = nil
            UserDefaults.standard.removeObject(forKey: CloudAccountModel.confirmedKey)
            UserDefaults.standard.removeObject(forKey: Self.automaticConsent)
            model.reloadSettings()
            return value
        case "cloud.sync":
            CloudSyncRunner.shared.poke()
            await CloudSyncRunner.shared.idle()
            return try BridgeJSON.encode(BridgeJSON.report(engine.cloudSyncLast()))
        case "cloud.setSync":
            guard let enable = input.values["enabled"] as? Bool else { throw PennelBridgeError("invalid_request", "Missing sync preference.") }
            let code: String? = try await Task.detached {
                if enable { return try engine.cloudSyncEnable() }
                try engine.cloudSyncDisable()
                return nil
            }.value
            guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
            let display = try await recoveryDisplay(code, identity: identity)
            model.reloadSettings()
            CloudSyncRunner.shared.poke()
            return try BridgeJSON.encode(display)
        case "cloud.devices": return try await background { try BridgeJSON.encode($0.cloudDevices().map(BridgeJSON.device)) }
        case "cloud.renameDevice":
            let id = try input.required("id")
            let name = try input.required("name")
            return try await background { try $0.cloudRenameDevice(id: id, name: name); return "null" }
        case "cloud.revoke":
            let id = try input.required("id")
            return try await background { try $0.cloudRevokeDevice(id: id); return "null" }
        case "cloud.recoveryCreate", "cloud.recoveryReset":
            let reset = method == "cloud.recoveryReset"
            let code = try await Task.detached {
                if reset { return try engine.cloudSyncReset() }
                let setup = try engine.cloudSyncSetup()
                if setup.phase == .newLibrary { return try engine.cloudSyncPrepare() ?? "" }
                return try engine.cloudSyncNewRecoveryCode()
            }.value
            guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
            guard !code.isEmpty else { throw PennelBridgeError("vault_locked", "Unlock your encrypted library first.") }
            let display = try await recoveryDisplay(code, identity: identity)
            model.reloadSettings()
            CloudSyncRunner.shared.poke()
            return try BridgeJSON.encode(display)
        case "cloud.recoveryUnlock":
            let code = try input.required("code")
            let result = try await background { try $0.cloudSyncUnlock(code: code); return "null" }
            guard loginGeneration.isCurrent(identity) else { throw PennelError.Cancelled }
            pendingRecovery = nil
            UserDefaults.standard.set(true, forKey: CloudAccountModel.confirmedKey)
            model.reloadSettings()
            CloudSyncRunner.shared.poke()
            return result
        case "cloud.recoveryConfirm":
            let token = try input.required("confirmation_token")
            guard let pending = pendingRecovery, pending.matches(token: token, identity: identity), engine.cloudSignedIn() else { throw PennelBridgeError("recovery_unconfirmed", "This recovery display changed. Open the current code before confirming.") }
            do { try await Task.detached { try engine.cloudRecoveryConfirm(code: pending.code, context: pending.context) }.value }
            catch {
                discardStaleRecovery(error, pending: pending)
                throw error
            }
            guard loginGeneration.isCurrent(identity), pendingRecovery?.token == token else { throw PennelError.Cancelled }
            UserDefaults.standard.set(true, forKey: CloudAccountModel.confirmedKey)
            pendingRecovery = nil
            model.reloadSettings()
            CloudSyncRunner.shared.poke()
            return "null"
        case "cloud.recoveryCopy":
            let code = try await recoveryCode(input, identity: identity)
            copy(code, secret: true)
            return "null"
        case "cloud.recoveryPrint":
            let code = try await recoveryCode(input, identity: identity)
            let printer = UIPrintInteractionController.shared
            let info = UIPrintInfo.printInfo()
            info.jobName = "Pennel Cloud recovery code"
            printer.printInfo = info
            printer.printFormatter = UISimpleTextPrintFormatter(text: "Pennel Cloud recovery code\n\n\(code)")
            printer.present(animated: true)
            return "null"
        case "cloud.confirmDeletes":
            let result = try await background { try $0.cloudSyncConfirmDeletes(); return "null" }
            CloudSyncRunner.shared.poke()
            return result
        case "cloud.deleteAccount":
            loginGeneration.cancel()
            engine.cloudLoginCancel()
            let result = try await background { try $0.cloudDeleteAccount(); return "null" }
            cachedAccount = nil
            pendingLogin = nil
            pendingRecovery = nil
            model.reloadSettings()
            UserDefaults.standard.removeObject(forKey: CloudAccountModel.confirmedKey)
            UserDefaults.standard.removeObject(forKey: Self.automaticConsent)
            return result
        default: throw PennelBridgeError("unsupported_operation", "This Cloud operation is unavailable.")
        }
    }

    private func recognition(_ method: String, _ input: BridgePayload) async throws -> String {
        let engine = model.engine
        switch method {
        case "recognition.start":
            guard input.flag("consent") else { throw PennelBridgeError("consent_required", "Confirm that this meeting's audio may be uploaded for Cloud speaker recognition.") }
            let id = try input.required("id")
            _ = try await settingsOperation {
                let result = try await self.background { try $0.patchMobileSettings(json: "{\"speaker_recognition\":\"cloud\"}") }
                self.model.reloadSettings()
                return result
            }
            let job = try await startRecognition(id: id)
            return try BridgeJSON.encode(BridgeJSON.recognition(job.snapshot()))
        case "recognition.list": return try BridgeJSON.encode(engine.speakerRecognitionJobs().map(BridgeJSON.recognition))
        case "recognition.get", "recognition.cancel", "recognition.resume":
            let id = try input.required("job_id")
            guard let job = engine.speakerRecognitionJob(jobId: id) else { throw PennelBridgeError("not_found", "Processing job not found.") }
            if method == "recognition.cancel" { try job.cancel() }
            if method == "recognition.resume" {
                let meetingId = try engine.speakerRecognitionMeetingId(jobId: id)
                let reader = try archiveReader(id: meetingId)
                let transport = recognitionTransport
                let confirmed = input.flag("confirm_unknown", default: input.flag("confirm_uncertain_retry"))
                let generation = finishBudgetGeneration
                let budget = UIApplication.shared.applicationState == .active ? SystemFinishScheduler().begin() : nil
                do {
                    let resumed = try await Task.detached { try engine.resumeSpeakerRecognition(jobId: id, reader: reader, transport: transport, confirmUncertainRetry: confirmed) }.value
                    recognitionJobs[id] = resumed
                    if generation != finishBudgetGeneration { try? resumed.interrupt() }
                    watchRecognition(resumed, budget: budget)
                } catch { budget?.end(success: false); throw error }
            }
            return try BridgeJSON.encode(BridgeJSON.recognition((recognitionJobs[id] ?? job).snapshot()))
        case "recognition.renameSpeaker":
            let id = try input.required("id")
            let voice = input.number("voice")
            guard voice >= 1, voice <= Double(UInt32.max), voice.rounded(.down) == voice else { throw PennelBridgeError("invalid_request", "Invalid speaker.") }
            let number = UInt32(voice)
            let name = input.text("name")
            let remember = input.flag("remember")
            let result = try await background { engine in
                if remember { try engine.rememberSpeaker(id: id, voice: number, name: name) }
                else { try engine.renameSpeaker(id: id, voice: number, name: name) }
                return "null"
            }
            CloudSyncRunner.shared.poke()
            return result
        default: throw PennelBridgeError("unsupported_operation", "This recognition operation is unavailable.")
        }
    }

    private func archiveReader(id: String) throws -> NativeRecognitionReader {
        guard !MeetingRuntime.shared.unfinalisedMeetingIds.contains(id), model.engine.meetingInfo(id: id) != nil else {
            throw PennelBridgeError("meeting_busy", "Finish this recording before recognizing its speakers.")
        }
        guard let archive = model.engine.meetingArchiveURL(id: id, speaker: .others) else {
            throw PennelBridgeError("audio_unavailable", "This meeting has no saved audio for speaker recognition.")
        }
        return NativeRecognitionReader(archive: archive)
    }

    private func startRecognition(id: String, ownsBudget: Bool = true) async throws -> SpeakerRecognitionJob {
        let engine = model.engine
        let reader = try archiveReader(id: id)
        let transport = recognitionTransport
        let generation = finishBudgetGeneration
        let budget = ownsBudget && UIApplication.shared.applicationState == .active ? SystemFinishScheduler().begin() : nil
        do {
            let job = try await Task.detached { try engine.startSpeakerRecognition(id: id, reader: reader, transport: transport) }.value
            recognitionJobs[job.id()] = job
            if generation != finishBudgetGeneration { try? job.interrupt() }
            if ownsBudget { watchRecognition(job, budget: budget) }
            return job
        } catch {
            budget?.end(success: false)
            throw error
        }
    }

    private func watchRecognition(_ job: SpeakerRecognitionJob, budget: FinishTask?) {
        let id = job.id()
        recognitionWatches[id]?.cancel()
        recognitionWatches[id] = Task { [weak self] in
            while !Task.isCancelled {
                let state = job.snapshot()
                guard [.preparing, .uploading, .recognizing, .applying].contains(state.stage) else {
                    budget?.end(success: state.stage == .completed)
                    if state.stage == .completed { CloudSyncRunner.shared.poke() }
                    self?.recognitionJobs[id] = nil
                    self?.recognitionWatches[id] = nil
                    self?.publish()
                    return
                }
                budget?.update(pending: max(1, state.totalChunks - min(state.confirmedChunks, state.totalChunks)))
                try? await Task.sleep(for: .milliseconds(250))
            }
            budget?.end(success: false)
        }
    }

    private func postprocess(_ finish: MeetingFinish) async {
        guard finish.error == nil, finish.stoppedEarly == nil else { return }
        let settings = model.engine.settings()
        if Self.automaticRecognitionAllowed(settings, consent: UserDefaults.standard.bool(forKey: Self.automaticConsent)) {
            do {
                let job = try await startRecognition(id: finish.id, ownsBudget: false)
                defer { recognitionJobs[job.id()] = nil }
                while [.preparing, .uploading, .recognizing, .applying].contains(job.snapshot().stage) {
                    try? await Task.sleep(for: .milliseconds(250))
                }
                guard job.snapshot().stage == .completed else { publish(); return }
                CloudSyncRunner.shared.poke()
            } catch { publish(); return }
        }
        guard MeetingRuntime.languageModelReady(model.engine.settings()) else { publish(); return }
        let engine = model.engine
        let template = engine.settings().defaultTemplate
        _ = try? await Task.detached { try engine.summarizeMeeting(id: finish.id, templateId: template, keys: Keychain.apiKeys()) }.value
        CloudSyncRunner.shared.poke()
        publish()
    }

    private func updateSettings(_ input: BridgePayload) async throws -> String {
        guard var requested = input.values["settings"] as? [String: Any] ?? input.values["patch"] as? [String: Any] else {
            throw PennelBridgeError("invalid_request", "Missing settings object.")
        }
        requested.removeValue(forKey: "cloud_speaker_automatic_consent")
        let typingPatch = try NativeKeyboardSettingsPatch(removingFrom: &requested)
        let current = try BridgeJSON.object(model.engine.mobileSettingsJson())
        let keyboard = KeyboardLayout.groupDefaults
        var nextLayout: KeyboardLayout?
        if let layout = requested.removeValue(forKey: "keyboard_layout") {
            guard let text = layout as? String, let value = KeyboardLayout(rawValue: text) else { throw PennelBridgeError("invalid_request", "Unknown keyboard layout.") }
            nextLayout = value
        }
        var nextSuggestions: Bool?
        if let suggestions = requested.removeValue(forKey: "keyboard_suggestions") {
            guard let value = suggestions as? Bool else { throw PennelBridgeError("invalid_request", "Invalid keyboard suggestion preference.") }
            nextSuggestions = value
        }
        var nextFeedback: Bool?
        if let feedback = requested.removeValue(forKey: "haptic_feedback") {
            guard let value = feedback as? Bool else { throw PennelBridgeError("invalid_request", "Invalid feedback preference.") }
            nextFeedback = value
        }
        var nextTypingFeedback: Bool?
        if let feedback = requested.removeValue(forKey: "keyboard_haptics") {
            guard let value = feedback as? Bool else { throw PennelBridgeError("invalid_request", "Invalid typing feedback preference.") }
            nextTypingFeedback = value
        }
        var nextDuration: SessionClock.Duration?
        if let duration = requested.removeValue(forKey: "session_duration_seconds") {
            guard let seconds = duration as? Double, let value = SessionClock.Duration(rawValue: seconds) else { throw PennelBridgeError("invalid_request", "Unknown session duration.") }
            nextDuration = value
        }
        var changed: [String: Any] = [:]
        for (key, value) in requested {
            let before = try BridgeJSON.encode(current[key] ?? NSNull())
            if try BridgeJSON.encode(value) != before { changed[key] = value }
        }
        let enabled = (changed["diarization"] as? Bool ?? model.settings.diarization)
        let mode = changed["speaker_recognition"] as? String ?? (model.settings.speakerRecognition == .cloud ? "cloud" : "local")
        let hadConsent = UserDefaults.standard.bool(forKey: Self.automaticConsent)
        let requestsAutomatic = changed["diarization"] as? Bool == true || (changed["speaker_recognition"] as? String == "cloud" && enabled)
        if requestsAutomatic && mode == "cloud", !hadConsent {
            guard input.flag("cloud_audio_consent") else { throw PennelBridgeError("consent_required", "Confirm automatic audio uploads before enabling Cloud speaker recognition.") }
        }
        let patch = try BridgeJSON.encode(changed)
        let apply: @MainActor () async throws -> String = {
            _ = try await self.background { try $0.patchMobileSettings(json: patch) }
            if enabled && mode == "cloud", hadConsent || input.flag("cloud_audio_consent") { UserDefaults.standard.set(true, forKey: Self.automaticConsent) }
            else { UserDefaults.standard.removeObject(forKey: Self.automaticConsent) }
            nextLayout?.store(in: keyboard)
            if let nextSuggestions { KeyboardPreferences.setSuggestionsEnabled(nextSuggestions, in: keyboard) }
            if let nextFeedback { KeyboardPreferences.setFeedbackEnabled(nextFeedback, in: keyboard) }
            if let nextTypingFeedback {
                KeyboardPreferences.setTypingFeedbackEnabled(nextTypingFeedback, in: keyboard)
                KeyboardPreferences.setTypingFeedbackEnabled(nextTypingFeedback, in: UserDefaults.standard)
            }
            typingPatch.apply(in: keyboard)
            KeyboardPreferences.mirrorKeyboardFeedback(from: keyboard, to: UserDefaults.standard)
            nextDuration?.store()
            self.model.reloadSettings()
            self.publishKeyboardPreferences()
            return try BridgeJSON.encode(self.settingsProjection())
        }
        if ["decision", "reranking", "providers", "cleanup_verification_enabled", "mode", "language", "dictionary", "style_rules", "llm"].contains(where: { changed[$0] != nil }) {
            return try await inferenceRequests.changingConfiguration(apply)
        }
        return try await apply()
    }

    private func keyboardSettings() -> [String: Any] {
        var value: [String: Any] = ["layout": KeyboardLayout.stored(in: KeyboardLayout.groupDefaults).rawValue,
         "suggestions": KeyboardPreferences.suggestionsEnabled(in: KeyboardLayout.groupDefaults),
         "duration": SessionClock.Duration.stored().rawValue]
        value.merge(NativeKeyboardSettingsPatch.projection(in: KeyboardLayout.groupDefaults)) { _, new in new }
        return value
    }

    private func publishKeyboardPreferences() {
        let language = uiLanguage()
        KeyboardPreferences.publish(dictionary: model.settings.dictionary, interfaceLanguage: language, in: KeyboardLayout.groupDefaults)
        if let previous = lastPublishedLanguage, previous != language {
            WidgetCenter.shared.reloadAllTimelines()
            ControlCenter.shared.reloadAllControls()
            Task.detached {
                for activity in Activity<SessionActivityAttributes>.activities { await activity.update(activity.content) }
                for activity in Activity<RecordingActivityAttributes>.activities { await activity.update(activity.content) }
            }
        }
        lastPublishedLanguage = language
    }

    private func uiLanguage() -> String {
        switch model.settings.uiLanguage {
        case .english: return "en"
        case .dutch: return "nl"
        case .system:
            return Locale.preferredLanguages.compactMap { value -> String? in
                let code = value.split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init)
                return code == "en" || code == "nl" ? code : nil
            }.first ?? "en"
        }
    }

    private func cloudState() -> [String: Any] {
        let report = BridgeJSON.report(CloudSyncRunner.shared.report ?? model.engine.cloudSyncLast())
        let signedIn = model.engine.cloudSignedIn()
        var state: [String: Any] = [
            "signed_in": signedIn, "account": signedIn ? BridgeJSON.optional(cachedAccount.map(BridgeJSON.account)) : NSNull(),
            "account_epoch": accountEpoch(), "recovery_display_token": signedIn ? BridgeJSON.optional(pendingRecovery?.token) : NSNull(),
            "syncing": CloudSyncRunner.shared.isRunning, "sync_enabled": model.settings.cloudSync,
            "status": report["status"] ?? "off", "report": report,
            "recovery_confirmed": UserDefaults.standard.object(forKey: CloudAccountModel.confirmedKey) as? Bool != false,
            "brake": CloudAccountModel.isMassDelete(report["error"] as? String),
            "device_name": UserDefaults.standard.string(forKey: "pennel.mobile.deviceName") ?? "iPhone",
            "notice": report["error"] ?? NSNull(),
        ]
        if let pendingLogin { state["login"] = ["phase": "code", "start": loginJSON(pendingLogin)] }
        else { state["login"] = ["phase": "idle"] }
        if cachedAccount == nil { state["account"] = NSNull() }
        return state
    }

    private func loginJSON(_ value: CloudLoginStart) -> [String: Any] {
        ["user_code": value.userCode, "verification_uri": value.verificationUri,
         "interval_secs": value.intervalSecs, "expires_at_ms": (loginExpiry ?? Date()).timeIntervalSince1970 * 1_000]
    }

    private func capabilities() -> [String: Any] {
        func capability(_ status: String, _ reason: String? = nil) -> [String: Any] { ["status": status, "reason": BridgeJSON.optional(reason)] }
        let nl = uiLanguage() == "nl"
        let desktop = nl ? "Werkt alleen op een computer." : "Available on desktop only."
        let microphone = AVAudioApplication.shared.recordPermission == .granted
        let intelligence = intelligenceAvailability()
        return [
            "microphone": capability(microphone ? "available" : "permission_required", microphone ? nil : (nl ? "Sta microfoontoegang toe." : "Allow microphone access.")),
            "apple_speech": capability("available"),
            "apple_intelligence": capability(intelligence.status, intelligence.reason),
            "cloud_speaker_recognition": capability(model.engine.cloudSignedIn() ? "available" : "permission_required", model.engine.cloudSignedIn() ? nil : (nl ? "Log in bij Pennel Cloud." : "Sign in to Pennel Cloud.")),
            "local_models": capability("unsupported", nl ? "Lokale modellen zijn niet beschikbaar op iPhone." : "Local model downloads are unavailable on iPhone."),
            "local_speaker_recognition": capability("unsupported", nl ? "Gebruik optioneel Pennel Cloud om sprekers te herkennen." : "Optionally use Pennel Cloud to recognize speakers."),
            "cli_agents": capability("unsupported", desktop), "echo_cancellation": capability("unsupported", desktop),
            "system_audio": capability("unsupported", nl ? "Op iPhone neemt Pennel alleen de microfoon op." : "Pennel records microphone audio only on iPhone."),
            "call_detection": capability("unsupported", desktop), "calendar": capability("unsupported", desktop),
            "app_styles": capability("unsupported", nl ? "iOS deelt de app waarin je typt niet met Pennel." : "iOS does not expose the receiving app to Pennel."),
            "keyboard_cold_start": capability("unavailable", nl ? "De toetsenbord-only start is nog niet geverifieerd. Start voorlopig een sessie in Pennel." : "Keyboard-only cold start is not verified. Start a session in Pennel in the meantime."),
        ]
    }

    private func kindCatalog() -> [[String: Any]] {
        ProviderKinds.all.map { value in
            let nl = uiLanguage() == "nl"
            let reason: String? = if isCliAgent(kind: value.kind) { nl ? "Werkt alleen op een computer." : "Available on desktop only." }
                else if value.kind == .local { nl ? "Lokale modellen zijn niet beschikbaar op iPhone." : "Local model downloads are unavailable on iPhone." }
                else { nil }
            let label: String = switch value.kind {
            case .appleSpeech: nl ? "Apple-dicteren" : "Apple dictation"
            case .local: nl ? "Lokaal op deze computer" : "Local on this device"
            case .custom: nl ? "Eigen server" : "Custom server"
            case .ollama: "Ollama"
            case .lmStudio: "LM Studio"
            default: value.label
            }
            let availability: [String: Any]
            if value.kind == .appleIntelligence {
                let model = intelligenceAvailability()
                availability = ["status": model.status, "reason": BridgeJSON.optional(model.reason)]
            } else { availability = ["status": reason == nil ? "available" : "unsupported", "reason": BridgeJSON.optional(reason)] }
            return ["kind": BridgeJSON.kind(value.kind), "label": label,
                    "requires_key": requiresKey(kind: value.kind), "stt": supports(kind: value.kind, task: .dictationStt),
                    "llm": supports(kind: value.kind, task: .llm), "default_base_url": defaultBaseUrl(kind: value.kind),
                    "default_stt_model": defaultModel(kind: value.kind, task: .dictationStt),
                    "default_llm_model": defaultModel(kind: value.kind, task: .llm),
                    "availability": availability]
        }
    }

    private func intelligenceAvailability() -> (status: String, reason: String?) {
        let value = FoundationLanguageModel.availability()
        if value.status == "available" || uiLanguage() != "nl" { return (value.status, value.reason) }
        return (value.status, value.status == "unsupported" ? "Apple Intelligence vereist iOS 26 of nieuwer en een geschikte iPhone." : "Apple Intelligence is niet beschikbaar. Controleer Instellingen en wacht tot het model is gedownload.")
    }

    private func snapshot() throws -> String {
        model.reloadSettings()
        adoptNavigation()
        let runtime = MeetingRuntime.shared
        let session = DictationSession.shared
        var value: [String: Any] = [
            "ready": true, "platform": "ios", "settings": try settingsProjection(), "inference_key_revisions": inferenceKeyRevisions,
            "cleanup_verification": BridgeJSON.cleanupVerification(model.engine.cleanupVerificationAvailability()),
            "capabilities": capabilities(), "kinds": kindCatalog(),
            "keys": Dictionary(uniqueKeysWithValues: model.settings.providers.map { ($0.id, Keychain.hasKey(for: $0.id)) }),
            "dictation": ["phase": dictation.phase.rawValue, "text": dictation.text, "level": dictation.level, "session_id": BridgeJSON.optional(dictation.sessionID), "practice": dictation.isPractice, "result_session_id": BridgeJSON.optional(dictation.resultSessionID), "error": BridgeJSON.optional(dictation.error), "error_code": BridgeJSON.optional(dictation.errorCode), "cleanup_verification": BridgeJSON.optional(dictation.cleanupVerification.map(BridgeJSON.cleanupVerification))] as [String: Any],
            "session": ["phase": session.phase.rawValue, "expires_at_ms": BridgeJSON.optional(session.expiresAt.map { $0.timeIntervalSince1970 * 1_000 }),
                        "full_access_seen": KeyboardFullAccess.seen(in: KeyboardLayout.groupDefaults), "error": BridgeJSON.optional(session.startError), "error_code": BridgeJSON.optional(session.startErrorCode), "cleanup_verification": BridgeJSON.optional(session.cleanupVerificationAvailability.map(BridgeJSON.cleanupVerification))] as [String: Any],
            "meeting": ["phase": phase(runtime.phase), "id": BridgeJSON.optional(runtime.activeMeetingId), "title": runtime.title,
                        "retained_id": BridgeJSON.optional(runtime.retainedId), "last_finished_id": BridgeJSON.optional(runtime.lastFinish?.id),
                        "elapsed_secs": Double(runtime.elapsedMs) / 1_000, "notes": runtime.notes,
                        "transcript": runtime.transcript.map(BridgeJSON.segment), "pending": runtime.pending,
                        "error": BridgeJSON.optional(runtime.warning ?? runtime.lastFinish?.error),
                        "error_code": BridgeJSON.optional(MeetingRuntime.Message.code(for: runtime.warning ?? runtime.lastFinish?.error))] as [String: Any],
            "cloud": cloudState(), "recognition_jobs": model.engine.speakerRecognitionJobs().map(BridgeJSON.recognition),
            "navigation": navigation.map { $0 as Any } ?? NSNull(),
        ]
        let comparable = try BridgeJSON.encode(value)
        if comparable != previousState { revision += 1; previousState = comparable }
        value["revision"] = revision
        return try BridgeJSON.encode(value)
    }

    private func phase(_ phase: MeetingRuntime.Phase) -> String {
        switch phase { case .idle: "idle"; case .recording: "recording"; case .interrupted: "interrupted"; case .finishing: "finishing" }
    }

    private func settingsProjection() throws -> [String: Any] {
        var settings = try BridgeJSON.object(model.engine.mobileSettingsJson())
        let consent = UserDefaults.standard.bool(forKey: Self.automaticConsent)
        settings["diarization"] = Self.automaticRecognitionAllowed(model.settings, consent: consent)
        settings["cloud_speaker_automatic_consent"] = consent
        settings["keyboard_layout"] = KeyboardLayout.stored(in: KeyboardLayout.groupDefaults).rawValue
        settings["keyboard_suggestions"] = KeyboardPreferences.suggestionsEnabled(in: KeyboardLayout.groupDefaults)
        settings["haptic_feedback"] = KeyboardPreferences.feedbackEnabled(in: KeyboardLayout.groupDefaults)
        settings["keyboard_haptics"] = KeyboardPreferences.typingFeedbackEnabled(in: KeyboardLayout.groupDefaults)
        for (key, value) in NativeKeyboardSettingsPatch.projection(in: KeyboardLayout.groupDefaults)
            where key.hasPrefix("keyboard_") && key != "keyboard_learned_count" {
            settings[key] = value
        }
        settings["session_duration_seconds"] = SessionClock.Duration.stored().rawValue
        return settings
    }

    nonisolated static func automaticRecognitionAllowed(_ settings: Settings, consent: Bool) -> Bool {
        settings.diarization && settings.speakerRecognition == .cloud && consent
    }

    private func requestNavigation(_ route: String, meetingId: String? = nil) {
        navigationSequence += 1
        var request: [String: Any] = ["sequence": navigationSequence, "route": route]
        if route == "meeting", let id = meetingId ?? MeetingRuntime.shared.activeMeetingId ?? MeetingRuntime.shared.lastFinish?.id { request["meeting_id"] = id }
        navigation = request
    }

    private func adoptNavigation() {
        let router = AppRouter.shared
        let key = "\(router.dictationRequests)/\(router.liveMeetingRequests)"
        guard key != lastNavigationRequest else { return }
        lastNavigationRequest = key
        let meeting = router.takeLiveMeetingRequest()
        let dictation = router.takeDictationRequest()
        if router.selectedTab == .dictate, dictation { requestNavigation("dictation") }
        else if meeting { requestNavigation("meeting") }
        else if dictation { requestNavigation("dictation") }
    }

    private func publish() {
        guard bootstrapped, !listeners.isEmpty, let value = try? snapshot() else { return }
        for listener in listeners.values { listener(value) }
    }

    private func observeLifecycle() {
        let center = NotificationCenter.default
        for name in [UIApplication.didBecomeActiveNotification, UIScene.didActivateNotification] {
            observations.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    CloudSyncRunner.shared.enterForeground()
                    self?.publish()
                }
            })
        }
        observations.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.inferenceRequests.cancelAll(); CloudSyncRunner.shared.enterBackground(); CloudSyncBackground.schedule() }
        })
        observations.append(center.addObserver(forName: Notification.Name("PennelFinishTaskExpired"), object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.finishBudgetGeneration += 1
                for job in self.recognitionJobs.values { try? job.interrupt() }
                self.publish()
            }
        })
    }

    private func accountEpoch() -> String { "\(loginGeneration.current):\(model.engine.cloudSignedIn() ? 1 : 0)" }

    private func recoveryDisplay(_ code: String?, identity: UInt64) async throws -> [String: Any] {
        guard let code else { return NativeRecoveryDisplay.empty(epoch: accountEpoch()) }
        let engine = model.engine
        let context = try await Task.detached { try engine.cloudRecoveryCodeContext(code: code) }.value
        guard loginGeneration.isCurrent(identity), engine.cloudSignedIn() else { throw PennelError.Cancelled }
        let display = NativeRecoveryDisplay(code: code, context: context, identity: identity)
        pendingRecovery = display
        UserDefaults.standard.set(false, forKey: CloudAccountModel.confirmedKey)
        return display.payload(epoch: accountEpoch())
    }

    private func recoveryCode(_ input: BridgePayload, identity: UInt64) async throws -> String {
        let token = try input.required("confirmation_token")
        guard let pending = pendingRecovery, pending.matches(token: token, identity: identity), model.engine.cloudSignedIn() else {
            throw PennelBridgeError("recovery_unconfirmed", "This recovery display changed. Open the current code again.")
        }
        let context = try await validatedRecoveryContext(pending)
        guard loginGeneration.isCurrent(identity), pendingRecovery?.token == token, context == pending.context else {
            throw PennelBridgeError("recovery_unconfirmed", "This recovery code changed. Open the current code again.")
        }
        return pending.code
    }

    private func validatedRecoveryContext(_ pending: NativeRecoveryDisplay) async throws -> String {
        let engine = model.engine
        do { return try await Task.detached { try engine.cloudRecoveryCodeContext(code: pending.code) }.value }
        catch {
            discardStaleRecovery(error, pending: pending)
            throw error
        }
    }

    private func discardStaleRecovery(_ error: Error, pending: NativeRecoveryDisplay) {
        guard loginGeneration.isCurrent(pending.identity), pendingRecovery?.token == pending.token,
              let error = error as? PennelError else { return }
        switch error {
        case .VaultLocked, .NotSignedIn: pendingRecovery = nil
        case .Cloud(let code, _) where ["recovery_code", "recovery_changed", "generation"].contains(code): pendingRecovery = nil
        default: break
        }
    }
}

@MainActor
final class NativeLoginGeneration {
    private(set) var current: UInt64 = 0
    func begin() -> UInt64 { current += 1; return current }
    func cancel() { current += 1 }
    func isCurrent(_ generation: UInt64) -> Bool { generation == current }
}

@MainActor
final class NativeLoginStartQueue {
    private var previous: Task<Void, Never>?

    func run(generation: UInt64, owner: NativeLoginGeneration,
             request: @escaping @Sendable () async throws -> CloudLoginStart) async throws -> CloudLoginStart {
        let predecessor = previous
        let operation = Task {
            if let predecessor { await predecessor.value }
            guard owner.isCurrent(generation) else { throw PennelError.Cancelled }
            let response = try await request()
            guard owner.isCurrent(generation) else { throw PennelError.Cancelled }
            return response
        }
        previous = Task { _ = try? await operation.value }
        return try await operation.value
    }
}

extension BridgePayload {
    func required(_ key: String, alternative: String) throws -> String {
        if let value = values[key] as? String, !value.isEmpty { return value }
        return try required(alternative)
    }
}
