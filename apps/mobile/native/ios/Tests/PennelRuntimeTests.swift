import AVFoundation
import Foundation
import Testing
import PennelMobileKit
@testable internal import PennelMobile

private final class UploadProgress: RecognitionUploadProgress, @unchecked Sendable {
    func sent(sentBytes: UInt64, totalBytes: UInt64) {}
}

private final class FixtureCapture: AudioCapturing, @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    private var running = false
    var starts: Int { lock.withLock { count } }
    var isRunning: Bool { lock.withLock { running } }
    func start(onSamples: @escaping @Sendable ([Float]) -> Void) throws { lock.withLock { count += 1; running = true } }
    func stop() { lock.withLock { running = false } }
}

private struct FixtureAudioSession: AudioSessionControlling {
    func activate() throws {}
    func deactivate() {}
}

private struct QuietDictationRunner: DictationRunning {
    func run(samples: [Float]) async -> Result<DictationResult, PennelError> { .failure(.NoSpeech) }
}

private final class PreparedDictationRunner: DictationRunning, @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false
    private var invocations = 0
    var isCancelled: Bool { lock.withLock { cancelled } }
    var calls: Int { lock.withLock { invocations } }
    func run(samples: [Float]) async -> Result<DictationResult, PennelError> {
        lock.withLock { invocations += 1 }
        return .failure(.NoSpeech)
    }
    func cancel() { lock.withLock { cancelled = true } }
}

@MainActor
private final class FixtureSessionActivity: SessionActivityControlling {
    func start(_ state: SessionActivityState) {}
    func update(_ state: SessionActivityState) {}
    func end(message: String?) {}
}

@MainActor
private final class PreflightGate {
    private var pending: CheckedContinuation<Bool, Never>?
    private var waiting: [CheckedContinuation<Void, Never>] = []
    private(set) var entered = false
    func request() async -> Bool {
        entered = true
        waiting.forEach { $0.resume() }
        waiting = []
        return await withCheckedContinuation { pending = $0 }
    }
    func waitUntilEntered() async {
        if entered { return }
        await withCheckedContinuation { waiting.append($0) }
    }
    func resolve() { pending?.resume(returning: true); pending = nil }
}

private final class DelayedAvailability: @unchecked Sendable {
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    func response() throws -> String {
        entered.signal()
        guard release.wait(timeout: .now() + 5) == .success else { throw PennelBridgeError("fixture_timeout", "Synthetic availability delay timed out.") }
        return "{\"available\":true}"
    }
}

