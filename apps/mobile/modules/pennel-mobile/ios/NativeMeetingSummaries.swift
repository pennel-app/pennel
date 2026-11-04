import Foundation

func invokeMeetingSummaryCommand(method: String, input: BridgePayload, engine: PennelEngine, language: InterfaceLanguage, keys: () -> ApiKeys) throws -> String {
    let id = try input.required("id")
    let template = input.text("template", default: engine.settings().defaultTemplate)
    switch method {
    case "meetings.summary":
        return try BridgeJSON.encode(BridgeJSON.summary(engine.meetingSummary(id: id)))
    case "meetings.summary.save", "meetings.summary.accept":
        guard let markdown = input.values["markdown"] as? String else {
            throw PennelBridgeError("invalid_request", "Missing summary text.")
        }
        let expectedRevision = try input.required("expected_revision")
        let expectedSourceRevision = try input.required("expected_source_revision")
        let document = method == "meetings.summary.accept"
            ? try engine.acceptMeetingSummary(id: id, expectedRevision: expectedRevision, expectedSourceRevision: expectedSourceRevision, markdown: markdown)
            : try engine.saveMeetingSummary(id: id, expectedRevision: expectedRevision, expectedSourceRevision: expectedSourceRevision, markdown: markdown)
        return try BridgeJSON.encode(BridgeJSON.summary(document))
    case "meetings.summary.history":
        return try BridgeJSON.encode(engine.meetingSummaryHistory(id: id).map(BridgeJSON.summaryRevision))
    case "meetings.summary.restore":
        return try BridgeJSON.encode(BridgeJSON.summary(engine.restoreMeetingSummary(id: id, revision: input.required("revision"), expectedRevision: input.required("expected_revision"), expectedSourceRevision: input.required("expected_source_revision"))))
    case "meetings.summary.generate":
        return try BridgeJSON.encode(BridgeJSON.summaryCandidate(engine.generateMeetingSummary(id: id, templateId: template, keys: keys())))
    case "meetings.shareableSummary":
        return try BridgeJSON.encode(BridgeJSON.optional(engine.shareableMeetingSummary(id: id).map(BridgeJSON.shareableSummary)))
    case "meetings.shareableSummary.generate":
        return try BridgeJSON.encode(BridgeJSON.shareableSummary(engine.generateShareableMeetingSummary(id: id, templateId: template, keys: keys())))
    case "meetings.export.preview":
        let scope: MeetingOutputScope
        switch try input.required("scope") {
        case "personal": scope = .personal
        case "shareable": scope = .shareable
        default: throw PennelBridgeError("invalid_request", "Unknown output scope.")
        }
        let requestedLanguage: InterfaceLanguage
        switch input.values["language"] as? String {
        case "en": requestedLanguage = .english
        case "nl": requestedLanguage = .dutch
        case nil: requestedLanguage = language
        default: throw PennelBridgeError("invalid_request", "Unsupported interface language.")
        }
        return try BridgeJSON.encode(BridgeJSON.meetingExport(engine.previewMeetingExport(id: id, scope: scope, includeTranscript: input.flag("include_transcript"), language: requestedLanguage)))
    default:
        throw PennelBridgeError("unsupported_operation", "This operation is not available on this device.")
    }
}

extension BridgeJSON {
    static func summary(_ value: MeetingSummaryDocument) -> [String: Any] {
        ["meeting_uid": value.meetingUid, "markdown": value.markdown, "revision": value.revision,
         "source_revision": value.sourceRevision, "edited": value.edited, "outdated": value.outdated]
    }

    static func summaryCandidate(_ value: MeetingSummaryCandidate) -> [String: Any] {
        ["meeting_uid": value.meetingUid, "markdown": value.markdown,
         "revision": value.revision, "source_revision": value.sourceRevision]
    }

    static func summaryRevision(_ value: MeetingSummaryRevision) -> [String: Any] {
        ["revision": value.revision, "markdown": value.markdown,
         "source_revision": value.sourceRevision, "created_at": value.createdAt]
    }

    static func shareableSummary(_ value: ShareableMeetingSummary) -> [String: Any] {
        ["meeting_uid": value.meetingUid, "markdown": value.markdown, "source_revision": value.sourceRevision,
         "generated_at": value.generatedAt, "provider_id": value.providerId, "model": value.model,
         "provenance": value.provenance, "outdated": value.outdated]
    }

    static func meetingExport(_ value: MeetingExport) -> [String: Any] {
        ["title": value.title, "markdown": value.markdown, "html": value.html]
    }

    static func preparedMeetingExport(_ input: BridgePayload) throws -> MeetingExport {
        guard let output = input.values["output"] as? [String: Any],
              let title = output["title"] as? String,
              let markdown = output["markdown"] as? String,
              let html = output["html"] as? String else {
            throw PennelBridgeError("invalid_request", "Prepare and review the document before sharing it.")
        }
        return MeetingExport(title: title, markdown: markdown, html: html)
    }
}
