import Foundation
import Testing
@testable internal import PennelMobile

struct ConversationRuntimeTests {
    @Test func selectedMobileConversationAndExactDraftSurviveEngineRestart() throws {
        let environmentHome = try #require(ProcessInfo.processInfo.environment["HOME"])
        #expect(URL(fileURLWithPath: environmentHome).resolvingSymlinksInPath() == URL(fileURLWithPath: NSHomeDirectory()).resolvingSymlinksInPath())
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let scope: [String: Any] = ["kind": "library"]
        let draft = "Draft retained through native restart.\n\nSecond paragraph."
        func prepare() throws -> String {
            let engine = try PennelEngine.newPreview(dataDir: directory.path)
            let read = try BridgeJSON.object(invokeConversationCommand(method: "conversations.read", requestJSON: BridgeJSON.encode(["scope": scope]), engine: engine, keys: { ApiKeys(byProvider: [:]) }))
            let conversation = try #require(read["conversation"] as? [String: Any])
            let id = try #require(conversation["id"] as? String)
            let revision = (conversation["draft_revision"] as? NSNumber)?.uint64Value ?? 0
            _ = try invokeConversationCommand(method: "conversations.update", requestJSON: BridgeJSON.encode(["scope": scope, "action": "draft", "conversation_id": id, "draft": draft, "expected_draft_revision": revision]), engine: engine, keys: { ApiKeys(byProvider: [:]) })
            return id
        }
        let id = try prepare()
        let reopened = try PennelEngine.newPreview(dataDir: directory.path)
        let read = try BridgeJSON.object(invokeConversationCommand(method: "conversations.read", requestJSON: BridgeJSON.encode(["scope": scope]), engine: reopened, keys: { ApiKeys(byProvider: [:]) }))
        #expect(read["selected_conversation_id"] as? String == id)
        #expect((read["conversation"] as? [String: Any])?["draft"] as? String == draft)
    }

    @Test func nativeConversationCommandsRetainIdentitiesAndNeverReadKeysForLocalEdits() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let engine = try PennelEngine.newPreview(dataDir: directory.path)
        var keyReads = 0
        func invoke(_ method: String, _ request: [String: Any]) throws -> [String: Any] {
            try BridgeJSON.object(invokeConversationCommand(method: method, requestJSON: BridgeJSON.encode(request), engine: engine, keys: {
                keyReads += 1
                return ApiKeys(byProvider: [:])
            }))
        }
        let scope: [String: Any] = ["kind": "library"]
        let first = try invoke("conversations.read", ["scope": scope])
        let original = try #require((first["conversation"] as? [String: Any])?["id"] as? String)
        let created = try invoke("conversations.update", ["scope": scope, "action": "create", "title": "Owned conversation"])
        let id = try #require((created["conversation"] as? [String: Any])?["id"] as? String)
        #expect(id != original)
        let revision = ((created["conversation"] as? [String: Any])?["draft_revision"] as? NSNumber)?.uint64Value ?? 0
        let draft = "Exact mobile draft\n\n@Els — private working text."
        let saved = try invoke("conversations.update", ["scope": scope, "action": "draft", "conversation_id": id, "draft": draft, "expected_draft_revision": revision])
        #expect((saved["conversation"] as? [String: Any])?["draft"] as? String == draft)
        let reloaded = try invoke("conversations.read", ["scope": scope, "conversation_id": id])
        #expect((reloaded["conversation"] as? [String: Any])?["draft"] as? String == draft)
        #expect(throws: (any Error).self) {
            _ = try invoke("conversations.update", ["scope": scope, "action": "draft", "conversation_id": id, "draft": "Stale draft", "expected_draft_revision": revision])
        }
        let renamed = try invoke("conversations.update", ["scope": scope, "action": "rename", "conversation_id": id, "title": "Renamed conversation"])
        #expect((renamed["conversation"] as? [String: Any])?["title"] as? String == "Renamed conversation")
        _ = try invoke("conversations.update", ["scope": scope, "action": "delete", "conversation_id": id])
        let remaining = try invoke("conversations.read", ["scope": scope])
        let ids = (remaining["conversations"] as? [[String: Any]] ?? []).compactMap { $0["id"] as? String }
        #expect(ids.contains(original))
        #expect(!ids.contains(id))
        #expect(keyReads == 0)
    }
}
