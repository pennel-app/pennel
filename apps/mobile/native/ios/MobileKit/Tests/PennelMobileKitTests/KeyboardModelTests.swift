import Foundation
import PennelMobileKit
import Testing

typealias KeyboardModel = KeyboardSessionModel

@MainActor
final class Harness {
    let store = HandoffStore(directory: FileManager.default.temporaryDirectory.appendingPathComponent("pennel-keyboard-\(UUID().uuidString)"))
    let start = Date(timeIntervalSince1970: 1_800_000_000)
    var now: Date
    var inserted: [String] = []
    var document = UUID()
    var generation: UInt64 = 0
    var visible = true

    init() { now = start }

    func model(fullAccess: Bool = true, pending: PendingHandoff = PendingHandoff(), lifetime: UUID = UUID()) -> KeyboardModel {
        KeyboardModel(
            hasFullAccess: fullAccess,
            client: HandoffClient(store: store, pending: pending),
            now: { [unowned self] in now },
            context: { [unowned self] in
                visible ? KeyboardInsertionContext(document: document, lifetime: lifetime, generation: generation) : nil
            },
            onResult: { [unowned self] in inserted.append($0) })
    }

    func session(_ phase: SessionPhase, heartbeat: Date? = nil) throws {
        try store.write(
            SessionState(phase: phase, expiresAt: now.addingTimeInterval(300), heartbeatAt: heartbeat ?? now, level: 0),
            as: .session)
    }

    var lastCommand: HandoffCommand? { store.read(HandoffCommand.self, as: .command) }

    func answer(_ text: String, to id: UUID) throws {
        try store.write(HandoffResult(id: id, text: text, error: nil, at: now), as: .result)
    }
}

@MainActor
struct KeyboardModelTests {
    let h = Harness()

