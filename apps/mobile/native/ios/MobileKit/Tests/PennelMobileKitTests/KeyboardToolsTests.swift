import Foundation
import Testing
@testable import PennelMobileKit

struct KeyboardToolsTests {
    @Test func emoji_search_handles_both_languages_and_accents() {
        #expect(KeyboardEmojiSearch.search("gefeliciteerd").contains { $0.symbol == "🎉" })
        #expect(KeyboardEmojiSearch.search("BELGIE").first?.symbol == "🇧🇪")
        #expect(KeyboardEmojiSearch.search("coffee").first?.symbol == "☕")
        #expect(KeyboardEmojiSearch.search("rood hart").map(\.symbol) == ["❤️"])
        #expect(KeyboardEmojiSearch.search("👨‍👩‍👧‍👦").map(\.symbol) == ["👨‍👩‍👧‍👦"])
        #expect(KeyboardEmojiSearch.search("no matching expression").isEmpty)
    }

    @Test func recents_are_unique_bounded_and_never_store_typed_queries() {
        let name = "KeyboardToolsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        for emoji in KeyboardEmojiSearch.catalogue.prefix(40) {
            KeyboardToolPreferences.recordEmoji(emoji.symbol, in: defaults)
        }
        KeyboardToolPreferences.recordEmoji("private sentence", in: defaults)
        KeyboardToolPreferences.recordEmoji("😀", in: defaults)
        let recent = KeyboardToolPreferences.recentEmoji(in: defaults)
        #expect(recent.count == 24)
        #expect(recent.first == "😀")
        #expect(Set(recent).count == recent.count)
        #expect(!recent.contains("private sentence"))
        #expect(KeyboardEmojiSearch.search("", recent: recent).first?.symbol == "😀")
        #expect(KeyboardEmojiSearch.search("", recent: recent, limit: -1).isEmpty)
    }

    @Test func movement_steps_are_symmetric_bounded_and_ignore_invalid_input() {
        #expect(KeyboardGestureSteps.offset(translation: 11) == 0)
        #expect(KeyboardGestureSteps.offset(translation: 25) == 2)
        #expect(KeyboardGestureSteps.offset(translation: -25) == -2)
        #expect(KeyboardGestureSteps.offset(translation: .infinity) == 0)
        #expect(KeyboardGestureSteps.offset(translation: 20, step: 0) == 0)
        #expect(KeyboardGestureSteps.offset(translation: 1_000_000) == 256)
    }

    @Test func swipe_payload_is_bounded_without_changing_coordinate_space() {
        let point = KeyboardSwipePoint(x: 0.25, y: 0.75)
        let key = KeyboardSwipeKey(key: "a", x: 0.1, y: 0.5)
        let gesture = KeyboardSwipeGesture(points: Array(repeating: point, count: 400), keys: Array(repeating: key, count: 80))
        #expect(gesture.points.count == 256)
        #expect(gesture.keys.count == 64)
        #expect(gesture.points.first == point)
        #expect(gesture.keys.first == key)
    }
}