@Suite("Expo native runtime", .serialized)
struct PennelRuntimeTests {
    @MainActor @Test func keyboardPreparesItsRunnerAtRecordingStartAndCancelsBeforeRecognition() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-cleanup-keyboard-\(UUID().uuidString)")
        let store = HandoffStore(directory: directory)
        let runner = PreparedDictationRunner()
        var factoryCalls = 0
        let session = DictationSession(capture: FixtureCapture(), audioSession: FixtureAudioSession(), runner: { factoryCalls += 1; return runner }, store: store, activity: FixtureSessionActivity(), observeCommands: false, requestPermission: { true })
        #expect(await session.start())
        #expect(factoryCalls == 0)
        let client = HandoffClient(store: store, pending: PendingHandoff())
        _ = try client.send(.start, now: Date())
        await session.receiveCommand()
        #expect(session.phase == .listening)
        #expect(factoryCalls == 1)
        #expect(runner.calls == 0)
        _ = try client.send(.cancel, now: Date())
        await session.receiveCommand()
        #expect(runner.isCancelled)
        #expect(runner.calls == 0)
        #expect(store.read(HandoffResult.self, as: .result) == nil)
        session.stop()
    }

    @MainActor @Test func cleanupVerificationFacadeSeparatesSavedPreferenceFromQualification() async throws {
        let runtime = PennelNativeRuntime.shared
        _ = try runtime.bootstrap()
        let engine = AppModel.shared.engine
        let original = engine.settings()
        defer { try? engine.updateSettings(settings: original); AppModel.shared.reloadSettings() }
        for enabled in [false, true] {
            _ = try await runtime.invoke(method: "settings.update", payload: BridgeJSON.encode(["patch": ["cleanup_verification_enabled": enabled]]))
            let status = try BridgeJSON.object(await runtime.invoke(method: "dictation.verificationAvailability", payload: "{}"))
            #expect(status["enabled"] as? Bool == enabled)
            #expect(status["available"] as? Bool == false)
            #expect(status["reason_code"] is String)
            let snapshot = try BridgeJSON.object(runtime.getSnapshot())
            let projected = try #require(snapshot["cleanup_verification"] as? [String: Any])
            #expect(projected["enabled"] as? Bool == enabled)
            #expect(projected["available"] as? Bool == false)
        }
    }

    @MainActor @Test func cleanupVerificationAdmissionSharesSuspendedNativeMutationScope() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-cleanup-mutation-\(UUID().uuidString)")
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        var settings = engine.settings()
        settings.cleanupVerificationEnabled = true
        settings.mode = .clean
        try engine.updateSettings(settings: settings)
        let requests = NativeInferenceRequests(engine: engine)
        let save = PreflightGate()
        let mutation = Task { @MainActor in
            await requests.changingConfiguration { _ = await save.request() }
        }
        await save.waitUntilEntered()
        #expect(engine.cleanupVerificationAvailability().reasonCode == "mutation_in_progress")
        requests.changingCredentials {
            #expect(engine.cleanupVerificationAvailability().reasonCode == "mutation_in_progress")
        }
        #expect(engine.cleanupVerificationAvailability().reasonCode == "mutation_in_progress")
        save.resolve()
        await mutation.value
        #expect(engine.cleanupVerificationAvailability().reasonCode != "mutation_in_progress")
    }

    @MainActor @Test func recorderKeepsFrozenVerificationPreferenceAcrossSettingsMutation() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-cleanup-recorder-\(UUID().uuidString)")
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let recorder = NativeDictationRecorder(engine: engine, capture: FixtureCapture(), audio: FixtureAudioSession(), requestPermission: { true }, reserveHistory: {}, observeSystem: false)
        try await recorder.start()
        #expect(recorder.cleanupVerification?.enabled == false)
        var settings = engine.settings()
        settings.cleanupVerificationEnabled = true
        try engine.updateSettings(settings: settings)
        #expect(engine.cleanupVerificationAvailability().enabled)
        #expect(recorder.cleanupVerification?.enabled == false)
        recorder.cancel()
        #expect(recorder.cleanupVerification == nil)
        #expect(try engine.history(limit: 10).isEmpty)
    }

    @MainActor @Test func inferenceRequestsRejectDuplicateAndOverCapacityWork() throws {
        let requests = NativeInferenceRequests()
        let first = try requests.begin(id: "first", fingerprint: "saved-config")
        do { _ = try requests.begin(id: "first", fingerprint: "replacement"); Issue.record("Duplicate request accepted.") }
        catch let error as PennelBridgeError { #expect(error.code == "operation_busy") }
        let second = try requests.begin(id: "second", fingerprint: "saved-config")
        do { _ = try requests.begin(id: "third", fingerprint: "saved-config"); Issue.record("Unbounded request accepted.") }
        catch let error as PennelBridgeError { #expect(error.code == "operation_busy") }
        #expect(requests.finish(first, currentFingerprint: "saved-config"))
        #expect(requests.finish(second, currentFingerprint: "saved-config"))
    }

    @MainActor @Test func cancelledAndChangedInferenceRequestsNeverApplyLateResults() throws {
        let requests = NativeInferenceRequests()
        let old = try requests.begin(id: "example", fingerprint: "old-config")
        #expect(requests.cancel(id: "example"))
        #expect(old.cancellation.isCancelled())
        #expect(!requests.finish(old, currentFingerprint: "old-config"))
        let fresh = try requests.begin(id: "example", fingerprint: "new-config")
        #expect(!requests.finish(old, currentFingerprint: "new-config"))
        #expect(requests.finish(fresh, currentFingerprint: "new-config"))
        let changed = try requests.begin(id: "changed", fingerprint: "one-key")
        #expect(!requests.finish(changed, currentFingerprint: "replacement-key"))
        #expect(!requests.cancel(id: "missing"))
    }

    @MainActor @Test func inferenceIdentifiersAndCapabilitiesFailClosed() throws {
        let requests = NativeInferenceRequests()
        for id in ["", "not an opaque id", String(repeating: "a", count: 97)] {
            do { _ = try requests.begin(id: id, fingerprint: "config"); Issue.record("Invalid identifier accepted.") }
            catch let error as PennelBridgeError { #expect(error.code == "invalid_request") }
        }
        #expect(try BridgeJSON.inferenceCapability("decision") == .decision)
        #expect(try BridgeJSON.inferenceCapability("reranking") == .reranking)
        do { _ = try BridgeJSON.inferenceCapability("dictation"); Issue.record("Unknown capability became speech.") }
        catch let error as PennelBridgeError { #expect(error.code == "invalid_request") }
    }

    @MainActor @Test func inferenceIsCancelledBeforeCredentialMutationEvenWhenSavingFails() throws {
        let requests = NativeInferenceRequests()
        let first = try requests.begin(id: "queued", fingerprint: "old-key")
        let second = try requests.begin(id: "running", fingerprint: "old-key")
        let saved = requests.changingCredentials {
            #expect(first.cancellation.isCancelled())
            #expect(second.cancellation.isCancelled())
            return false
        }
        #expect(!saved)
        #expect(!requests.finish(first, currentFingerprint: "old-key"))
        #expect(!requests.finish(second, currentFingerprint: "old-key"))
    }

    @MainActor @Test func inferenceAdmissionStaysClosedAcrossSuspendedAndFailedConfigurationChanges() async throws {
        for shouldFail in [false, true] {
            let requests = NativeInferenceRequests()
            let old = try requests.begin(id: "old-request", fingerprint: "old-config")
            let save = PreflightGate()
            var workInvocations = 0
            let expectBlocked: @MainActor (String) -> Void = { id in
                do {
                    let admitted = try requests.begin(id: id, fingerprint: "old-config")
                    workInvocations += 1
                    _ = requests.finish(admitted, currentFingerprint: "old-config")
                    Issue.record("A provider operation was admitted during configuration mutation.")
                } catch let error as PennelBridgeError { #expect(error.code == "operation_busy") }
                catch { Issue.record("Unexpected inference admission error: \(error)") }
            }
            let mutation = Task { @MainActor in
                try await requests.changingConfiguration {
                    #expect(old.cancellation.isCancelled())
                    _ = await save.request()
                    requests.changingCredentials { expectBlocked("nested-key-change") }
                    expectBlocked("model-reload")
                    if shouldFail { throw PennelBridgeError("fixture_save", "Synthetic save failure.") }
                }
            }
            await save.waitUntilEntered()
            expectBlocked("queued-during-save")
            #expect(workInvocations == 0)
            save.resolve()
            do { try await mutation.value; #expect(!shouldFail) }
            catch let error as PennelBridgeError { #expect(shouldFail && error.code == "fixture_save") }
            #expect(workInvocations == 0)
            #expect(!requests.finish(old, currentFingerprint: "new-config"))
            let fresh = try requests.begin(id: "after-save", fingerprint: "new-config")
            #expect(requests.finish(fresh, currentFingerprint: "new-config"))
        }
    }

    @MainActor @Test func delayedAvailabilityKeepsTheMainActorResponsiveAndRejectsOldIdentity() async throws {
        let requests = NativeInferenceRequests()
        let delayed = DelayedAvailability()
        defer { delayed.release.signal() }
        var fingerprint = "old-account"
        var finished = false
        let start = Date()
        let operation = Task { @MainActor in
            defer { finished = true }
            return try await requests.availability(fingerprint: fingerprint, currentFingerprint: { fingerprint }, work: { try delayed.response() })
        }
        await Task.detached { delayed.entered.wait() }.value
        #expect(Date().timeIntervalSince(start) < 4)
        #expect(!finished)
        fingerprint = "replacement-account"
        delayed.release.signal()
        do { _ = try await operation.value; Issue.record("A delayed old-account capability response was accepted.") }
        catch let error as PennelError { #expect(error == .Cancelled) }
        let current = try await requests.availability(fingerprint: fingerprint, currentFingerprint: { fingerprint }, work: { "{\"available\":true}" })
        #expect(current == "{\"available\":true}")
    }

    @MainActor @Test func availabilityCannotPublishWhileAnIdentityMutationIsSuspended() async throws {
        let requests = NativeInferenceRequests()
        let delayed = DelayedAvailability()
        defer { delayed.release.signal() }
        let availability = Task { @MainActor in
            try await requests.availability(fingerprint: "unchanged", currentFingerprint: { "unchanged" }, work: { try delayed.response() })
        }
        await Task.detached { delayed.entered.wait() }.value
        let gate = PreflightGate()
        let mutation = Task { @MainActor in await requests.changingConfiguration { _ = await gate.request() } }
        await gate.waitUntilEntered()
        delayed.release.signal()
        do { _ = try await availability.value; Issue.record("Availability was published before the identity mutation completed.") }
        catch let error as PennelError { #expect(error == .Cancelled) }
        gate.resolve()
        await mutation.value
    }

    @MainActor @Test func managedCloudInferenceRequiresSignInAndPreservesTaskAssignments() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("pennel-managed-cloud-\(UUID().uuidString)")
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let original = engine.settings()
        #expect(!engine.cloudSignedIn())
        var fixture = original
        let cloud = ProviderAccount(id: "managed-cloud", kind: .pennelCloud, name: "Pennel Cloud", baseUrl: "https://untrusted.invalid")
        fixture.providers.append(cloud)
        var choice = newInferenceConfig(providerId: cloud.id, protocol: .pennelCloud)
        choice.model = "pennel-auto"
        fixture.decision = choice
        fixture.reranking = choice
        try engine.updateSettings(settings: fixture)
        for capability in [InferenceCapability.decision, .reranking] {
            let availability = engine.inferenceAvailability(capability: capability)
            #expect(availability.configured)
            #expect(!availability.available)
            let result = try engine.verifyInference(capability: capability, cancellation: InferenceCancellation(), keys: ApiKeys(byProvider: [:]))
            #expect(!result.verified)
            #expect(result.identity == nil)
        }
        let saved = engine.settings()
        #expect(saved.dictationStt == original.dictationStt)
        #expect(saved.meetingStt == original.meetingStt)
        #expect(saved.llm == original.llm)
        #expect(saved.decision == choice)
        #expect(saved.reranking == choice)
    }

    @MainActor @Test func inferenceFacadeReportsUnconfiguredCapabilitiesWithoutInference() async throws {
        let runtime = PennelNativeRuntime.shared
        _ = try runtime.bootstrap()
        let model = AppModel.shared
        let original = model.settings
        var fixture = original
        fixture.decision = nil
        fixture.reranking = nil
        try model.engine.updateSettings(settings: fixture)
        model.reloadSettings()
        defer { try? model.engine.updateSettings(settings: original); model.reloadSettings() }
        let metadata = try await runtime.invoke(method: "inference.protocols", payload: "{}")
        let protocols = try #require(try JSONSerialization.jsonObject(with: Data(metadata.utf8)) as? [[String: Any]])
        #expect(protocols.count == 8)
        #expect(protocols.contains { $0["protocol"] as? String == "open_router_decisions" && $0["decision"] as? Bool == true })
        #expect(protocols.contains { $0["protocol"] as? String == "tei" && $0["reranking"] as? Bool == true })
        let cloud = try #require(protocols.first { $0["protocol"] as? String == "pennel_cloud" })
        #expect(cloud["managed"] as? Bool == true)
        #expect(cloud["allowed_provider_kinds"] as? [String] == ["pennel_cloud"])
        #expect(cloud["decision"] as? Bool == true)
        #expect(cloud["reranking"] as? Bool == true)
        #expect(cloud["requires_key"] as? Bool == false)
        for capability in ["decision", "reranking"] {
            let availability = try BridgeJSON.object(await runtime.invoke(method: "inference.availability", payload: BridgeJSON.encode(["capability": capability])))
            #expect(availability["configured"] as? Bool == false)
            #expect(availability["available"] as? Bool == false)
            let result = try BridgeJSON.object(await runtime.invoke(method: "inference.verify", payload: BridgeJSON.encode(["capability": capability, "request_id": "unconfigured-\(capability)"])))
            #expect(result["verified"] as? Bool == false)
            #expect(result["identity"] is NSNull)
        }
        do {
            _ = try await runtime.invoke(method: "inference.verify", payload: "{\"capability\":\"speech\",\"request_id\":\"invalid-capability\"}")
            Issue.record("An unknown inference capability was accepted by the native facade.")
        } catch let error as PennelBridgeError { #expect(error.code == "invalid_request") }
    }

    @MainActor @Test func nativeFailuresUseEnglishDiagnosticsAndStableCodes() throws {
        #expect(ErrorText.message(PennelError.NoSpeech) == "No speech recognized. Try again.")
        #expect(ErrorText.failureCode(PennelError.NoSpeech) == "no_speech")
        #expect(ErrorText.failureCode(PennelError.Http(status: 401, message: "Private provider details")) == "provider_auth")
        #expect(ErrorText.failureCode(PennelError.Http(status: 403, message: "Private provider details")) == "provider_auth")
        #expect(ErrorText.failureCode(PennelError.NotSignedIn) == "not_signed_in")
        #expect(ErrorText.code(PennelError.Cloud(code: "private provider code", message: "private provider details")) == "cloud")
        #expect(NativeLanguageModelError.invalidRequest.errorDescription == "Invalid request for the built-in Apple text model.")
        #expect(NativeLanguageModelAvailability.unavailable(.notReady).reason?.contains("Wait until iOS") == true)
        #expect(MeetingRuntime.Message.code(for: "De opname is gestopt omdat iOS Pennel heeft afgesloten.") == "recording_terminated")
        #expect(MeetingRuntime.Message.code(for: MeetingRuntime.Message.terminated) == "recording_terminated")
        #expect(CloudAccountModel.isMassDelete(CloudAccountModel.massDelete))
        #expect(CloudAccountModel.isMassDelete("Veel meetings zijn verdwenen uit je map. Sync heeft niets verwijderd; bevestig in Instellingen → Account als dat de bedoeling was."))
        let legacy = Data("{\"gaps\":[],\"stoppedEarly\":\"De opname is gestopt omdat iOS Pennel heeft afgesloten.\"}".utf8)
        let saved = try JSONDecoder().decode(MeetingInterruptionLog.Record.self, from: legacy)
        #expect(saved.stoppedEarlyCode == nil)
        #expect(saved.stoppedEarly == "De opname is gestopt omdat iOS Pennel heeft afgesloten.")
    }

    @Test func roomAndLegacyArchivesUseTheSharedResolver() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let session = try engine.startMeeting(title: "Room fixture", keys: ApiKeys(byProvider: [:]))
        let id = session.id()
        let tracks = try engine.prepareMeetingRecording(id: id, environment: RecordingEnvironment(
            appVersion: "test", build: "test", platform: "ios", osVersion: "test"
        ), roomRecording: true)
        let track = try #require(tracks.first)
        #expect(track.source == .roomMicrophone)
        #expect(track.speaker == .others)
        #expect(track.relativePath == "audio/room.m4a")
        let root = URL(fileURLWithPath: engine.meetingDir(id: id))
        let room = root.appendingPathComponent(track.relativePath)
        try Data([1, 2, 3]).write(to: room)
        #expect(engine.meetingArchiveURL(id: id, speaker: .others) == room.resolvingSymlinksInPath())
        try session.stop()
        try engine.finishMeetingRecording(id: id, durationMs: 1_234, complete: true)
        #expect(try engine.meetingRecordingMetadata(id: id)?.durationMs == 1_234)

        let legacy = try engine.startMeeting(title: "Legacy fixture", keys: ApiKeys(byProvider: [:]))
        try legacy.stop()
        let legacyID = legacy.id()
        let legacyURL = URL(fileURLWithPath: engine.meetingDir(id: legacyID)).appendingPathComponent("anderen.m4a")
        try Data([1, 2, 3]).write(to: legacyURL)
        #expect(engine.meetingArchiveURL(id: legacyID, speaker: .others) == legacyURL.resolvingSymlinksInPath())
        #expect(try engine.meetingRecordingMetadata(id: legacyID) == nil)
    }

    @MainActor @Test func staleRetryCannotSaveAnotherRecording() async throws {
        _ = try PennelNativeRuntime.shared.bootstrap()
        do {
            _ = try await PennelNativeRuntime.shared.invoke(method: "meetings.retrySave", payload: "{\"id\":\"not-a-retained-recording\"}")
            Issue.record("A stale retry must reject instead of reporting a save.")
        } catch let error as PennelBridgeError { #expect(error.code == "meeting_changed") }
    }

    @Test func stoppedModuleObservationCannotInstallALateListener() throws {
        let observation = ModuleObservation()
        let first = try #require(observation.begin())
        #expect(observation.remove(permanently: false) == nil)
        #expect(!observation.install(UUID(), generation: first.generation))
        let second = try #require(observation.begin())
        let id = UUID()
        #expect(observation.install(id, generation: second.generation))
        let third = try #require(observation.begin())
        #expect(third.previous == id)
        #expect(!observation.install(UUID(), generation: second.generation))
        #expect(observation.remove(permanently: true) == nil)
        #expect(!observation.install(UUID(), generation: third.generation))
        #expect(observation.begin() == nil)
    }

    @MainActor @Test func cancelledPermissionPreflightCannotStartTheMicrophone() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let capture = FixtureCapture()
        let permission = PreflightGate()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: FixtureAudioSession(), requestPermission: { await permission.request() }, reserveHistory: {}, observeSystem: false)
        let first = Task { try await recorder.start() }
        await permission.waitUntilEntered()
        try await recorder.start()
        recorder.cancel()
        permission.resolve()
        do { try await first.value; Issue.record("Cancelled preflight must reject.") }
        catch PennelError.Cancelled {}
        #expect(capture.starts == 0)
        #expect(recorder.phase == .idle)
    }

    @MainActor @Test func cancelledCapacityPreflightCannotStartTheMicrophone() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let capture = FixtureCapture()
        let capacity = PreflightGate()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: FixtureAudioSession(), requestPermission: { true }, reserveHistory: { _ = await capacity.request() }, observeSystem: false)
        let first = Task { try await recorder.start() }
        await capacity.waitUntilEntered()
        recorder.cancel()
        capacity.resolve()
        do { try await first.value; Issue.record("Cancelled capacity preflight must reject.") }
        catch PennelError.Cancelled {}
        #expect(capture.starts == 0)
        #expect(recorder.phase == .idle)
    }

    @MainActor @Test func concurrentStartsCreateOnlyOneCapture() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let capture = FixtureCapture()
        let permission = PreflightGate()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: FixtureAudioSession(), requestPermission: { await permission.request() }, reserveHistory: {}, observeSystem: false)
        let first = Task { try await recorder.start() }
        await permission.waitUntilEntered()
        try await recorder.start()
        permission.resolve()
        try await first.value
        #expect(capture.starts == 1)
        recorder.cancel()
    }

    @MainActor @Test func stoppedKeyboardSessionCannotStartAfterPermissionReturns() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let capture = FixtureCapture()
        let permission = PreflightGate()
        let session = DictationSession(capture: capture, audioSession: FixtureAudioSession(), runner: { QuietDictationRunner() }, store: HandoffStore(directory: directory), activity: FixtureSessionActivity(), observeCommands: false, requestPermission: { await permission.request() })
        let starting = Task { await session.start() }
        await permission.waitUntilEntered()
        session.stop()
        permission.resolve()
        #expect(await starting.value == false)
        #expect(capture.starts == 0)
        #expect(session.phase == .off)
    }

    @MainActor @Test func cancelledPreviewCannotReplaceTheNextDictation() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let capture = FixtureCapture()
        let recorder = NativeDictationRecorder(engine: engine, capture: capture, audio: FixtureAudioSession(), requestPermission: { true }, reserveHistory: {}, observeSystem: false)
        try await recorder.start()
        let old = try #require(recorder.previewObserver)
        recorder.cancel()
        try await recorder.start()
        old.receive(DictationPreviewUpdate(sessionId: "old", revision: 1, text: "stale fixture"))
        #expect(recorder.phase == .listening)
        #expect(recorder.text.isEmpty)
        #expect(capture.starts == 2)
        recorder.cancel()
    }

    @MainActor @Test func replacedLoginGenerationCannotClearTheNewFlow() {
        let generation = NativeLoginGeneration()
        let old = generation.begin()
        generation.cancel()
        let replacement = generation.begin()
        #expect(!generation.isCurrent(old))
        #expect(generation.isCurrent(replacement))
    }

    @MainActor @Test func obsoleteQueuedLoginStartsAreDiscarded() async throws {
        let queue = NativeLoginStartQueue()
        let generation = NativeLoginGeneration()
        let gate = PreflightGate()
        let firstEpoch = generation.begin()
        let first = Task {
            try await queue.run(generation: firstEpoch, owner: generation) {
                _ = await gate.request()
                return CloudLoginStart(userCode: "fixture", verificationUri: "https://example.invalid", deviceCode: "native-only", intervalSecs: 5)
            }
        }
        await gate.waitUntilEntered()
        let obsoleteEpoch = generation.begin()
        let obsolete = Task {
            try await queue.run(generation: obsoleteEpoch, owner: generation) {
                Issue.record("An obsolete queued login must not reach the native request.")
                return CloudLoginStart(userCode: "obsolete", verificationUri: "https://example.invalid", deviceCode: "native-only", intervalSecs: 5)
            }
        }
        generation.cancel()
        let latestEpoch = generation.begin()
        let latest = Task {
            try await queue.run(generation: latestEpoch, owner: generation) {
                CloudLoginStart(userCode: "latest", verificationUri: "https://example.invalid", deviceCode: "native-only", intervalSecs: 5)
            }
        }
        gate.resolve()
        do { _ = try await first.value; Issue.record("Old in-flight start must reject.") } catch PennelError.Cancelled {}
        do { _ = try await obsolete.value; Issue.record("Old queued start must reject.") } catch PennelError.Cancelled {}
        #expect(try await latest.value.userCode == "latest")
    }

    @MainActor @Test func intentsBeforeJavaScriptCoalesceToTheLatestRoute() throws {
        _ = try PennelNativeRuntime.shared.bootstrap()
        let router = AppRouter.shared
        router.requestLiveMeeting()
        router.requestDictation()
        let first = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())["navigation"] as? [String: Any]
        #expect(first?["route"] as? String == "dictation")
        router.requestDictation()
        router.requestLiveMeeting()
        let second = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())["navigation"] as? [String: Any]
        #expect(second?["route"] as? String == "meeting")
    }
    @Test func boundedArchiveReadsSeekAndEndWithoutWholeRecordingBuffers() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let archive = directory.appendingPathComponent("fixture.caf")
        let format = try #require(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false))
        var writer: AVAudioFile? = try AVAudioFile(forWriting: archive, settings: format.settings)
        let audio = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 64_000))
        audio.frameLength = 64_000
        for index in 0..<64_000 { audio.floatChannelData![0][index] = Float(index % 100) / 100 }
        try writer?.write(from: audio)
        try writer?.write(from: audio)
        writer = nil

        let reader = NativeRecognitionReader(archive: archive)
        let info = try reader.info()
        #expect(info.totalFrames == 128_000)
        #expect(info.audioDigest.count == 64)
        let middle = try reader.readFrames(offset: 64_010, maxFrames: 64_000)
        #expect(middle.samples.count == 63_990)
        #expect(abs(middle.samples[0] - 0.1) < 0.0001)
        #expect(middle.eof)
        let end = try reader.readFrames(offset: 128_000, maxFrames: 64_000)
        #expect(end.samples.isEmpty)
        #expect(end.eof)
        #expect(throws: Error.self) { try reader.readFrames(offset: 0, maxFrames: 64_001) }
        #expect(throws: Error.self) { try reader.readFrames(offset: 128_001, maxFrames: 1) }
    }

    @Test func preCancelledUploadCannotCreateANetworkRequest() throws {
        let transport = NativeRecognitionTransport()
        transport.cancel(requestId: "cancelled-before-dispatch")
        let request = RecognitionUploadRequest(requestId: "cancelled-before-dispatch", url: "https://127.0.0.1:1/must-not-run", bearerToken: "fixture", filePath: "/missing-fixture", timeoutSeconds: 1)
        do {
            _ = try transport.upload(request: request, progress: UploadProgress())
            Issue.record("A pre-cancelled upload must fail before touching its file or endpoint.")
        } catch PennelError.Cancelled {}
    }

    @Test func aacMeetingArchiveSupportsBoundedSeekAndExactEOF() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let archive = directory.appendingPathComponent("anderen.m4a")
        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 32_000]
        var writer: AVAudioFile? = try AVAudioFile(forWriting: archive, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        let format = try #require(writer?.processingFormat)
        let audio = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 64_000))
        audio.frameLength = 64_000
        for index in 0..<64_000 { audio.floatChannelData![0][index] = 0.1 }
        try writer?.write(from: audio)
        try writer?.write(from: audio)
        writer = nil

        let reader = NativeRecognitionReader(archive: archive)
        let info = try reader.info()
        try #require(info.totalFrames > 64_000)
        let first = try reader.readFrames(offset: 0, maxFrames: 64_000)
        #expect(first.samples.count == 64_000)
        #expect(!first.eof)
        let tail = try reader.readFrames(offset: info.totalFrames - 137, maxFrames: 64_000)
        #expect(tail.samples.count == 137)
        let finiteSamples = tail.samples.allSatisfy(\.isFinite)
        #expect(finiteSamples)
        #expect(tail.eof)
        let end = try reader.readFrames(offset: info.totalFrames, maxFrames: 64_000)
        #expect(end.samples.isEmpty)
        #expect(end.eof)
    }

    @Test func changedArchiveCannotBeAppliedAgainstAPinnedDecoder() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let archive = directory.appendingPathComponent("fixture.caf")
        let format = try #require(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1_000))
        buffer.frameLength = 1_000
        for index in 0..<1_000 { buffer.floatChannelData![0][index] = 0.1 }
        var writer: AVAudioFile? = try AVAudioFile(forWriting: archive, settings: format.settings)
        try writer?.write(from: buffer)
        writer = nil
        let reader = NativeRecognitionReader(archive: archive)
        _ = try reader.info()
        for index in 0..<1_000 { buffer.floatChannelData![0][index] = 0.2 }
        writer = try AVAudioFile(forWriting: archive, settings: format.settings)
        try writer?.write(from: buffer)
        writer = nil
        #expect(abs(try reader.readFrames(offset: 0, maxFrames: 1_000).samples[0] - 0.1) < 0.0001)
        do { _ = try reader.info(); Issue.record("A changed original archive must invalidate the job.") }
        catch PennelError.Cloud(let code, _) { #expect(code == "speaker_stale") }
    }

    @Test func conflictFilesUseTheSharedNamingConventionAndExcludeSymlinks() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let conflict = directory.appendingPathComponent("notes (conflict laptop 2026-10-27).md")
        try Data("fixture".utf8).write(to: conflict)
        try Data("fixture".utf8).write(to: directory.appendingPathComponent("notes.md"))
        try FileManager.default.createSymbolicLink(at: directory.appendingPathComponent("meeting (conflict link).md"), withDestinationURL: conflict)
        #expect(PennelNativeRuntime.conflictURLs(directory).map(\.lastPathComponent) == [conflict.lastPathComponent])
    }

    @MainActor @Test func reconnectSnapshotIsMonotonicAndContainsNoNativeSecrets() throws {
        let first = try BridgeJSON.object(PennelNativeRuntime.shared.bootstrap())
        let second = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())
        let firstRevision = try #require(first["revision"] as? NSNumber).uint64Value
        let secondRevision = try #require(second["revision"] as? NSNumber).uint64Value
        #expect(firstRevision > 0)
        #expect(secondRevision >= firstRevision)
        #expect(second["platform"] as? String == "ios")
        #expect(second["ready"] as? Bool == true)
        #expect(second["settings"] is [String: Any])
        #expect(second["capabilities"] is [String: Any])
        let encoded = try BridgeJSON.encode(second)
        for field in ["bearer_token", "refresh_token", "vault_key", "device_code", "samples", "embedding"] {
            #expect(!encoded.contains("\"\(field)\""))
        }
    }

    @MainActor @Test func unknownCommandsFailRatherThanClaimSuccess() async throws {
        do {
            _ = try await PennelNativeRuntime.shared.invoke(method: "fixture.invalid", payload: "{}")
            Issue.record("Unknown native operations must fail.")
        } catch let error as PennelBridgeError { #expect(error.code == "unsupported_operation") }
    }

    @MainActor @Test func invalidSettingsDoNotReplaceSavedChoices() async throws {
        let before = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())["settings"] as? [String: Any]
        do {
            _ = try await PennelNativeRuntime.shared.invoke(method: "settings.update", payload: "{\"settings\":{\"mode\":\"invalid-mode\"}}")
            Issue.record("Invalid settings must fail.")
        } catch {}
        let after = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())["settings"] as? [String: Any]
        #expect(before?["mode"] as? String == after?["mode"] as? String)
    }

    @MainActor @Test func importedCloudSettingDoesNotAuthorizeAutomaticUploads() async throws {
        _ = try PennelNativeRuntime.shared.bootstrap()
        let model = AppModel.shared
        let original = model.settings
        let key = "pennel.mobile.cloudSpeakerAutomaticConsent"
        let savedConsent = UserDefaults.standard.object(forKey: key)
        defer {
            try? model.engine.updateSettings(settings: original)
            model.reloadSettings()
            if let savedConsent { UserDefaults.standard.set(savedConsent, forKey: key) }
            else { UserDefaults.standard.removeObject(forKey: key) }
        }
        model.update { $0.diarization = true; $0.speakerRecognition = .cloud }
        UserDefaults.standard.removeObject(forKey: key)
        #expect(!PennelNativeRuntime.automaticRecognitionAllowed(model.settings, consent: false))
        let before = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())["settings"] as? [String: Any]
        #expect(before?["diarization"] as? Bool == false)
        let nextMode = model.settings.mode == .fast ? "clean" : "fast"
        _ = try await PennelNativeRuntime.shared.invoke(method: "settings.update", payload: "{\"settings\":{\"mode\":\"\(nextMode)\"}}")
        let after = try BridgeJSON.object(PennelNativeRuntime.shared.getSnapshot())["settings"] as? [String: Any]
        #expect(after?["mode"] as? String == nextMode)
        #expect(after?["diarization"] as? Bool == false)
        #expect(!UserDefaults.standard.bool(forKey: key))
    }

    @MainActor @Test func failedProviderSavePreservesItsCredential() async throws {
        _ = try PennelNativeRuntime.shared.bootstrap()
        let model = AppModel.shared
        let original = model.settings
        let provider = newAccount(kind: .custom, existing: original.providers)
        var fixture = original
        fixture.providers.append(provider)
        try model.engine.updateSettings(settings: fixture)
        model.reloadSettings()
        try #require(Keychain.set(key: "synthetic-fixture", for: provider.id))
        let settingsURL = AppPaths.dataDirectory.appendingPathComponent("settings.json")
        let saved = try Data(contentsOf: settingsURL)
        try FileManager.default.removeItem(at: settingsURL)
        try FileManager.default.createDirectory(at: settingsURL, withIntermediateDirectories: false)
        defer {
            try? FileManager.default.removeItem(at: settingsURL)
            try? saved.write(to: settingsURL, options: .atomic)
            try? model.engine.updateSettings(settings: original)
            model.reloadSettings()
            Keychain.set(key: nil, for: provider.id)
        }
        let payload = try BridgeJSON.encode(["provider_id": provider.id])
        do {
            _ = try await PennelNativeRuntime.shared.invoke(method: "providers.remove", payload: payload)
            Issue.record("A failed provider save must reject.")
        } catch {}
        #expect(model.engine.settings().providers.contains(where: { $0.id == provider.id }))
        #expect(Keychain.hasKey(for: provider.id))
    }

    @MainActor @Test func staleModelVerificationCannotVerifyAnotherModel() async throws {
        _ = try PennelNativeRuntime.shared.bootstrap()
        let model = AppModel.shared
        let original = model.settings
        var provider = newAccount(kind: .custom, existing: original.providers)
        provider.baseUrl = "https://example.invalid"
        var fixture = original
        fixture.providers.append(provider)
        fixture.dictationStt = ModelChoice(providerId: provider.id, model: "fixture-current-model")
        try model.engine.updateSettings(settings: fixture)
        model.reloadSettings()
        defer { try? model.engine.updateSettings(settings: original); model.reloadSettings() }
        let payload = try BridgeJSON.encode(["provider_id": provider.id, "task": "dictation_stt", "verify_task": true, "expected_model": "fixture-obsolete-model"])
        do {
            _ = try await PennelNativeRuntime.shared.invoke(method: "providers.test", payload: payload)
            Issue.record("A stale verification request must not test a different model.")
        } catch let error as PennelBridgeError { #expect(error.code == "provider_changed") }
    }
}
