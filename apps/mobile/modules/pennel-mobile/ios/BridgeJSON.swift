import Foundation

enum BridgeJSON {
    static func encode(_ value: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .fragmentsAllowed])
        guard let text = String(data: data, encoding: .utf8) else {
            throw PennelBridgeError("encoding", "Pennel could not encode its response.")
        }
        return text
    }

    static func object(_ text: String) throws -> [String: Any] {
        guard let data = text.data(using: .utf8),
              let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw PennelBridgeError("invalid_request", "Expected a JSON object.")
        }
        return value
    }

    static func optional<T>(_ value: T?) -> Any { value.map { $0 as Any } ?? NSNull() }
    static func segment(_ value: Segment) -> [String: Any] {
        ["start_ms": value.startMs, "end_ms": value.endMs, "speaker": value.speaker == .me ? "me" : "others",
         "text": value.text, "failed": value.failed, "voice": optional(value.voice)]
    }
    static func transcriptProjection(_ text: String) throws -> [String: Any] {
        let value = try object(text)
        guard let uid = value["meeting_uid"] as? String, !uid.isEmpty,
              value["revision"] is NSNumber, value["metadata_status"] is String,
              value["transcript"] is [[String: Any]] else {
            throw PennelBridgeError("invalid_response", "The transcript snapshot could not be read.")
        }
        return value
    }
    static func utteranceAudioProjection(_ text: String) throws -> (speaker: Speaker, sourceID: String, startMs: UInt64) {
        let value = try object(text)
        guard let source = value["source"] as? String,
              let sourceID = value["source_id"] as? String, !sourceID.isEmpty,
              let start = value["start_ms"] as? NSNumber,
              start.doubleValue.isFinite, start.doubleValue >= 0,
              start.doubleValue.rounded(.towardZero) == start.doubleValue,
              start.doubleValue < Double(UInt64.max) else {
            throw PennelBridgeError("invalid_response", "The utterance audio source could not be read.")
        }
        let speaker: Speaker
        switch source {
        case "me": speaker = .me
        case "others": speaker = .others
        default: throw PennelBridgeError("invalid_response", "Unknown physical audio source.")
        }
        return (speaker, sourceID, start.uint64Value)
    }
    static func audioTrack(_ track: MeetingAudioTrack) -> [String: Any] {
        var value: [String: Any] = ["speaker": track.speaker == .me ? "me" : "others", "available": track.available]
        switch track.relativePath {
        case "audio/microphone.m4a": value["source_id"] = "microphone"
        case "audio/system.m4a": value["source_id"] = "system_audio"
        case "audio/room.m4a": value["source_id"] = "room_microphone"
        default: break
        }
        return value
    }
    static func meeting(_ value: MeetingInfo) -> [String: Any] {
        ["id": value.id, "title": value.title, "start_iso": value.startIso,
         "duration_min": value.durationMin, "has_summary": value.hasSummary]
    }
    static func provider(_ value: ProviderAccount) -> [String: Any] {
        ["id": value.id, "kind": kind(value.kind), "name": value.name, "base_url": value.baseUrl]
    }
    static func history(_ value: HistoryEntry) -> [String: Any] {
        let status: String = switch value.status { case .done: "done"; case .failed: "failed"; case .unsaved: "unsaved" }
        return ["id": value.id, "created_at_ms": value.createdAtMs, "raw": value.raw, "text": value.text,
                "status": status, "error": optional(value.error), "has_audio": value.hasAudio]
    }
    static func passage(_ value: Passage) -> [String: Any] {
        let kind: String = switch value.kind { case .notes: "notes"; case .transcript: "transcript"; case .summary: "summary" }
        var result: [String: Any] = ["meeting_id": value.meetingId, "title": value.title, "start_iso": value.startIso,
                                    "kind": kind, "start_ms": optional(value.startMs), "text": value.text, "snippet": value.snippet]
        if let uid = value.meetingUid { result["meeting_uid"] = uid }
        if !value.utteranceIds.isEmpty {
            result["utterance_ids"] = value.utteranceIds
            result["utterance_fingerprints"] = value.utteranceFingerprints
        }
        return result
    }
    static func answer(_ value: MeetingAnswer) -> [String: Any] {
        ["text": value.text, "citations": value.citations.map { ["number": $0.number, "passage": passage($0.passage)] }]
    }
    static func template(_ value: MeetingTemplate) -> [String: Any] {
        ["id": value.id, "name": value.name, "builtin": value.builtin, "customized": value.customized]
    }
    static func inferenceProtocol(_ value: InferenceProtocol) -> String {
        switch value {
        case .openRouterDecisions: "open_router_decisions"
        case .openRouterSystemOne: "open_router_system_one"
        case .typeSafe: "type_safe"
        case .perplexity: "perplexity"
        case .mercury: "mercury"
        case .localDecider: "local_decider"
        case .tei: "tei"
        case .pennelCloud: "pennel_cloud"
        }
    }
    static func inferenceCapability(_ value: String) throws -> InferenceCapability {
        switch value {
        case "decision": .decision
        case "reranking": .reranking
        default: throw PennelBridgeError("invalid_request", "Unknown inference capability.")
        }
    }
    static func inferenceProtocol(_ value: InferenceProtocolInfo) -> [String: Any] {
        ["protocol": inferenceProtocol(value.protocol), "name": value.name, "default_base_url": value.defaultBaseUrl,
         "decision": value.decision, "reranking": value.reranking, "extraction": value.extraction, "requires_key": value.requiresKey,
         "allowed_provider_kinds": value.allowedProviderKinds.map(kind), "managed": value.managed]
    }
    static func inferenceIdentity(_ value: InferenceIdentity) -> [String: Any] {
        ["protocol": inferenceProtocol(value.protocol), "endpoint": value.endpoint,
         "requested_model": value.requestedModel, "requested_upstream": optional(value.requestedUpstream),
         "actual_model": optional(value.actualModel), "actual_revision": optional(value.actualRevision), "actual_provider": optional(value.actualProvider), "request_id": optional(value.requestId)]
    }
    static func inferenceAvailability(_ value: InferenceAvailability) -> [String: Any] {
        ["capability": value.capability == .decision ? "decision" : "reranking", "configured": value.configured,
         "available": value.available, "reason": optional(value.reason), "provider_id": optional(value.providerId),
         "model": optional(value.model), "protocol": optional(value.protocol.map(inferenceProtocol)), "endpoint": optional(value.endpoint)]
    }
    static func inferenceVerification(_ value: InferenceVerification) -> [String: Any] {
        ["verified": value.verified, "message": value.message, "identity": optional(value.identity.map(inferenceIdentity))]
    }
    static func cleanupVerification(_ value: CleanupVerificationAvailability) -> [String: Any] {
        ["enabled": value.enabled, "available": value.available, "reason_code": value.reasonCode,
         "provider_id": optional(value.providerId), "model": optional(value.model), "endpoint": optional(value.endpoint)]
    }
    static func account(_ value: CloudAccount) -> [String: Any] {
        ["email": value.email, "audio_seconds": value.audioSeconds, "tokens": value.tokens,
         "storage_bytes": value.storageBytes, "limit_audio_seconds": value.limitAudioSeconds,
         "limit_tokens": value.limitTokens, "limit_storage_bytes": value.limitStorageBytes]
    }
    static func device(_ value: CloudDevice) -> [String: Any] {
        ["id": value.id, "name": value.name, "platform": value.platform,
         "last_seen_iso": value.lastSeenIso, "current": value.current]
    }
    static func report(_ value: SyncReport) -> [String: Any] {
        let (status, error): (String, String?) = switch value.status {
        case .off: ("off", nil)
        case .signedOut: ("signed_out", nil)
        case .needsSetup: ("needs_setup", nil)
        case .needsRecoveryCode: ("needs_recovery_code", nil)
        case .vaultReset: ("vault_reset", nil)
        case .pending: ("pending", nil)
        case .synced: ("synced", nil)
        case .offline: ("offline", nil)
        case .storageFull(let message): ("storage_full", message)
        case .error(let message): ("error", message)
        }
        return ["status": status, "uploaded": value.uploaded, "downloaded": value.downloaded,
                "conflicts": value.conflicts, "last_success_ms": optional(value.lastSuccessMs), "error": optional(error)]
    }
    static func kind(_ value: ProviderKind) -> String {
        switch value {
        case .openAi: "open_ai"; case .openRouter: "open_router"; case .groq: "groq"; case .mistral: "mistral"
        case .ollama: "ollama"; case .lmStudio: "lm_studio"; case .custom: "custom"; case .local: "local"
        case .claudeCode: "claude_code"; case .codex: "codex"; case .openCode: "open_code"; case .copilot: "copilot"
        case .cursorAgent: "cursor_agent"; case .geminiCli: "gemini_cli"; case .appleSpeech: "apple_speech"
        case .appleIntelligence: "apple_intelligence"; case .pennelCloud: "pennel_cloud"
        }
    }
    static func kind(_ text: String) throws -> ProviderKind {
        guard let providerKind = ProviderKinds.all.first(where: { kind($0.kind) == text })?.kind else {
            throw PennelBridgeError("invalid_provider", "Unknown provider.")
        }
        return providerKind
    }
    static func recognition(_ value: SpeakerRecognitionJobSnapshot) -> [String: Any] {
        let stage: String = switch value.stage {
        case .preparing: "preparing"; case .uploading: "uploading"; case .recognizing: "recognizing"
        case .responseUnknown: "response_unknown"; case .interrupted: "interrupted"; case .applying: "applying"
        case .completed: "completed"; case .cancelled: "cancelled"; case .failed: "failed"
        }
        let error: Any = value.errorCode.map { ["code": $0, "message": value.errorMessage ?? ""] as Any } ?? NSNull()
        return ["id": value.jobId, "job_id": value.jobId, "meeting_uid": value.meetingUid, "meeting_id": value.meetingId, "stage": stage,
                "decoded_frames": value.decodedFrames, "total_frames": value.totalFrames,
                "uploaded_bytes": value.uploadedBytes, "total_bytes": value.uploadTotalBytes,
                "confirmed_chunks": value.confirmedChunks, "total_chunks": value.totalChunks,
                "recoverable": value.recoverable, "error": error]
    }
}

