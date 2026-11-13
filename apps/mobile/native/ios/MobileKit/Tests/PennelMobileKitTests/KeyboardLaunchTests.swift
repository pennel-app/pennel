import Foundation
import PennelMobileKit
import Testing

@MainActor
struct KeyboardLaunchTests {
    let h = Harness()

    @Test func launch_link_uses_the_bundle_scheme_and_the_app_start_route() {
        #expect(KeyboardModel.launchURL(scheme: "pennel-preview")?.absoluteString == "pennel-preview://sessie/start")
        #expect(KeyboardModel.launchURL(scheme: nil)?.absoluteString == "pennel://sessie/start")
        #expect(KeyboardModel.launchURL(scheme: "https") == nil)
    }

    @Test func requested_launch_starts_listening_once_the_session_is_ready() throws {
        let model = h.model()
        model.refresh()
        #expect(model.requestLaunch())
        #expect(h.lastCommand == nil)
        h.now = h.now.addingTimeInterval(8)
        try h.session(.ready)
        model.refresh()
        #expect(model.pad == .listening)
        #expect(h.lastCommand?.action == .start)
        let start = try #require(h.lastCommand)
        try h.session(.listening)
        model.refresh()
        try h.session(.ready)
        h.now = h.now.addingTimeInterval(2)
        model.refresh()
        #expect(h.lastCommand?.id == start.id)
    }

    @Test func launch_request_survives_a_recreated_keyboard() throws {
        let defaults = try #require(UserDefaults(suiteName: "pennel-launch-\(UUID().uuidString)"))
        let first = h.model(launchDefaults: defaults)
        first.refresh()
        #expect(first.requestLaunch())
        try h.session(.ready)
        let recreated = h.model(launchDefaults: defaults)
        recreated.refresh()
        #expect(recreated.pad == .listening)
        #expect(h.lastCommand?.action == .start)
        #expect(defaults.object(forKey: KeyboardModel.launchRequestKey) == nil)
    }

    @Test func stale_launch_request_never_starts_listening() throws {
        let model = h.model()
        model.refresh()
        #expect(model.requestLaunch())
        h.now = h.now.addingTimeInterval(KeyboardModel.launchStartWindow + 1)
        try h.session(.ready)
        model.refresh()
        #expect(model.pad == .ready)
        #expect(h.lastCommand == nil)
    }

    @Test func launch_waits_for_a_live_insertion_context() throws {
        let model = h.model()
        model.refresh()
        #expect(model.requestLaunch())
        try h.session(.ready)
        h.visible = false
        model.refresh()
        #expect(model.pad == .ready)
        #expect(h.lastCommand == nil)
        h.visible = true
        model.refresh()
        #expect(model.pad == .listening)
    }

    @Test func launch_is_only_requested_without_a_session_and_with_full_access() throws {
        let locked = h.model(fullAccess: false)
        locked.refresh()
        #expect(!locked.requestLaunch())
        try h.session(.ready)
        let ready = h.model()
        ready.refresh()
        #expect(!ready.requestLaunch())
    }

    @Test func cancelling_a_launch_prevents_the_automatic_start() throws {
        let model = h.model()
        model.refresh()
        #expect(model.requestLaunch())
        model.cancelLaunch()
        try h.session(.ready)
        model.refresh()
        #expect(model.pad == .ready)
        #expect(h.lastCommand == nil)
    }

    @Test func launch_copy_exists_in_both_languages() {
        for language in KeyboardInterfaceLanguage.allCases {
            let copy = KeyboardCopy(language: language)
            for key in [KeyboardCopy.Key.startPennel, .startPennelHint, .openPennelFallback] {
                #expect(!copy.text(key).isEmpty)
            }
        }
    }
}
