import Foundation
import Testing
@testable internal import PennelMobile

struct SummaryRuntimeTests {
    @Test func native_summary_edits_keep_exact_revisions_history_and_conflicts() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let session = try engine.startMeeting(title: "Owner summary", keys: ApiKeys(byProvider: [:]))
        try session.setNotes(text: "Personal note sentinel")
        try session.stop()
        let id = session.id()
        var keyReads = 0
        func invoke(_ method: String, _ values: [String: Any]) throws -> String {
            try invokeMeetingSummaryCommand(method: method, input: BridgePayload(values: values), engine: engine, language: .english, keys: {
                keyReads += 1
                return ApiKeys(byProvider: [:])
            })
        }
        let before = try BridgeJSON.object(invoke("meetings.summary", ["id": id]))
        let revision = try #require(before["revision"] as? String)
        let source = try #require(before["source_revision"] as? String)
        let text = "## Reviewed summary\n\nExact owner wording — first version.\n"
        let saved = try BridgeJSON.object(invoke("meetings.summary.save", ["id": id, "expected_revision": revision, "expected_source_revision": source, "markdown": text]))
        #expect(saved["markdown"] as? String == text)
        #expect(saved["edited"] as? Bool == true)
        let nextRevision = try #require(saved["revision"] as? String)
        #expect(nextRevision != revision)
        #expect(throws: (any Error).self) {
            _ = try invoke("meetings.summary.save", ["id": id, "expected_revision": revision, "expected_source_revision": source, "markdown": "Stale replacement"])
        }
        _ = try invoke("meetings.summary.save", ["id": id, "expected_revision": nextRevision, "expected_source_revision": source, "markdown": "Second owner version"])
        let historyData = try #require(try invoke("meetings.summary.history", ["id": id]).data(using: .utf8))
        let history = try #require(try JSONSerialization.jsonObject(with: historyData) as? [[String: Any]])
        #expect(history.contains { $0["revision"] as? String == nextRevision && $0["markdown"] as? String == text })
        let current = try BridgeJSON.object(invoke("meetings.summary", ["id": id]))
        let restored = try BridgeJSON.object(invoke("meetings.summary.restore", ["id": id, "revision": nextRevision, "expected_revision": try #require(current["revision"] as? String), "expected_source_revision": source]))
        #expect(restored["markdown"] as? String == text)
        try engine.setMeetingNotes(id: id, text: "Personal notes changed after generation")
        let changed = try BridgeJSON.object(invoke("meetings.summary", ["id": id]))
        #expect(changed["outdated"] as? Bool == true)
        let changedSource = try #require(changed["source_revision"] as? String)
        let edited = try BridgeJSON.object(invoke("meetings.summary.save", ["id": id, "expected_revision": try #require(changed["revision"] as? String), "expected_source_revision": changedSource, "markdown": "Edited stale summary"]))
        #expect(edited["outdated"] as? Bool == true)
        #expect(throws: (any Error).self) {
            _ = try invoke("meetings.summary.accept", ["id": id, "expected_revision": try #require(edited["revision"] as? String), "expected_source_revision": source, "markdown": "Candidate from old source"])
        }
        let accepted = try BridgeJSON.object(invoke("meetings.summary.accept", ["id": id, "expected_revision": try #require(edited["revision"] as? String), "expected_source_revision": changedSource, "markdown": "Reviewed against current source"]))
        #expect(accepted["markdown"] as? String == "Reviewed against current source")
        #expect(accepted["outdated"] as? Bool == false)
        #expect(keyReads == 0)
    }

    @Test func native_scoped_preview_never_falls_back_to_personal_notes() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let session = try engine.startMeeting(title: "Output boundary", keys: ApiKeys(byProvider: [:]))
        try session.setNotes(text: "Personal note sentinel")
        try session.stop()
        func preview(_ scope: String?) throws -> String {
            var payload: [String: Any] = ["id": session.id(), "include_transcript": false]
            if let scope { payload["scope"] = scope }
            return try invokeMeetingSummaryCommand(method: "meetings.export.preview", input: BridgePayload(values: payload), engine: engine, language: .english, keys: { Issue.record("Local export must not read credentials"); return ApiKeys(byProvider: [:]) })
        }
        #expect(throws: (any Error).self) { _ = try preview(nil) }
        #expect(throws: (any Error).self) { _ = try preview("shareable") }
        let intentionalPersonal = try BridgeJSON.object(preview("personal"))
        #expect((intentionalPersonal["markdown"] as? String)?.contains("Personal note sentinel") == true)
        let reviewed = ["title": "Reviewed", "markdown": "Exact reviewed bytes\n", "html": "<p>Exact reviewed bytes</p>"]
        let output = try BridgeJSON.preparedMeetingExport(BridgePayload(values: ["output": reviewed]))
        #expect(output.markdown == reviewed["markdown"])
        #expect(output.html == reviewed["html"])
        #expect(throws: (any Error).self) { _ = try BridgeJSON.preparedMeetingExport(BridgePayload(values: ["id": session.id()])) }
    }

    @Test func automatic_summary_requires_explicit_policy_and_does_not_retry_after_failure_or_restart() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        let session = try engine.startMeeting(title: "Automatic admission", keys: ApiKeys(byProvider: [:]))
        try session.setNotes(text: "Saved owner notes")
        try session.stop()
        let id = session.id()
        let keys = ApiKeys(byProvider: [:])
        var settings = engine.settings()
        #expect(settings.summaryAutomation == .unanswered)
        #expect(try engine.summarizeMeetingAutomatically(id: id, templateId: settings.defaultTemplate, keys: keys) == nil)
        settings.summaryAutomation = .manual
        try engine.updateSettings(settings: settings)
        #expect(try engine.summarizeMeetingAutomatically(id: id, templateId: settings.defaultTemplate, keys: keys) == nil)
        settings.summaryAutomation = .automatic
        try engine.updateSettings(settings: settings)
        #expect(throws: (any Error).self) { _ = try engine.summarizeMeetingAutomatically(id: id, templateId: settings.defaultTemplate, keys: keys) }
        #expect(try engine.summarizeMeetingAutomatically(id: id, templateId: settings.defaultTemplate, keys: keys) == nil)
        let reopened = try PennelEngine.newPreview(dataDir: directory.path)
        #expect(try reopened.summarizeMeetingAutomatically(id: id, templateId: settings.defaultTemplate, keys: keys) == nil)
        #expect(reopened.meetingNotes(id: id) == "Saved owner notes")
    }
}