struct PennelBridgeError: LocalizedError, Sendable {
    let code: String
    let message: String
    init(_ code: String, _ message: String) { self.code = code; self.message = message }
    var errorDescription: String? { message }
}

@MainActor
final class NativeTranscriptProjectionCache {
    private struct Request: Sendable {
        let key: String
        let revision: UInt64
        let load: @Sendable () throws -> String
    }
    // The parsed JSON tree is immutable; only the owning main actor publishes its value copies.
    private struct Projection: @unchecked Sendable {
        let value: [String: Any]
        let revision: UInt64
        init(_ text: String) throws {
            value = try BridgeJSON.transcriptProjection(text)
            guard let number = value["revision"] as? NSNumber else { throw PennelBridgeError("invalid_response", "The transcript revision is unavailable.") }
            revision = number.uint64Value
        }
    }
    private var wanted: Request?
    private var cached: (key: String, projection: Projection)?
    private var failed: (key: String, revision: UInt64)?
    private var running = false
    private var changed: (@MainActor () -> Void)?

    func value(key: String, revision: UInt64, load: @escaping @Sendable () throws -> String, changed: @escaping @MainActor () -> Void) -> [String: Any]? {
        if wanted?.key != key { cached = nil; failed = nil }
        wanted = Request(key: key, revision: revision, load: load)
        self.changed = changed
        startLatest()
        return cached?.key == key ? cached?.projection.value : nil
    }

