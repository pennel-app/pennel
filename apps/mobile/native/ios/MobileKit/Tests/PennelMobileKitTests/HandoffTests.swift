import Foundation
import Testing
@testable import PennelMobileKit

private func tempDirectory() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("pennel-handoff-\(UUID().uuidString)")
}

private func whole(_ date: Date) -> Date {
    Date(timeIntervalSince1970: (date.timeIntervalSince1970 * 1000).rounded() / 1000)
}

struct HandoffClientTests {
    let store: HandoffStore
    let now = whole(Date())

    init() {
        store = HandoffStore(directory: tempDirectory())
    }

    private func client() -> HandoffClient {
        HandoffClient(store: store, pending: PendingHandoff())
    }

    @Test func session_preview_fields_round_trip_and_legacy_states_decode() throws {
        let request = UUID()
        let state = SessionState(phase: .listening, expiresAt: now.addingTimeInterval(300), heartbeatAt: now, level: 0.3, partialText: "Een voorlopig woord", previewRequestID: request)
        try store.write(state, as: .session)
        #expect(store.read(SessionState.self, as: .session) == state)
        let legacy = SessionState(phase: .ready, expiresAt: now.addingTimeInterval(300), heartbeatAt: now, level: 0)
        let data = try JSONEncoder().encode(legacy)
        let decoded = try JSONDecoder().decode(SessionState.self, from: data)
        #expect(decoded.partialText == nil)
        #expect(decoded.previewRequestID == nil)
    }

    @Test func failure_codes_round_trip_and_legacy_results_still_decode() throws {
        let result = HandoffResult(id: UUID(), text: nil, error: "No connection", errorCode: "network", at: now)
        let encoder = JSONEncoder()
        let data = try encoder.encode(result)
        #expect(try JSONDecoder().decode(HandoffResult.self, from: data) == result)
        var legacy = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        legacy.removeValue(forKey: "errorCode")
        let old = try JSONDecoder().decode(HandoffResult.self, from: JSONSerialization.data(withJSONObject: legacy))
        #expect(old.errorCode == nil)
        #expect(old.error == result.error)
    }

    @Test func result_is_taken_once_and_only_for_pending_id() throws {
        let client = client()
        let id = try client.send(.start, now: now)
        #expect(try #require(store.read(HandoffCommand.self, as: .command)).id == id)

        try store.write(HandoffResult(id: id, text: "hallo", error: nil, at: now), as: .result)
        let first = client.takeResult()
        #expect(first?.text == "hallo")
        #expect(client.takeResult() == nil)
        #expect(store.read(HandoffResult.self, as: .result) == nil)
    }

    @Test func foreign_result_is_ignored() throws {
        let mine = client()
        let other = client()
        _ = try mine.send(.start, now: now)
        let foreign = HandoffResult(id: UUID(), text: "van iemand anders", error: nil, at: now)
        try store.write(foreign, as: .result)

        #expect(mine.takeResult() == nil)
        #expect(other.takeResult() == nil)
        #expect(store.read(HandoffResult.self, as: .result) == foreign)
    }

    @Test func second_instance_never_takes_the_result_of_the_first() throws {
        let first = client()
        let second = client()
        let id = try first.send(.start, now: now)
        _ = try second.send(.cancel, now: now)
        try store.write(HandoffResult(id: id, text: "eerste", error: nil, at: now), as: .result)
        #expect(second.takeResult() == nil)
    }

    @Test func pending_id_survives_a_re_shown_keyboard_without_inserting_twice() throws {
        let slot = PendingHandoff()
        let shown = HandoffClient(store: store, pending: slot)
        let id = try shown.send(.start, now: now)

        let reshown = HandoffClient(store: store, pending: slot)
        try store.write(HandoffResult(id: id, text: "eens", error: nil, at: now), as: .result)
        #expect(reshown.takeResult()?.text == "eens")
        #expect(shown.takeResult() == nil)
        #expect(reshown.takeResult() == nil)
    }

    @Test func default_clients_in_one_process_share_the_pending_id() throws {
        let directory = tempDirectory()
        let store = HandoffStore(directory: directory)
        let id = try HandoffClient(store: store).send(.start, now: now)
        try store.write(HandoffResult(id: id, text: "terug", error: nil, at: now), as: .result)
        #expect(HandoffClient(store: HandoffStore(directory: directory)).takeResult()?.text == "terug")
    }

    @Test func send_failure_throws_and_leaves_no_pending_id() throws {
        let blocker = tempDirectory()
        try Data().write(to: blocker)
        let unwritable = HandoffStore(directory: blocker.appendingPathComponent("handoff"))
        let slot = PendingHandoff()
        let client = HandoffClient(store: unwritable, pending: slot)

        #expect(throws: (any Error).self) { try client.send(.start, now: now) }
        #expect(slot.current == nil)
    }