    @Test func provisional_text_belongs_to_this_live_start_and_is_never_inserted() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        let start = try #require(h.lastCommand)
        try h.store.write(SessionState(phase: .listening, expiresAt: h.now.addingTimeInterval(300),
            heartbeatAt: h.now, level: 0, partialText: "Hallo world", previewRequestID: start.id), as: .session)
        model.refresh()
        #expect(model.partialText == "Hallo world")
        #expect(h.inserted.isEmpty)
        let recreated = h.model()
        recreated.refresh()
        #expect(recreated.partialText == nil)
        try h.store.write(SessionState(phase: .listening, expiresAt: h.now.addingTimeInterval(300),
            heartbeatAt: h.now, level: 0, partialText: "Foreign preview", previewRequestID: UUID()), as: .session)
        model.refresh()
        #expect(model.partialText == nil)
        #expect(h.inserted.isEmpty)
    }

    @Test func stop_context_change_and_stale_heartbeat_clear_provisional_text() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        let start = try #require(h.lastCommand)
        try h.store.write(SessionState(phase: .listening, expiresAt: h.now.addingTimeInterval(300),
            heartbeatAt: h.now, level: 0, partialText: "Preview", previewRequestID: start.id), as: .session)
        model.refresh()
        #expect(model.partialText == "Preview")
        model.contextDidChange()
        #expect(model.partialText == nil)
        model.refresh()
        #expect(model.partialText == nil)
        h.now = h.now.addingTimeInterval(6)
        model.refresh()
        #expect(model.partialText == nil)
        #expect(h.inserted.isEmpty)
    }

    @Test func no_full_access_shows_settings_hint_and_letters_still_work() throws {
        try h.session(.ready)
        let model = h.model(fullAccess: false)
        model.refresh()
        #expect(model.pad == .needsFullAccess)
        model.tapPill()
        model.holdBegan()
        #expect(h.lastCommand == nil)
        #expect(Copy.Keyboard.fullAccessNeeded.contains("Typen werkt ook zonder"))
        #expect(KeyboardLayout.qwerty.rows.count == 3)
    }

    @Test func session_off_does_not_launch_or_send_a_command() {
        let model = h.model()
        model.refresh()
        #expect(model.pad == .sessionOff)
        model.tapPill()
        #expect(model.pad == .sessionOff)
        #expect(h.lastCommand == nil)
    }

    @Test func unavailable_session_remains_truthful_after_hold() {
        let model = h.model()
        model.refresh()
        model.holdBegan()
        #expect(model.pad == .sessionOff)
        #expect(h.lastCommand == nil)
    }

    @Test func hold_sends_start_then_stop() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.holdBegan()
        let start = try #require(h.lastCommand)
        #expect(start.action == .start)
        #expect(model.pad == .listening)
        model.holdEnded()
        let stop = try #require(h.lastCommand)
        #expect(stop.action == .stop)
        #expect(stop.id != start.id)
        #expect(model.pad == .processing)
    }

    @Test func tap_tap_sends_start_then_stop() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        #expect(h.lastCommand?.action == .start)
        #expect(model.pad == .listening)
        model.holdEnded()
        #expect(h.lastCommand?.action == .start)
        model.tapPill()
        #expect(h.lastCommand?.action == .stop)
        #expect(model.pad == .processing)
    }

    @Test func release_after_a_hands_free_start_does_not_stop() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.holdEnded()
        #expect(h.lastCommand?.action == .start)
    }

    @Test func cancel_sends_cancel() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.cancel()
        #expect(h.lastCommand?.action == .cancel)
        #expect(model.pad == .ready)
    }

    @Test func taps_while_processing_send_nothing() throws {
        try h.session(.processing)
        let model = h.model()
        model.refresh()
        #expect(model.pad == .processing)
        model.tapPill()
        model.holdBegan()
        #expect(h.lastCommand == nil)
    }

    @Test func result_inserted_once() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        try h.answer("hallo wereld", to: stop.id)
        model.refresh()
        model.refresh()
        #expect(h.inserted == ["hallo wereld"])
        #expect(model.pad == .ready)
    }

    @Test func foreign_result_is_never_inserted() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        try h.answer("van iemand anders", to: UUID())
        model.refresh()
        #expect(h.inserted.isEmpty)
    }

    @Test func error_result_shows_the_message_and_retry_clears_it() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        try h.store.write(HandoffResult(id: stop.id, text: nil, error: "Geen spraak gehoord.", at: h.now), as: .result)
        model.refresh()
        #expect(model.pad == .failed("No speech recognized. Try again."))
        #expect(h.inserted.isEmpty)
        model.retry()
        #expect(model.pad == .ready)
    }

    @Test func handoff_failure_code_uses_the_keyboard_language_and_never_inserts_diagnostics() throws {
        try h.session(.ready)
        let model = KeyboardModel(hasFullAccess: true, client: HandoffClient(store: h.store, pending: PendingHandoff()),
            now: { h.now }, context: { .init(document: h.document, lifetime: h.document, generation: h.generation) },
            copy: KeyboardCopy(language: .dutch), onResult: { h.inserted.append($0) })
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        try h.store.write(HandoffResult(id: stop.id, text: nil, error: "No connection", errorCode: "network", at: h.now), as: .result)
        model.refresh()
        #expect(model.pad == .failed("Geen verbinding. Probeer opnieuw."))
        #expect(h.inserted.isEmpty)
    }

    @Test func provider_authentication_failures_keep_the_provider_recovery_action() {
        let english = KeyboardCopy(language: .english)
        let dutch = KeyboardCopy(language: .dutch)
        for code in ["provider_auth", "http_401", "http_403"] {
            #expect(english.failure(code: code, message: "Private diagnostics") == "Check your provider API key and model access in Pennel Settings.")
            #expect(dutch.failure(code: code, message: "Private diagnostics") == "Controleer de API-sleutel en modeltoegang van je provider bij Instellingen in Pennel.")
        }
        #expect(english.failure(code: nil, message: "API-sleutel geweigerd") == "Check your provider API key and model access in Pennel Settings.")
    }

    @Test func dismiss_and_reappear_offers_last_result_once() throws {
        try h.session(.ready)
        let first = h.model()
        first.refresh()
        first.tapPill()
        first.tapPill()
        let stop = try #require(h.lastCommand)
        try h.answer("gemist", to: stop.id)

        let reappeared = h.model()
        reappeared.refresh()
        #expect(h.inserted.isEmpty)
        #expect(reappeared.lastResult?.text == "gemist")
        reappeared.insertLast()
        reappeared.insertLast()
        #expect(h.inserted == ["gemist"])
        #expect(reappeared.lastResult == nil)

        let third = h.model()
        third.refresh()
        #expect(third.lastResult == nil)
        #expect(h.inserted == ["gemist"])
    }

    @Test func same_process_reappearance_requires_explicit_recovery() throws {
        let pending = PendingHandoff()
        try h.session(.ready)
        let first = h.model(pending: pending)
        first.refresh()
        first.tapPill()
        first.tapPill()
        try h.answer("een keer", to: try #require(h.lastCommand).id)

        let reappeared = h.model(pending: pending)
        reappeared.refresh()
        first.refresh()
        reappeared.refresh()
        #expect(h.inserted.isEmpty)
        #expect(reappeared.lastResult?.text == "een keer")
        reappeared.insertLast()
        reappeared.insertLast()
        #expect(h.inserted == ["een keer"])
    }

    @Test func stop_in_flight_is_not_undone_by_a_listening_session_and_never_sent_twice() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        try h.session(.listening)
        model.tapPill()
        let stop = try #require(h.lastCommand)
        #expect(stop.action == .stop)
        model.refresh()
        #expect(model.pad == .processing)
        model.tapPill()
        model.holdEnded()
        #expect(try #require(h.lastCommand).id == stop.id)
        try h.answer("eerste stop", to: stop.id)
        model.refresh()
        #expect(h.inserted == ["eerste stop"])
    }

    @Test func consumed_result_does_not_rearm_processing() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        try h.session(.processing)
        try h.answer("klaar", to: stop.id)
        model.refresh()
        #expect(h.inserted == ["klaar"])
        try h.session(.ready)
        h.now = h.start.addingTimeInterval(0.2)
        try h.session(.ready)
        model.refresh()
        #expect(model.pad == .ready)
        model.tapPill()
        #expect(h.lastCommand?.action == .start)
    }

    @Test func stale_session_reads_off() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        #expect(model.pad == .ready)
        h.now = h.start.addingTimeInterval(6)
        model.refresh()
        #expect(model.pad == .sessionOff)
        model.tapPill()
        #expect(h.lastCommand == nil)
    }

    @Test func listening_resets_when_the_session_goes_off() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        h.now = h.start.addingTimeInterval(6)
        model.refresh()
        #expect(model.pad == .sessionOff)
    }

    @Test func a_start_the_app_never_acted_on_does_not_stay_listening() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        h.now = h.start.addingTimeInterval(2)
        try h.session(.ready)
        model.refresh()
        #expect(model.pad == .ready)
    }

    @Test func field_change_defers_result_and_never_inserts_into_another_document() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        h.document = UUID()
        model.contextDidChange()
        try h.answer("keep for recovery", to: stop.id)
        model.refresh()
        #expect(h.inserted.isEmpty)
        #expect(model.lastResult?.text == "keep for recovery")
        model.insertLast()
        model.refresh()
        #expect(h.inserted == ["keep for recovery"])
        #expect(model.lastResult == nil)
    }

    @Test func cursor_generation_change_defers_delivery() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        h.generation += 1
        try h.answer("cursor moved", to: stop.id)
        model.refresh()
        #expect(h.inserted.isEmpty)
        #expect(model.lastResult?.text == "cursor moved")
    }

    @Test func returning_to_the_original_context_does_not_revalidate_a_pending_result() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        model.contextDidChange()
        try h.answer("explicit recovery", to: stop.id)
        model.refresh()
        #expect(h.inserted.isEmpty)
        #expect(model.lastResult?.text == "explicit recovery")
    }

    @Test func dismiss_while_recording_stops_capture_and_defers_delivery() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.contextDidChange()
        #expect(h.lastCommand?.action == .stop)
        h.visible = false
        try h.answer("after dismissal", to: try #require(h.lastCommand).id)
        model.refresh()
        model.insertLast()
        #expect(h.inserted.isEmpty)
    }

    @Test func processing_cancel_discards_a_late_result_instead_of_offering_it() throws {
        try h.session(.ready)
        let model = h.model()
        model.refresh()
        model.tapPill()
        model.tapPill()
        let stop = try #require(h.lastCommand)
        model.cancel()
        #expect(h.lastCommand?.action == .cancel)
        try h.answer("cancelled", to: stop.id)
        model.refresh()
        #expect(h.inserted.isEmpty)
        #expect(model.lastResult == nil)
    }

    @Test func cancelled_late_result_is_not_offered_by_a_recreated_keyboard_in_the_same_process() throws {
        let pending = PendingHandoff()
        try h.session(.ready)
        let first = h.model(pending: pending)
        first.refresh()
        first.tapPill()
        first.tapPill()
        let stop = try #require(h.lastCommand)
        first.cancel()
        try h.answer("cancelled", to: stop.id)
        let second = h.model(pending: pending)
        second.refresh()
        #expect(second.lastResult == nil)
        #expect(h.store.read(HandoffResult.self, as: .result) == nil)
        #expect(h.inserted.isEmpty)
    }
}

struct KeyboardLayoutTests {
    @Test func azerty_swaps_the_letters_qwerty_keeps() {
        #expect(KeyboardLayout.qwerty.rows[0].first == "q")
        #expect(KeyboardLayout.azerty.rows[0].first == "a")
        #expect(KeyboardLayout.azerty.rows.count == 3)
    }

    @Test func stored_choice_defaults_to_qwerty_and_ignores_garbage() {
        let defaults = UserDefaults(suiteName: "pennel-layout-\(UUID().uuidString)")!
        #expect(KeyboardLayout.stored(in: defaults) == .qwerty)
        KeyboardLayout.azerty.store(in: defaults)
        #expect(KeyboardLayout.stored(in: defaults) == .azerty)
        defaults.set("dvorak", forKey: KeyboardLayout.defaultsKey)
        #expect(KeyboardLayout.stored(in: defaults) == .qwerty)
    }
}
