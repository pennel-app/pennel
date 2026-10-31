import Foundation
import Testing
import PennelMobileKit
@testable internal import PennelMobile

@MainActor
struct NativeKeyboardSettingsTests {
    private func isolatedDefaults() -> UserDefaults {
        UserDefaults(suiteName: "pennel-keyboard-settings-tests-\(UUID().uuidString)")!
    }

    @Test func parses_native_keys_without_writing_until_apply() throws {
        let defaults = isolatedDefaults()
        var values: [String: Any] = [
            "keyboard_typing_languages": ["en", "nl"], "keyboard_autocorrection": false,
            "keyboard_learning": true, "keyboard_swipe": true,
            "keyboard_snippet_expansion": true, "keyboard_haptic_style": "crisp", "mode": "fast"
        ]
        let patch = try NativeKeyboardSettingsPatch(removingFrom: &values)
        #expect(values.count == 1 && values["mode"] as? String == "fast")
        #expect(!KeyboardPreferences.learningEnabled(in: defaults))
        #expect(!KeyboardPreferences.swipeEnabled(in: defaults))
        patch.apply(in: defaults)
        #expect(KeyboardPreferences.typingLanguages(in: defaults) == ["en", "nl"])
        #expect(!KeyboardPreferences.autocorrectionEnabled(in: defaults))
        #expect(KeyboardPreferences.learningEnabled(in: defaults))
        #expect(KeyboardPreferences.swipeEnabled(in: defaults))
        #expect(KeyboardPreferences.snippetExpansionEnabled(in: defaults))
        #expect(KeyboardPreferences.typingFeedbackStyle(in: defaults) == .crisp)
    }

    @Test func invalid_patch_never_applies_a_valid_prefix() {
        let defaults = isolatedDefaults()
        var values: [String: Any] = ["keyboard_learning": true, "keyboard_swipe": "yes"]
        #expect(throws: (any Error).self) { _ = try NativeKeyboardSettingsPatch(removingFrom: &values) }
        #expect(!KeyboardPreferences.learningEnabled(in: defaults))
    }

    @Test func rejects_numeric_booleans_duplicate_languages_and_read_only_count() {
        let patches: [[String: Any]] = [
            ["keyboard_autocorrection": NSNumber(value: 1)], ["keyboard_typing_languages": []],
            ["keyboard_typing_languages": ["en", "en"]], ["keyboard_typing_languages": ["fr"]],
            ["keyboard_haptic_style": "heavy"], ["keyboard_learned_count": 0]
        ]
        for var values in patches {
            #expect(throws: (any Error).self) { _ = try NativeKeyboardSettingsPatch(removingFrom: &values) }
        }
    }

    @Test func validates_snippet_identity_triggers_and_utf8_size() throws {
        let defaults = isolatedDefaults()
        let snippet = ["id": "one", "title": "Greeting", "keyword": ";hi", "text": "Hallo café 👨‍👩‍👦!\nTot morgen."]
        var values: [String: Any] = ["keyboard_snippets": [snippet]]
        try NativeKeyboardSettingsPatch(removingFrom: &values).apply(in: defaults)
        #expect(KeyboardPreferences.snippets(in: defaults).first?.text == snippet["text"])
        for invalid in [
            [snippet, snippet],
            [snippet, ["id": "two", "title": "Other", "keyword": ";hi", "text": "Other"]],
            [["id": "two", "title": "Other", "keyword": "has space", "text": "Other"]],
            [["id": "two", "title": "Other", "keyword": "", "text": String(repeating: "😀", count: 513)]]
        ] {
            var values: [String: Any] = ["keyboard_snippets": invalid]
            #expect(throws: (any Error).self) { _ = try NativeKeyboardSettingsPatch(removingFrom: &values) }
        }
    }

    @Test func projection_has_count_without_learned_content_and_reset_is_explicit() throws {
        let defaults = isolatedDefaults()
        KeyboardPreferences.setLearningEnabled(true, in: defaults)
        KeyboardPreferences.learn("Pennel", previous: "hello", in: defaults)
        let projection = NativeKeyboardSettingsPatch.projection(in: defaults)
        #expect(projection["keyboard_learned_count"] as? Int == 1)
        #expect(projection["learned_words"] == nil)
        #expect(projection["keyboard_learned_words"] == nil)
        var values: [String: Any] = ["keyboard_learning_reset": false]
        try NativeKeyboardSettingsPatch(removingFrom: &values).apply(in: defaults)
        #expect(KeyboardPreferences.learnedWords(in: defaults).count == 1)
        values = ["keyboard_learning_reset": true]
        try NativeKeyboardSettingsPatch(removingFrom: &values).apply(in: defaults)
        #expect(KeyboardPreferences.learnedWords(in: defaults).isEmpty)
        #expect(KeyboardPreferences.learnedPairs(in: defaults).isEmpty)
    }
}