    func clear() {
        wanted = nil
        cached = nil
        failed = nil
    }

    private func startLatest() {
        guard !running, let request = wanted,
              !(cached?.key == request.key && (cached?.projection.revision ?? 0) >= request.revision),
              !(failed?.key == request.key && failed?.revision == request.revision) else { return }
        running = true
        Task {
            let result = await Task.detached(priority: .utility) { Result { try Projection(request.load()) } }.value
            running = false
            if wanted?.key == request.key {
                switch result {
                case .success(let projection):
                    if projection.revision < request.revision {
                        failed = (request.key, request.revision)
                    } else if cached?.key != request.key || (cached?.projection.revision ?? 0) <= projection.revision {
                        cached = (request.key, projection)
                        failed = nil
                        changed?()
                    }
                case .failure:
                    failed = (request.key, request.revision)
                }
            }
            startLatest()
        }
    }
}

struct BridgePayload: @unchecked Sendable {
    let values: [String: Any]
    func text(_ key: String, default fallback: String = "") -> String { values[key] as? String ?? fallback }
    func flag(_ key: String, default fallback: Bool = false) -> Bool { values[key] as? Bool ?? fallback }
    func number(_ key: String, default fallback: Double = 0) -> Double { (values[key] as? NSNumber)?.doubleValue ?? fallback }
    func required(_ key: String) throws -> String {
        guard let value = values[key] as? String, !value.isEmpty else {
            throw PennelBridgeError("invalid_request", "Missing \(key).")
        }
        return value
    }
}