    @Test func stale_heartbeat_reads_as_off() throws {
        let client = client()
        #expect(client.session(now: now) == .off)

        try store.write(SessionState(phase: .ready, expiresAt: now.addingTimeInterval(300), heartbeatAt: now, level: 0), as: .session)
        #expect(client.session(now: now.addingTimeInterval(4.9)) == .ready)
        #expect(client.session(now: now.addingTimeInterval(5.1)) == .off)
    }

    @Test func expired_session_reads_as_off() throws {
        try store.write(SessionState(phase: .ready, expiresAt: now.addingTimeInterval(1), heartbeatAt: now.addingTimeInterval(2), level: 0), as: .session)
        #expect(client().session(now: now.addingTimeInterval(2)) == .off)
    }

    @Test func last_uninserted_only_returns_recent_results_and_is_not_repeated() throws {
        let client = client()
        let recent = HandoffResult(id: UUID(), text: "laatste", error: nil, at: now.addingTimeInterval(-9 * 60))
        try store.write(recent, as: .result)
        #expect(client.lastUninserted(now: now) == recent)
        client.markInserted(recent)
        #expect(client.lastUninserted(now: now) == nil)

        try store.write(HandoffResult(id: UUID(), text: "oud", error: nil, at: now.addingTimeInterval(-11 * 60)), as: .result)
        #expect(client.lastUninserted(now: now) == nil)
    }

    @Test func stale_result_is_never_taken_and_is_removed() throws {
        let client = client()
        let id = try client.send(.stop, now: now)
        try store.write(HandoffResult(id: id, text: "oud", error: nil, at: now), as: .result)

        let later = now.addingTimeInterval(HandoffClient.resultLifetime + 1)
        #expect(client.takeResult(now: later) == nil)
        #expect(store.read(HandoffResult.self, as: .result) == nil)
    }

    @Test func pending_id_expires_so_a_late_keyboard_never_auto_inserts() throws {
        let slot = PendingHandoff()
        let client = HandoffClient(store: store, pending: slot)
        let id = try client.send(.stop, now: now)
        let later = now.addingTimeInterval(HandoffClient.resultLifetime + 1)
        // A result written just now for an id the keyboard asked for long ago.
        try store.write(HandoffResult(id: id, text: "laat", error: nil, at: later), as: .result)

        #expect(client.takeResult(now: later) == nil)
        #expect(slot.current == nil)
        #expect(client.lastUninserted(now: later)?.text == "laat")
    }

    @Test func purge_keeps_a_recent_result_and_removes_an_expired_one() throws {
        let recent = HandoffResult(id: UUID(), text: "nieuw", error: nil, at: now)
        try store.write(recent, as: .result)
        store.purgeExpiredResult(now: now.addingTimeInterval(HandoffClient.resultLifetime), lifetime: HandoffClient.resultLifetime)
        #expect(store.read(HandoffResult.self, as: .result) == recent)

        store.purgeExpiredResult(now: now.addingTimeInterval(HandoffClient.resultLifetime + 1), lifetime: HandoffClient.resultLifetime)
        #expect(store.read(HandoffResult.self, as: .result) == nil)
    }

    @Test func atomic_write_never_leaves_partial_json() async throws {
        let state = SessionState(phase: .listening, expiresAt: nil, heartbeatAt: now, level: 0.5)
        let other = SessionState(phase: .ready, expiresAt: now, heartbeatAt: now, level: 0.25)
        try store.write(state, as: .session)

        let store = store
        let writer = Task.detached {
            for index in 0..<400 {
                try store.write(index.isMultiple(of: 2) ? other : state, as: .session)
            }
        }
        var garbage = 0
        for _ in 0..<2000 {
            let read = store.read(SessionState.self, as: .session)
            if read != state && read != other { garbage += 1 }
        }
        try await writer.value
        #expect(garbage == 0)
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: store.directory.path).filter { $0.hasSuffix(".tmp") }
        #expect(leftovers.isEmpty)
    }

    @Test func signal_posts_reach_observers() async throws {
        let received = Counter()
        let token = HandoffSignal.result.observe { received.increment() }
        HandoffSignal.result.post()
        for _ in 0..<50 where received.value == 0 {
            try await Task.sleep(for: .milliseconds(50))
        }
        #expect(received.value >= 1)
        withExtendedLifetime(token) {}
    }
}

final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var value: Int { lock.withLock { count } }
    func increment() { lock.withLock { count += 1 } }
}
